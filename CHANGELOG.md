# Changelog

All notable changes to Footprints are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/zaccesss/footprints/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/zaccesss/footprints/releases/tag/v0.1.0
