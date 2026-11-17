use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use walkdir::WalkDir;

use crate::windows::{self, DRIVE};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Moderate,
    Risky,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "Safe",
            Self::Moderate => "Moderate",
            Self::Risky => "Risky",
        }
    }
}

pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub risk: Risk,
    pub items: Vec<Item>,
    pub roots: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl Target {
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|item| item.bytes).sum()
    }

    pub fn count(&self) -> u64 {
        self.items.iter().map(|item| item.files).sum()
    }
}

#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub directory: bool,
    pub modified: Option<SystemTime>,
    fingerprint: u64,
}

fn join(base: &Option<PathBuf>, rel: &str) -> Vec<PathBuf> {
    base.iter().map(|path| path.join(rel)).collect()
}

fn cache_dirs(base: &Option<PathBuf>, rel: &str, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    join(base, rel)
        .iter()
        .flat_map(|path| fs::read_dir(path).into_iter().flatten().flatten())
        .filter(|entry| entry.file_name().to_str().is_some_and(&matches))
        .filter(|entry| entry.file_type().is_ok_and(|ty| ty.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

fn profile_caches(base: &Option<PathBuf>, rel: &str, caches: &[&str]) -> Vec<PathBuf> {
    cache_dirs(base, rel, |_| true)
        .into_iter()
        .flat_map(|root| caches.iter().map(move |cache| root.join(cache)))
        .collect()
}

fn named_caches(base: &Option<PathBuf>, rel: &str, names: &[&str]) -> Vec<PathBuf> {
    join(base, rel)
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .collect()
}

fn has_version(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|version| {
        !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn editor_caches(base: &Option<PathBuf>, editor: &str) -> Vec<PathBuf> {
    ["Cache", "Code Cache", "GPUCache", "CachedData", "logs"]
        .iter()
        .flat_map(|cache| join(base, &format!("{editor}/{cache}")))
        .collect()
}

pub fn targets(profile: &windows::Profile) -> Vec<Target> {
    use Risk::{Moderate, Risky, Safe};
    let local = &profile.local;
    let roaming = &profile.roaming;
    let browser = |rel| profile_caches(local, rel, &["Cache", "Code Cache", "GPUCache"]);
    let target = |id, name, risk, roots| Target {
        id,
        name,
        risk,
        roots,
        items: Vec::new(),
        warnings: Vec::new(),
    };
    vec![
        target(
            "directx",
            "DirectX Shader Cache",
            Safe,
            join(local, "D3DSCache"),
        ),
        target(
            "thumbnails",
            "Thumbnail Cache",
            Safe,
            join(local, "Microsoft/Windows/Explorer"),
        ),
        target(
            "crash-dumps",
            "User Crash Dumps",
            Moderate,
            join(local, "CrashDumps"),
        ),
        target(
            "chrome",
            "Chrome Cache",
            Safe,
            browser("Google/Chrome/User Data"),
        ),
        target(
            "edge",
            "Edge Cache",
            Safe,
            browser("Microsoft/Edge/User Data"),
        ),
        target(
            "brave",
            "Brave Cache",
            Safe,
            browser("BraveSoftware/Brave-Browser/User Data"),
        ),
        target(
            "firefox",
            "Firefox Cache",
            Safe,
            profile_caches(local, "Mozilla/Firefox/Profiles", &["cache2"]),
        ),
        target(
            "vscode",
            "VS Code Cache and Logs",
            Safe,
            editor_caches(roaming, "Code"),
        ),
        target(
            "cursor",
            "Cursor Cache and Logs",
            Safe,
            editor_caches(roaming, "Cursor"),
        ),
        target(
            "discord",
            "Discord Cache and Logs",
            Safe,
            editor_caches(roaming, "discord"),
        ),
        target(
            "npm",
            "npm Download Cache",
            Safe,
            join(local, "npm-cache/_cacache"),
        ),
        target("pip", "pip Download Cache", Safe, join(local, "pip/Cache")),
        target("go-build", "Go Build Cache", Safe, join(local, "go-build")),
        target(
            "slack",
            "Slack Cache and Logs",
            Safe,
            editor_caches(roaming, "Slack"),
        ),
        target(
            "spotify",
            "Spotify Web Cache",
            Safe,
            named_caches(
                local,
                "Spotify/Browser",
                &["Cache", "Code Cache", "GPUCache"],
            ),
        ),
        target(
            "nvidia",
            "NVIDIA Shader Cache",
            Safe,
            named_caches(local, "NVIDIA", &["DXCache", "GLCache", "ComputeCache"]),
        ),
        target(
            "amd",
            "AMD Shader Cache",
            Safe,
            named_caches(local, "AMD", &["DxCache", "DxcCache", "GLCache", "VkCache"]),
        ),
        target(
            "epic-cache",
            "Epic Games Launcher Web Cache",
            Safe,
            cache_dirs(local, "EpicGamesLauncher/Saved", |name| {
                name == "webcache" || has_version(name, "webcache_")
            }),
        ),
        target(
            "epic-logs",
            "Epic Games Launcher Logs",
            Moderate,
            join(local, "EpicGamesLauncher/Saved/Logs"),
        ),
        target(
            "pnpm-cache",
            "pnpm Download and Metadata Cache",
            Safe,
            join(local, "pnpm-cache"),
        ),
        target(
            "uv-cache",
            "uv Download Cache",
            Safe,
            cache_dirs(local, "uv/cache", |name| {
                ["sdists-v", "simple-v", "wheels-v"]
                    .iter()
                    .any(|prefix| has_version(name, prefix))
            }),
        ),
        target(
            "yarn-cache",
            "Yarn Classic Download Cache",
            Safe,
            join(local, "Yarn/Cache"),
        ),
        target(
            "windows-web",
            "Windows Internet Cache",
            Safe,
            join(local, "Microsoft/Windows/INetCache"),
        ),
        target(
            "wer",
            "User Windows Error Reports",
            Moderate,
            named_caches(
                local,
                "Microsoft/Windows/WER",
                &["ReportArchive", "ReportQueue"],
            ),
        ),
        target(
            "squirrel-temp",
            "Squirrel Installer Temp",
            Moderate,
            join(local, "SquirrelTemp"),
        ),
        target(
            "edge-webview",
            "Edge WebView2 Cache",
            Safe,
            profile_caches(
                local,
                "Microsoft/EdgeWebView/User Data",
                &["Cache", "Code Cache", "GPUCache"],
            ),
        ),
        target(
            "downloads",
            "Old Downloads (90+ Days)",
            Risky,
            join(&profile.home, "Downloads"),
        ),
        target(
            "user-temp",
            "User Temporary Files",
            Moderate,
            join(local, "Temp"),
        ),
        target(
            "windows-temp",
            "Windows Temporary Files",
            Moderate,
            vec![Path::new(DRIVE).join("Windows/Temp")],
        ),
    ]
}

fn checked_metadata(path: &Path, device: u64) -> io::Result<fs::Metadata> {
    windows::windows_path(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.dev() != device || !(metadata.is_dir() || metadata.is_file()) {
        return Err(io::Error::other("Mounted volume, link, or special file"));
    }
    Ok(metadata)
}

fn validate_path(path: &Path, device: u64) -> io::Result<()> {
    for part in path
        .ancestors()
        .take_while(|part| *part != Path::new(DRIVE))
    {
        checked_metadata(part, device)?;
    }
    Ok(())
}

fn check_deadline(deadline: Option<Instant>) -> io::Result<()> {
    if deadline.is_some_and(|end| Instant::now() >= end) {
        return Err(io::ErrorKind::TimedOut.into());
    }
    Ok(())
}

fn snapshot(
    path: &Path,
    device: u64,
    min_age: Duration,
    deadline: Option<Instant>,
) -> io::Result<Option<Item>> {
    check_deadline(deadline)?;
    let root = checked_metadata(path, device)?;
    let mut fingerprint = DefaultHasher::new();
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    let mut newest = None;
    let now = SystemTime::now();
    let mut record = |path: &Path, metadata: &fs::Metadata| {
        let modified = metadata.modified().ok();
        let age = modified
            .and_then(|time| now.duration_since(time).ok())
            .unwrap_or_default();
        if age < min_age {
            return false;
        }
        newest = newest.max(modified);
        if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
            files += 1;
        }
        path.hash(&mut fingerprint);
        metadata.ino().hash(&mut fingerprint);
        metadata.is_dir().hash(&mut fingerprint);
        metadata.len().hash(&mut fingerprint);
        modified.hash(&mut fingerprint);
        metadata.ctime().hash(&mut fingerprint);
        metadata.ctime_nsec().hash(&mut fingerprint);
        true
    };
    if !record(path, &root) {
        return Ok(None);
    }
    if root.is_dir() {
        for entry in WalkDir::new(path)
            .min_depth(1)
            .follow_links(false)
            .follow_root_links(false)
            .same_file_system(true)
            .sort_by_file_name()
        {
            check_deadline(deadline)?;
            let entry = entry.map_err(io::Error::other)?;
            let metadata = checked_metadata(entry.path(), device)?;
            if !record(entry.path(), &metadata) {
                return Ok(None);
            }
        }
    }

    Ok(Some(Item {
        path: path.to_owned(),
        bytes,
        files,
        directory: root.is_dir(),
        modified: newest,
        fingerprint: fingerprint.finish(),
    }))
}

fn age_limit(id: &str) -> Duration {
    Duration::from_secs(
        86400
            * match id {
                "downloads" => 90,
                "user-temp" | "windows-temp" | "squirrel-temp" => 7,
                _ => 0,
            },
    )
}

fn accepts(id: &str, entry: &fs::DirEntry) -> bool {
    match id {
        "downloads" => {
            entry.file_type().is_ok_and(|ty| ty.is_file())
                && !entry.file_name().eq_ignore_ascii_case("desktop.ini")
        }
        "thumbnails" => {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("thumbcache_")
                && name.ends_with(".db")
                && entry.file_type().is_ok_and(|ty| ty.is_file())
        }
        _ => true,
    }
}

fn scan_root(
    root: &Path,
    device: u64,
    target: &mut Target,
    deadline: Option<Instant>,
) -> io::Result<()> {
    check_deadline(deadline)?;
    validate_path(root, device)?;
    if matches!(target.id, "user-temp" | "windows-temp") {
        for path in windows::old_temp_entries(root)? {
            scan_item(path, device, target, deadline)?;
        }
    } else {
        for entry in fs::read_dir(root)? {
            check_deadline(deadline)?;
            let entry = entry?;
            if accepts(target.id, &entry) {
                scan_item(entry.path(), device, target, deadline)?;
            }
        }
    }
    Ok(())
}

fn scan_item(
    path: PathBuf,
    device: u64,
    target: &mut Target,
    deadline: Option<Instant>,
) -> io::Result<()> {
    match snapshot(&path, device, age_limit(target.id), deadline) {
        Ok(Some(item)) => target.items.push(item),
        Ok(None) => {}
        Err(err) if err.kind() == io::ErrorKind::TimedOut => return Err(err),
        Err(err) => target.warnings.push(format!("{} — {err}", path.display())),
    }
    Ok(())
}

pub fn scan_target(mut target: Target, device: u64, quick: bool) -> Target {
    let deadline = quick.then(|| Instant::now() + Duration::from_secs(3));
    for root in target.roots.clone() {
        match scan_root(&root, device, &mut target, deadline) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::TimedOut && quick => {
                target.warnings.push(format!(
                    "{}: Quick scan is incomplete. Press f for a full scan.",
                    target.name
                ));
                break;
            }
            Err(err) if err.kind() != io::ErrorKind::NotFound => {
                target.warnings.push(format!("{} — {err}", root.display()));
            }
            Err(_) => {}
        }
    }
    target
        .items
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    target
}

pub fn children(path: &Path, device: u64) -> io::Result<Vec<Item>> {
    validate_path(path, device)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(path)? {
        // Show immediate children without scanning the whole subtree again.
        let path = entry?.path();
        if let Ok(metadata) = checked_metadata(&path, device) {
            items.push(Item {
                path,
                bytes: if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                files: u64::from(metadata.is_file()),
                directory: metadata.is_dir(),
                modified: metadata.modified().ok(),
                fingerprint: 0,
            });
        }
    }
    Ok(items)
}

pub fn drive_device() -> io::Result<u64> {
    let metadata = fs::metadata(DRIVE)
        .map_err(|err| io::Error::other(format!("Could not access {DRIVE}: {err}")))?;
    if metadata.is_dir() && metadata.dev() != fs::metadata("/mnt")?.dev() {
        Ok(metadata.dev())
    } else {
        Err(io::Error::other("C: is not mounted at /mnt/c"))
    }
}

#[derive(Default)]
pub struct Report {
    pub moved: usize,
    pub bytes: u64,
    pub errors: Vec<String>,
}

pub fn clean(plan: Vec<(&'static str, Item)>, device: u64) -> Report {
    let mut report = Report::default();
    // Bounded batches reduce interop overhead while keeping validation close to recycling.
    for batch in plan.chunks(64) {
        let mut paths = Vec::new();
        let mut items = Vec::new();
        for (id, item) in batch {
            let checked = validate_path(&item.path, device)
                .and_then(|()| snapshot(&item.path, device, age_limit(id), None))
                .and_then(|current| {
                    if current.is_some_and(|current| current.fingerprint == item.fingerprint) {
                        windows::windows_path(&item.path)
                    } else {
                        Err(io::Error::other(
                            "Contents changed since scan; rescan required",
                        ))
                    }
                });
            match checked {
                Ok(path) => {
                    paths.push(path);
                    items.push(item);
                }
                Err(err) => report
                    .errors
                    .push(format!("{} — {err}", item.path.display())),
            }
        }
        if paths.is_empty() {
            continue;
        }
        match windows::recycle(&paths) {
            Ok(errors) => {
                for (item, error) in items.into_iter().zip(errors) {
                    if let Some(err) = error {
                        report
                            .errors
                            .push(format!("{} — {err}", item.path.display()));
                    } else {
                        report.moved += 1;
                        report.bytes += item.bytes;
                    }
                }
            }
            Err(err) => {
                report.errors.push(format!(
                    "Recycle Bin status unavailable for the last batch: {err}"
                ));
                break;
            }
        }
    }
    report
}

pub fn size(bytes: u64) -> String {
    let mut value = bytes as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if value < 1024.0 || unit == "TB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    unreachable!()
}
