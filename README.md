# win-cleanup

Clean up Windows caches from a terminal in WSL. I liked how [mac-cleanup-go](https://github.com/2ykwang/mac-cleanup-go) shows what it found before you delete anything, and wanted the same workflow on my Windows machine.

The tool scans C: for things like old temp files, browser caches, GPU shader caches, and package manager caches. Pick what to remove, review the paths and sizes, then send the selection to the Windows Recycle Bin.

## Run it

From WSL, with Rust 1.88 or newer, C: mounted at `/mnt/c`, and access to Windows PowerShell:

```sh
git clone https://github.com/TobinPalmer/win-cleanup.git
cd win-cleanup
cargo run --locked --release
```

To install it as a command, run `cargo install --path . --locked` from the repo, then run `win-cleanup` with Cargo's bin directory on your PATH.

Use **Space** to select categories, **Enter** to review, and **?** for all keys. The first scan gives each category about three seconds and flags partial results. Press **w** for scan warnings or **f** to run a full scan. Old Downloads are opt-in per file.

## From the command line

```sh
win-cleanup targets
win-cleanup scan chrome edge --items
win-cleanup clean directx --dry-run
```

Use `scan` with no category names to scan everything. If Windows offers to permanently delete an item it can't recycle, choose **Cancel** to keep it.

[MIT license](LICENSE)
