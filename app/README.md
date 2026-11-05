# Footprints menu bar app

A minimal menu bar app for macOS. It shows today's recorded changes, newest first, with an undo
button on each one that can be undone. It reads the same ledger as the CLI: `FOOTPRINTS_HOME` when
set, otherwise the platform default.

The undo button calls the same library function as `footprints undo`, so the conflict check is
identical. When the file changed since it was recorded, the app shows the conflict and leaves the
file alone. There is no force option. Use `footprints undo <id> --force` in a terminal for that.

Each undo leaves its own record in the list, marked as the undo of the original change, with a
"Take back this undo" button. Pressing it puts the agent's change back, so a mistaken click can
be reversed without a terminal.

## Build

The app is a separate crate with its own workspace and lock file, so `cargo test` at the root of
the repository never compiles Tauri. It needs Rust 1.89 or later and the Xcode command line tools.

```sh
cargo build --manifest-path app/src-tauri/Cargo.toml --release
./app/src-tauri/target/release/footprints-app
```

The icon appears in the menu bar. Click it to open the window. Its menu has "Show today's
changes" and "Quit Footprints". Closing the window hides it; the app keeps running in the menu
bar.

A `.app` bundle needs the Tauri CLI:

```sh
cargo install tauri-cli --version "^2" --locked
cd app/src-tauri && cargo tauri build
```

## Signing

> [!IMPORTANT]
> The app is unsigned. macOS Gatekeeper warns about an unsigned app the first time it opens.
> Signing and notarising need an Apple Developer account, which this project does not have yet.
> Until then, build the app locally from source.

## What it does not do yet

- It only lists today's actions, read from the newest 5,000 in the ledger.
- It shows no diffs. `footprints diff <id>` does.
- It has only been built for macOS. Tauri supports Linux and Windows too, but neither has been
  tried.
