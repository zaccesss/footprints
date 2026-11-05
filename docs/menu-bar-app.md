# Menu bar app

[`app/`](../app/README.md) holds a minimal Tauri 2 menu bar app for macOS. It shows today's
recorded changes newest first, with an undo button on each one that can be undone. It reads the
same ledger as the CLI: `FOOTPRINTS_HOME` when set, otherwise the platform default.

## What it does

- Lists today's actions, read from the newest 5,000 in the ledger.
- Undoes one with the same library function as `footprints undo`, so the conflict check is
  identical. When the file changed since it was recorded, the app shows the conflict and leaves
  the file alone.
- Lists each undo as its own entry, marked as the undo of the original change, with a "Take back
  this undo" button. Pressing it puts the agent's change back, so a mistaken click can be
  reversed without a terminal.

There is no force option. `footprints undo <id> --force` in a terminal does that, with the
limits described in [forcing an undo](undo.md#forcing-an-undo).

## Build and run

The app is a separate crate with its own workspace and lock file, so `cargo test` at the root of
the repository never compiles Tauri. It needs Rust 1.89 or later and the Xcode command line
tools.

```sh
cargo build --manifest-path app/src-tauri/Cargo.toml --release
./app/src-tauri/target/release/footprints-app
```

The icon appears in the menu bar. Clicking it opens the window. Its menu has "Show today's
changes" and "Quit Footprints". Closing the window hides it while the app keeps running in the
menu bar.

A `.app` bundle needs the Tauri CLI:

```sh
cargo install tauri-cli --version "^2" --locked
cd app/src-tauri && cargo tauri build
```

## Signing

> [!IMPORTANT]
> The app is unsigned. macOS Gatekeeper warns about an unsigned app the first time it opens.
> Signing and notarising need an Apple Developer account, which the project does not have yet.
> Until then, build the app locally from source. A signed build is
> [on the roadmap](../ROADMAP.md#next).

## Accessibility

The window uses the system's own colours, writes each entry's state in words and gives every
button a 44 by 44 pixel target with a screen reader label. The full list is in
[ACCESSIBILITY.md](../ACCESSIBILITY.md#the-menu-bar-app).

## Limits

- It only lists today's actions.
- It shows no diffs. `footprints diff <id>` does.
- It has only been built for macOS. Tauri supports Linux and Windows too, but neither has been
  tried.
- CI builds, lints and tests it on macOS only. See the
  [workflow overview](../.github/workflows/README.md).
