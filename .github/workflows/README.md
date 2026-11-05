# Workflows

| Workflow | Runs on | What it does |
| --- | --- | --- |
| [`ci.yml`](ci.yml), `check` job | Every pull request, every push to `main` and on demand | `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test` and `cargo build --release` on Ubuntu, macOS and Windows, then one end-to-end run of the release binary with [`scripts/e2e.sh`](../../scripts/e2e.sh) |
| [`ci.yml`](ci.yml), `msrv` job | The same triggers | `cargo check --all-targets` on Rust 1.89, the `rust-version` in `Cargo.toml`, on Ubuntu only to keep it cheap |
| [`ci.yml`](ci.yml), `build` job | The same triggers | A release build for each published target: macOS arm64 and x86_64, Linux x86_64 and aarch64 (musl) and Windows x86_64. Each binary is kept as a workflow artefact for seven days |
| [`ci.yml`](ci.yml), `app` job | The same triggers | Clippy, tests and a release build of the menu bar app in [`app/`](../../app/README.md), on macOS only |
| [`release.yml`](release.yml), `version`, `build`, `attach` and `publish` jobs | A pushed `v*` tag | Checks the tag against `Cargo.toml`, builds the same five targets, packages each with the docs, attaches the archives and their sha256 sums to the existing GitHub release for the tag with `gh release upload`, then publishes the crate to crates.io |
| [`release.yml`](release.yml), `homebrew` job | The same tag, once the files are attached | Fills [`packaging/homebrew/footprints.rb`](../../packaging/homebrew/footprints.rb) from the release's `.sha256` files with [`packaging/fill-checksums.sh`](../../packaging/fill-checksums.sh) then writes it to `Formula/footprints.rb` on the main branch of `zaccesss/homebrew-tap` as a Zac Bot commit. Skipped with a note in the run summary when the `TAP_TOKEN` secret is missing. A failed update is a warning that never fails the run |
| [`release.yml`](release.yml), `winget` job | The same tag, once the files are attached | Opens the pull request for the new version of `zaccesss.Footprints` in `microsoft/winget-pkgs` from the `zaccesss/winget-pkgs` fork with `vedantmgoyal9/winget-releaser`. Skipped with a note in the run summary when the `WINGET_TOKEN` secret is missing or the package is not in winget yet |

All three operating systems run because the capture code differs between them. macOS clones files
with APFS `clonefile` and watches with FSEvents. Linux reflinks or copies files and watches with
inotify. Windows copies files and watches with ReadDirectoryChangesW. The tests use temporary
directories only and set `FOOTPRINTS_HOME` to one, so a run never touches a real ledger. On
Windows the `check` job puts Git for Windows' `usr\bin` on the path, since the tests drive the
wrapper with small `sh` scripts.

`ci.yml` has no secrets and only read access to the repository contents. Every action in both
workflows is pinned to a commit.

The release workflow never creates a GitHub release. The release is made by a maintainer from a
signed tag. The workflow waits up to 10 minutes for it before attaching files. Its `attach`
job is the only one with write access to contents, used only to attach those files. The
`homebrew` and `winget` jobs write to other repositories with their own tokens and only read
this one.

Its `publish` job uses crates.io Trusted Publishing. It runs in the `crates-io` environment with
`id-token: write`. `rust-lang/crates-io-auth-action` swaps GitHub's OIDC token for a
short-lived crates.io token, so no crates.io token is stored as a secret. It runs only after every
platform's files are attached. When the version in `Cargo.toml` is already on crates.io the job
skips publishing and says so in the run summary.

## Homebrew and winget

Both jobs run after `attach` and neither blocks the crates.io publish. Each checks for its secret
first. A missing secret skips the job with a note in the run summary, so the release itself still
succeeds. A tap update that fails is reported as a warning in the run summary without failing the
run. A `winget` secret that is present but wrong fails that job, so the problem shows on the run.

| Secret | Kind | Scope | Used by |
| --- | --- | --- | --- |
| `TAP_TOKEN` | Fine-grained personal access token from the account that owns the tap | Only `zaccesss/homebrew-tap`, with Contents read and write | `homebrew` |
| `WINGET_TOKEN` | Classic personal access token | `public_repo` | `winget` |

The `homebrew` job copies `packaging/` to a temporary folder and moves the copy to the tag's
version first, so the formula template does not have to be bumped by hand before a release.
`zaccesss/homebrew-tap` belongs to the maintainer, so a pull request opened with their token would
count on their profile. The job writes to `main` instead, which the tap's ruleset allows for the
admin role. Through the GitHub API alone it:

1. Stops early when `Formula/footprints.rb` in the tap already matches the filled formula.
2. Writes `Formula/footprints.rb` on `main` as a commit by `Zac Bot <automation@isaacadjei.me>`.

The `winget` job uses
[winget-releaser](https://github.com/vedantmgoyal9/winget-releaser), pinned to a commit. It runs
[Komac](https://github.com/russellbanks/Komac), which reads the newest accepted manifests of
`zaccesss.Footprints`, swaps in the new zip and its sum, then opens the pull request from the
`zaccesss/winget-pkgs` fork. Komac can only update a package that already exists. The first
version was submitted by hand, so until that pull request is accepted the job finds no package
and skips. The winget maintainers review every pull request, which usually takes a day or two.

> [!IMPORTANT]
> crates.io can only add a trusted publisher to a crate that already exists. The very first
> publish of `footprints` is therefore done once by hand with `cargo publish`. After that the
> trusted publisher is added on the crate's settings page on crates.io: repository
> `zaccesss/footprints`, workflow `release.yml` and environment `crates-io`. The `crates-io`
> environment must exist in the repository's settings too.
