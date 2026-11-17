mod scan;
mod tui;
mod windows;

use std::{
    env,
    io::{self, IsTerminal, Write},
    process::ExitCode,
};

use scan::{Risk, size};

const HELP: &str = "\
win-cleanup — Preview-first Windows cleanup from WSL

Usage:
  win-cleanup                         Interactive cleanup
  win-cleanup targets                 List cleanup categories
  win-cleanup scan [TARGET ...] [--items]
  win-cleanup clean TARGET ... --dry-run
  win-cleanup clean TARGET ... --yes

Cleanup sends items to the Windows Recycle Bin. Use the TUI to include
individual items from the Old Downloads category.
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("win-cleanup: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<u8> {
    let mut stdout = io::stdout();
    let args: Vec<_> = env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("tui");
    if matches!(command, "-h" | "--help" | "help") {
        write!(stdout, "{HELP}")?;
        return Ok(0);
    }
    if !matches!(command, "tui" | "targets" | "scan" | "clean") {
        eprintln!("{HELP}");
        return Ok(2);
    }
    let mut ids = Vec::new();
    let mut show_items = false;
    let mut dry_run = false;
    let mut yes = false;
    for arg in args.iter().skip(1) {
        match (command, arg.as_str()) {
            ("scan", "--items") => show_items = true,
            ("clean", "--dry-run") => dry_run = true,
            ("clean", "--yes") => yes = true,
            ("scan" | "clean", id) if !id.starts_with('-') => ids.push(id),
            _ => {
                eprintln!("Unexpected argument: {arg}\n\n{HELP}");
                return Ok(2);
            }
        }
    }
    if command == "clean" && (ids.is_empty() || dry_run == yes) {
        eprintln!("Pass target IDs and exactly one of --dry-run or --yes.");
        return Ok(2);
    }

    let device = scan::drive_device()?;
    let profile = windows::profile()?;
    let mut targets = scan::targets(&profile);
    for id in &ids {
        if !targets.iter().any(|target| target.id == *id) {
            eprintln!("Unknown target: {id}");
            return Ok(2);
        }
    }
    targets.retain(|target| ids.is_empty() || ids.contains(&target.id));

    match command {
        "tui" if io::stdin().is_terminal() && io::stdout().is_terminal() => {
            tui::run(targets, device)?;
            return Ok(0);
        }
        "tui" => {
            return Err(io::Error::other(
                "Use an interactive terminal or the scan command",
            ));
        }
        "targets" => {
            for target in targets {
                writeln!(
                    stdout,
                    "{:16} {:9} {}",
                    target.id,
                    target.risk.label(),
                    target.name
                )?;
            }
            return Ok(0);
        }
        "clean" if yes && targets.iter().any(|target| target.risk == Risk::Risky) => {
            eprintln!("Include individual Old Downloads items in the TUI.");
            return Ok(2);
        }
        _ => {}
    }
    let mut plan = Vec::new();
    for target in targets {
        let target = scan::scan_target(target, device, false);
        writeln!(
            stdout,
            "{:16} {:>11} {:>8} files",
            target.id,
            size(target.bytes()),
            target.count()
        )?;
        for warning in &target.warnings {
            eprintln!("{warning}");
        }
        for item in target.items {
            if show_items || dry_run {
                writeln!(
                    stdout,
                    "  {:>11}  {}",
                    size(item.bytes),
                    item.path.display()
                )?;
            }
            if yes {
                plan.push((target.id, item));
            }
        }
    }
    if yes {
        let report = scan::clean(plan, device);
        writeln!(
            stdout,
            "Moved {} items ({}) to the Recycle Bin.",
            report.moved,
            size(report.bytes)
        )?;
        for error in &report.errors {
            eprintln!("{error}");
        }
        if !report.errors.is_empty() {
            return Ok(3);
        }
    }
    Ok(0)
}
