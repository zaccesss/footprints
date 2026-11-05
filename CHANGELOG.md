# Changelog

All notable changes to Footprints are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-11-05

### Added

- The menu bar app on Windows and Linux. It sits in the Windows notification area and in a Linux
  panel through AppIndicator, with a colour tray icon that reads on light and dark taskbars and
  panels. macOS keeps its template icon. Each release attaches the app as a universal macOS zip,
  a Windows NSIS installer and a Linux `.deb` and AppImage, each with a `.sha256` file. They are
  unsigned, so Gatekeeper and SmartScreen warn the first time. CI lints and tests the app on
  macOS, Windows and Linux.
- A screenshot of the menu bar app in the README and in the menu bar app guide.
- A dark and a light banner and a dark social preview image in `assets/`, beside the light one.

### Changed

- The release workflow writes the Homebrew formula straight to the tap's main branch as a Zac Bot
  commit instead of opening and merging a pull request.

## [0.3.0] - 2026-11-05

### Added

- `footprints prune`, which removes old history by `--older-than`, `--before`, `--session`,
  `--agent`, `--all` or a `--max-size` budget that gives up the oldest sessions first. It prints
  what it would remove and changes nothing without `--yes`. The pruned actions and their file
  copies go to the Trash (the Recycle Bin on Windows) as one archive folder unless
  `--permanent` is given. A file copy that a remaining action still uses is never deleted. The
  ledger is vacuumed afterwards. Sessions still being recorded are kept. A command and its
  file changes are pruned together. Prune is not offered over MCP.
- `footprints import`, which restores a pruned archive with every action's original id and
  refuses an archive that was already restored.
- `locks/ledger.lock`, held shared by every process with the ledger open, so `footprints prune
  --yes` refuses while a run, the daemon, the MCP server, the menu bar app or a hook is using
  the ledger.
- `FOOTPRINTS_TRASH_DIR`, a folder that prune archives go to instead of the system Trash.
- `CLICOLOR_FORCE`, which turns colour on through a pipe. `NO_COLOR` and `--no-color` still turn
  it off.
- A screenshot of a real Footprints run in a real terminal in the README and the getting started
  guide, with full alt text. The transcript is also kept as text in `assets/demo/demo.txt`.
- crates.io and Homebrew badges in the README, plus `winget install zaccesss.Footprints` in the
  install table, which works once the first winget submission is accepted.
- winget manifest templates in `packaging/winget/` for the package `zaccesss.Footprints`, with
  `zaccesss` as its publisher and author. `packaging/fill-checksums.sh` fills them as well as
  the Homebrew formula and takes an optional folder of `.sha256` files.
- `packaging/README.md`, the manual fallback for Homebrew and winget.
- Two release workflow jobs. `homebrew` fills the formula from the release's sums and sends it
  to `zaccesss/homebrew-tap` through the GitHub API, as a Zac Bot commit on a `footprints-X.Y.Z`
  branch and a squash-merged pull request. A failed tap update is a warning in the run summary
  and never fails the run. `winget` opens the pull request for the new version in
  `microsoft/winget-pkgs` through winget-releaser. Each skips with a note in the run summary when
  its secret (`TAP_TOKEN` or `WINGET_TOKEN`) is missing. The winget job also skips until the
  package exists in winget.

### Fixed

- The workflows README now gives the right wait for a release (10 minutes) and names `attach`
  as the only job that writes to the repository.

## [0.2.0] - 2026-11-05

### Added

- Windows support. The data directory defaults to `%LOCALAPPDATA%\footprints`. Paths with drive
  letters, forward slashes and the `\\?\` prefix are stored in one normal form. Run locks use
  `LockFileEx` through `File::lock`. A killed wrapper is detected with `OpenProcess`. The
  wrapper survives `Ctrl+C` and `Ctrl+Break` while its child runs. On a console close it records
  in the seconds Windows allows. The file cache uses the NTFS file index and change time.
- `footprints daemon`, an opt-in recorder for changes made by any process under a list of roots.
  It uses FSEvents on macOS, inotify on Linux and ReadDirectoryChangesW on Windows. `--fanotify` on
  Linux names the process behind each write and `--usn` on Windows also reads the NTFS change
  journal. `--snapshot` keeps a pre-image of every file from the start. Changes another tier
  already recorded are joined to it rather than recorded twice. A file moved in from outside a
  root is never deleted on undo. Lost events trigger a walk of every root.
- Hook adapters for Cursor, OpenAI Codex CLI and Gemini CLI through `footprints hook pre --from`
  and `footprints hook post --from`, with `footprints install cursor|codex|gemini --print`.
  Codex `apply_patch` calls record every file named in the patch. Cursor's Write and Delete
  tools are recorded through `preToolUse` and `postToolUse` with a real pre-image. An
  `afterFileEdit` that no `preToolUse` saw gets a pre-image rebuilt from the edit text only
  when it can be proved exact.
- A minimal menu bar app in `app/` built with Tauri 2. It lists today's changes and undoes one
  with the same conflict check as the CLI. Its own undo records are listed too, so an undo made
  from the app can be taken back there. It is unsigned.
- The Footprints mark: a trail of two filled steps and a hollow one, in `assets/` as light, dark,
  automatic and one-colour SVGs plus a lockup with the name. The README header shows it. The
  menu bar app uses it as its template tray icon and its app icon.
- Linux pre-images use a `FICLONE` reflink where the file system supports it.
- `scripts/e2e.sh`, an end-to-end run of a built binary that checks the restored bytes.
- Release binaries for macOS (arm64 and x86_64), Linux (x86_64 and aarch64) and Windows (x86_64)
  with sha256 sums, attached to each GitHub release by a new release workflow. The same workflow
  publishes to crates.io through Trusted Publishing and skips a version that is already there.
- A Homebrew formula template for `zaccesss/homebrew-tap` and `packaging/fill-checksums.sh`.
- ROADMAP.md, with what is done, what is next and what is deliberately later.
- The Claude Code hooks record `PowerShell`, Claude Code's native Windows shell tool, the same way
  as `Bash`. Release archives carry the `docs/` folder so the README's links work offline.

### Changed

- CI runs on Windows as well as Ubuntu and macOS, runs the end-to-end script on all three, builds
  every release target and builds the menu bar app on macOS.
- `..` at the root of an absolute path now stays at the root when a path is normalised.
- CI pins every action to a commit and passes the minimum Rust version as a toolchain input, so Dependabot can bump the actions without changing the Rust version.
- The README is now a short overview. The detail moved into a new `docs/` folder with pages
  for getting started, capture, undo, every command, MCP, export, data and privacy and the menu
  bar app, plus a setup guide for each supported agent and for agents without a native adapter.

## [0.1.0] - 2026-11-05

### Added

- A SQLite ledger of agent actions with a content-addressed blob store. Each file image is stored
  once under its BLAKE3 hash and compressed with zstd. The data directory is `FOOTPRINTS_HOME`,
  defaulting to `~/.local/share/footprints`.
- Five action kinds: `file_write`, `file_create`, `file_delete`, `command` and `network`. Commands
  and network calls are always marked not reversible.
- `footprints log` with session, agent and kind filters plus `--json`.
- `footprints show` and `footprints diff`, a unified diff from pre-image to post-image.
- `footprints undo` for one action and `undo --session` for a whole session in reverse order. An
  undo refuses with exit status 3 when the file changed since it was recorded. On the command
  line `--force` overrides the check only when the current contents can be stored first. Those
  contents are kept in the ledger. The file is checked again just before the restored copy is
  renamed into place.
- Session undo simulates every step first (conflicts, missing pre-images, parent directories and
  a file that replaced a directory) and changes nothing if any step fails. Each undo command
  records its actions in its own `undo-` session. Undoing an undo makes the original change
  undoable again.
- The Claude Code hook adapter, `footprints hook pre` and `footprints hook post`, covering Write,
  Edit, MultiEdit, NotebookEdit, Bash, WebFetch and WebSearch. `PostToolUseFailure` is handled
  too, so a failed call still records its file changes.
- Symlinks are recorded as changes but marked not reversible, so an undo never deletes a file
  that replaced one.
- `footprints install claude-code --print` and `hooks/claude-code/settings.example.json`.
- `footprints run`, a wrapper for any agent. It clones the watched directory with APFS
  `clonefile` on macOS, runs the child there, watches it while the child runs and records every
  created, changed and deleted file. It survives `SIGINT`, `SIGQUIT`, `SIGHUP` and `SIGTERM`
  while the child runs and refuses to watch a directory another run is already watching.
- `footprints recover`, which records a run that was killed outright from its staging area.
- Bash hook snapshots stop at 256 MiB of new content or 10 seconds so the agent is never held
  up. The command's note says when its file changes were not recorded.
- `footprints export --otel`, OTLP JSON spans following the OpenTelemetry GenAI conventions.
- `footprints mcp`, an MCP server over stdio with `footprints_log`, `footprints_diff` and
  `footprints_undo`. The undo tool has no force option, so an agent can never overwrite newer
  work.
- Private storage: a `077` umask, `0700` directories and `0600` blobs and ledger.
- CI on Ubuntu and macOS running format, Clippy, tests and a release build, plus a check on the
  minimum supported Rust version, 1.89.

[Unreleased]: https://github.com/zaccesss/footprints/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/zaccesss/footprints/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/zaccesss/footprints/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/zaccesss/footprints/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/zaccesss/footprints/releases/tag/v0.1.0
