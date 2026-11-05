# Workflows

| Workflow | Runs on | What it checks |
| --- | --- | --- |
| [`ci.yml`](ci.yml) | Every pull request, every push to `main` and on demand | `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test` and `cargo build --release` on Ubuntu and macOS |
| [`ci.yml`](ci.yml), `msrv` job | The same triggers | `cargo check --all-targets` on Rust 1.89, the `rust-version` in `Cargo.toml`, on Ubuntu only to keep it cheap |

Both operating systems run because the capture code differs between them: macOS clones files with
APFS `clonefile` and watches with FSEvents, while Linux copies files and watches with inotify. The
tests use temporary directories only and set `FOOTPRINTS_HOME` to one, so a run never touches a
real ledger.

The job has no secrets and only read access to the repository contents.
