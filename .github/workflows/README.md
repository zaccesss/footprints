# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [`ci.yml`](ci.yml), `check` job | Every pull request, every push to `main` and on demand | `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test` and `cargo build --release` on Ubuntu, macOS and Windows, then one end-to-end run of the release binary with [`scripts/e2e.sh`](../../scripts/e2e.sh) |
| [`ci.yml`](ci.yml), `msrv` job | The same triggers | `cargo check --all-targets` on Rust 1.89, the `rust-version` in `Cargo.toml`, on Ubuntu only to keep it cheap |
| [`ci.yml`](ci.yml), `build` job | The same triggers | A release build for each published target: macOS arm64 and x86_64, Linux x86_64 and aarch64 (musl) and Windows x86_64. Each binary is kept as a workflow artefact for seven days |
| [`ci.yml`](ci.yml), `app` job | The same triggers | Clippy, tests and a release build of the menu bar app in [`app/`](../../app/README.md), on macOS only |
| [`release.yml`](release.yml) | A pushed `v*` tag | Builds the same five targets, packages each with the docs, attaches the archives and their sha256 sums to the existing GitHub release for the tag with `gh release upload`, then publishes the crate to crates.io |

All three operating systems run because the capture code differs between them. macOS clones files
with APFS `clonefile` and watches with FSEvents. Linux reflinks or copies files and watches with
inotify. Windows copies files and watches with ReadDirectoryChangesW. The tests use temporary
directories only and set `FOOTPRINTS_HOME` to one, so a run never touches a real ledger. On
Windows the `check` job puts Git for Windows' `usr\bin` on the path, since the tests drive the
wrapper with small `sh` scripts.

`ci.yml` has no secrets and only read access to the repository contents. Every action is pinned
to a commit.

The release workflow never creates a GitHub release. The release is made by a maintainer from a
signed tag. The workflow waits up to 20 minutes for it before attaching files. Its `build`
job has write access to contents only to attach those files.

Its `publish` job uses crates.io Trusted Publishing. It runs in the `crates-io` environment with
`id-token: write`. `rust-lang/crates-io-auth-action` swaps GitHub's OIDC token for a
short-lived crates.io token, so no crates.io token is stored as a secret. It runs only after every
platform's files are attached. When the version in `Cargo.toml` is already on crates.io the job
skips publishing and says so in the run summary.

> [!IMPORTANT]
> crates.io can only add a trusted publisher to a crate that already exists. The very first
> publish of `footprints` is therefore done once by hand with `cargo publish`. After that the
> trusted publisher is added on the crate's settings page on crates.io: repository
> `zaccesss/footprints`, workflow `release.yml` and environment `crates-io`. The `crates-io`
> environment must exist in the repository's settings too.
