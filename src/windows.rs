use std::{
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, de::DeserializeOwned};

pub const DRIVE: &str = "/mnt/c";

// Keep the command static. Send file names as UTF-8 JSON over stdin.
fn powershell(script: &str, input: Option<&[u8]>) -> io::Result<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; [Console]::InputEncoding=[Text.UTF8Encoding]::new($false); [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); try {{ {script} }} catch {{ [Console]::Error.WriteLine($_.Exception.GetBaseException().Message); exit 1 }}"
    );
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut child = Command::new("/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .current_dir(DRIVE)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let write_result = input.map(|data| child.stdin.take().unwrap().write_all(data));
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    if let Some(result) = write_result {
        result?;
    }
    Ok(output.stdout)
}

fn json<T: DeserializeOwned>(script: &str, input: Option<&[u8]>) -> io::Result<T> {
    let output = powershell(script, input)?;
    serde_json::from_slice(&output).map_err(io::Error::other)
}

#[derive(Deserialize)]
struct WindowsPaths {
    local: String,
    roaming: String,
    profile: String,
}

pub struct Profile {
    pub local: Option<PathBuf>,
    pub roaming: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

pub fn profile() -> io::Result<Profile> {
    let paths: WindowsPaths = json(
        "@{local=$env:LOCALAPPDATA; roaming=$env:APPDATA; profile=$env:USERPROFILE} | ConvertTo-Json -Compress",
        None,
    )
    .map_err(|err| io::Error::other(format!("Could not read Windows profile via WSL interop: {err}")))?;
    Ok(Profile {
        local: mounted_path(&paths.local),
        roaming: mounted_path(&paths.roaming),
        home: mounted_path(&paths.profile),
    })
}

fn mounted_path(path: &str) -> Option<PathBuf> {
    let prefix = path.get(..3)?;
    if !prefix.eq_ignore_ascii_case("C:\\") {
        return None;
    }
    let result = Path::new(DRIVE).join(path.get(3..)?.replace('\\', "/"));
    windows_path(&result).ok()?;
    Some(result)
}

pub fn windows_path(path: &Path) -> io::Result<String> {
    let invalid =
        || io::Error::other("Path is outside /mnt/c or cannot be addressed safely on Windows");
    let relative = path.strip_prefix(DRIVE).map_err(|_| invalid())?;
    let mut result = String::from("C:");
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        let name = name.to_str().ok_or_else(invalid)?;
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if reserved
            || name.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        {
            return Err(invalid());
        }
        result.push('\\');
        result.push_str(name);
    }
    if result.len() == 2 {
        return Err(invalid());
    }
    Ok(result)
}

// DrvFS directory enumeration can be slow for large Windows temp folders.
pub fn old_temp_entries(root: &Path) -> io::Result<Vec<PathBuf>> {
    let input = serde_json::to_vec(&windows_path(root)?).map_err(io::Error::other)?;
    let names: Vec<String> = json(
        r#"
        $path = ConvertFrom-Json ([Console]::In.ReadToEnd());
        $cutoff = [DateTime]::UtcNow.AddDays(-7);
        $directory = [IO.DirectoryInfo]::new($path);
        $names = @(
            foreach ($entry in $directory.EnumerateFileSystemInfos()) {
                if ($entry.LastWriteTimeUtc -le $cutoff) { $entry.Name; }
            }
        );
        ConvertTo-Json -Compress -InputObject $names;
        "#,
        Some(&input),
    )?;
    Ok(names.into_iter().map(|name| root.join(name)).collect())
}

pub fn open(path: &Path) -> io::Result<()> {
    Command::new("/mnt/c/Windows/explorer.exe")
        .arg(windows_path(path)?)
        .current_dir(DRIVE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
}

// FOF_WANTNUKEWARNING preserves the Windows warning if recycling is unavailable.
const RECYCLE_SHELL: &str = r#"
using System;
using System.IO;
using System.Runtime.InteropServices;

public static class CleanupShell {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct Operation {
        public IntPtr Window;
        public uint Function;
        public IntPtr From;
        public IntPtr To;
        public ushort Flags;
        [MarshalAs(UnmanagedType.Bool)] public bool Aborted;
        public IntPtr Mappings;
        public IntPtr Title;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern int SHFileOperation(ref Operation operation);

    public static void Recycle(string path) {
        for (string candidate = path; candidate.Length > 3; candidate = Path.GetDirectoryName(candidate)) {
            CheckAttributes(candidate);
        }
        if ((File.GetAttributes(path) & FileAttributes.Directory) != 0) {
            var pending = new System.Collections.Generic.Stack<string>();
            pending.Push(path);
            while (pending.Count > 0) {
                foreach (string child in Directory.EnumerateFileSystemEntries(pending.Pop())) {
                    if ((CheckAttributes(child) & FileAttributes.Directory) != 0) {
                        pending.Push(child);
                    }
                }
            }
        }

        const ushort silent = 0x0004;
        const ushort noConfirmation = 0x0010;
        const ushort allowUndo = 0x0040;
        const ushort noConnectedElements = 0x2000;
        const ushort wantNukeWarning = 0x4000;
        // The shell requires a UTF-16 path with two terminating nulls.
        IntPtr from = Marshal.StringToHGlobalUni(path + "\0");
        try {
            var op = new Operation {
                Function = 3, // FO_DELETE
                From = from,
                Flags = silent | noConfirmation | allowUndo | noConnectedElements | wantNukeWarning
            };
            int code = SHFileOperation(ref op);
            if (code != 0) throw new IOException("Windows shell error (0x" + code.ToString("X") + ")");
            if (op.Aborted) throw new OperationCanceledException("Windows canceled the operation");
        } finally {
            Marshal.FreeHGlobal(from);
        }
    }

    private static FileAttributes CheckAttributes(string path) {
        var attributes = File.GetAttributes(path);
        if ((attributes & FileAttributes.ReparsePoint) != 0) {
            throw new IOException("Windows reparse point: " + path);
        }
        return attributes;
    }
}
"#;

pub fn recycle(paths: &[String]) -> io::Result<Vec<Option<String>>> {
    let input = serde_json::to_vec(paths).map_err(io::Error::other)?;
    let script = r#"
        $paths = ConvertFrom-Json ([Console]::In.ReadToEnd());
        $results = @(
            foreach ($path in $paths) {
                try {
                    [CleanupShell]::Recycle($path);
                    $null;
                } catch {
                    $_.Exception.GetBaseException().Message;
                }
            }
        );
        ConvertTo-Json -Compress -InputObject $results;
    "#;
    let source = RECYCLE_SHELL.replace('\'', "''");
    let command = format!("Add-Type -TypeDefinition '{source}'; {script}");
    let results: Vec<Option<String>> = json(&command, Some(&input))?;
    if results.len() != paths.len() {
        return Err(io::Error::other(
            "Unexpected response from Windows Recycle Bin",
        ));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_keep_unicode_spaces_and_punctuation() {
        let path = Path::new("/mnt/c/Users/Élodie/Cache/O'Brien & Co.txt");
        assert_eq!(
            windows_path(path).unwrap(),
            r"C:\Users\Élodie\Cache\O'Brien & Co.txt"
        );
        assert_eq!(
            mounted_path(r"c:\Users\Élodie").unwrap(),
            Path::new("/mnt/c/Users/Élodie")
        );
    }

    #[test]
    fn windows_paths_reject_unsafe_names_and_other_drives() {
        for path in [
            "/mnt/c",
            "/mnt/d/Cache",
            "/mnt/c/../d/Cache",
            "/mnt/c/cache.",
            "/mnt/c/cache ",
            "/mnt/c/cache:stream",
            "/mnt/c/nul.txt",
            "/mnt/c/COM²",
        ] {
            assert!(windows_path(Path::new(path)).is_err(), "accepted {path}");
        }
        assert!(mounted_path(r"D:\Cache").is_none());
    }
}
