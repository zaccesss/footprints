# Contributing

Bug reports, fixes and new agent adapters are welcome. Footprints is young, so a short issue before
a large pull request saves wasted work on both sides.

## What belongs here

- Bug fixes, above all anything where an undo restores the wrong bytes or loses a change
- Hook adapters for other agents that have pre and post tool hooks
- Capture improvements for macOS and Linux that keep the "refuse on conflict" rule
- Clearer output, error messages and docs

## What does not belong here

- Anything that sends ledger data off the machine by default
- An undo that skips the conflict check without `--force`
- Changes that make a reversible action look reversible when it is not

## Building and testing

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

CI runs the same four checks on Ubuntu and macOS. The tests create temporary directories and set
`FOOTPRINTS_HOME` to one, so they never touch a real ledger. New tests should do the same. A change
to capture or undo needs a test that checks the restored bytes, not only the exit status.

## Layout

| Path | What it holds |
| --- | --- |
| `src/store.rs` | The SQLite ledger and the action model |
| `src/blob.rs` | The content-addressed blob store |
| `src/snapshot.rs` | Directory walks, file images and manifests |
| `src/hook.rs` | The Claude Code hook adapter |
| `src/run.rs` | The `footprints run` wrapper |
| `src/undo.rs` | Undo, conflict checks and session undo |
| `src/diff.rs`, `src/output.rs` | Diffs and readable output |
| `src/export.rs` | OpenTelemetry export |
| `src/mcp.rs` | The MCP server |
| `tests/` | Integration tests that drive the real binary |

## How to contribute

1. Open an issue with the bug report or feature request form.
2. Create a branch named `fix/<short-description>` or `feat/<short-description>`.
3. Make the change with tests and run the four checks above.
4. Add a line under `Unreleased` in [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.
5. Open a pull request with the template and describe what changed and why.

Code comments explain why something is done, not what the line does. Output must stay readable
without colour: see [ACCESSIBILITY.md](ACCESSIBILITY.md).

## Using AI

Every pull request declares whether AI was used, following the
[AI policy](https://github.com/zaccesss/ai-policy) (also on
[isaacadjei.me/ai-policy](https://isaacadjei.me/ai-policy)). The pull request template asks for it.

## The shared guide

> [!NOTE]
> One shared contributing guide covers every project here:
> [zaccesss/contribute](https://github.com/zaccesss/contribute), also on
> [isaacadjei.me/contribute](https://isaacadjei.me/contribute). This file takes precedence where
> the two differ.
