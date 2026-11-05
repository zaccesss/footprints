# Footprints menu bar app

A minimal tray app for macOS, Windows and Linux. It shows today's recorded changes, newest
first, with an undo button on each one that can be undone. It reads the same ledger as the CLI:
`FOOTPRINTS_HOME` when set, otherwise the platform default.

The undo button calls the same library function as `footprints undo`, so the conflict check is
identical. When the file changed since it was recorded, the app shows the conflict and leaves the
file alone. There is no force option. Use `footprints undo <id> --force` in a terminal for that.

Each undo leaves its own record in the list, marked as the undo of the original change, with a
"Take back this undo" button. Pressing it puts the agent's change back, so a mistaken click can
be reversed without a terminal.

## Build

The app is a separate crate with its own workspace and lock file, so `cargo test` at the root of
the repository never compiles Tauri. It needs Rust 1.89 or later plus the Xcode command line tools
on macOS, the Visual Studio C++ build tools on Windows or the WebKitGTK and AppIndicator
development packages on Linux. The [menu bar app guide](../docs/menu-bar-app.md#build-from-source)
lists them.

```sh
cargo build --manifest-path app/src-tauri/Cargo.toml --release
./app/src-tauri/target/release/footprints-app
```

The icon appears in the macOS menu bar, the Windows notification area or a Linux panel. Its menu
has "Show today's changes" and "Quit Footprints". Closing the window hides it; the app keeps
running in the tray.

A bundle for the current platform (`.app`, an NSIS installer or a `.deb` and an AppImage) needs
the Tauri CLI:

```sh
cd app/src-tauri
npx --yes @tauri-apps/cli@2.11.5 build -- --locked
```

## Releases and signing

> [!IMPORTANT]
> From 0.4.0 each release carries the app for macOS, Windows and Linux. Every build is unsigned:
> Gatekeeper on macOS and SmartScreen on Windows warn the first time it opens. The
> [menu bar app guide](../docs/menu-bar-app.md#install-a-release) shows what to expect and how to
> get past it.

## What it does not do yet

- It only lists today's actions, read from the newest 5,000 in the ledger.
- It shows no diffs. `footprints diff <id>` does.
- On Linux it needs a panel that shows AppIndicator icons. GNOME needs the AppIndicator extension
  outside Ubuntu.
