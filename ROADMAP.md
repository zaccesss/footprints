# Roadmap

What Footprints does today, what comes next and what is deliberately left for later. The
[CHANGELOG](CHANGELOG.md) records what each release actually shipped.

## Done

| Area | What works |
| --- | --- |
| Ledger | SQLite actions plus a BLAKE3 and zstd blob store. Undo with a conflict check, session undo that simulates every step first and undo of an undo |
| Hook adapters | Claude Code, Cursor, OpenAI Codex CLI and Gemini CLI, each from its own documented hook payload. See the [coverage table](docs/agents/README.md#hook-coverage-per-native-adapter) |
| Wrapper | `footprints run` for any agent, with APFS `clonefile` pre-images on macOS, `FICLONE` reflinks on Btrfs and XFS and a plain copy everywhere else |
| Daemon | `footprints daemon`, an opt-in recorder for changes made by any process under a list of roots. FSEvents on macOS, inotify on Linux with optional fanotify, ReadDirectoryChangesW on Windows with an optional USN journal reader |
| Platforms | macOS, Linux and Windows, all three tested in CI with a real end-to-end run |
| Interfaces | The CLI, an MCP server with no force option and OpenTelemetry GenAI export |
| Menu bar app | A minimal Tauri app in [`app/`](app/README.md) with today's changes and an undo button. Built in CI on macOS, unsigned |
| Release | A release workflow that builds binaries for macOS (arm64 and x86_64), Linux (x86_64 and aarch64) and Windows (x86_64) with sha256 sums, attaches them to the release and publishes to crates.io. A Homebrew formula for `zaccesss/homebrew-tap` is filled from those sums. None of this has run for a real release until v0.2.0 ships |

## Next

- Pruning and a size budget for the blob store. Nothing is ever deleted today, so the data
  directory only grows.
- A signed and notarised menu bar app. Signing needs an Apple Developer account, which the
  project does not have yet.
- `footprints install <agent>` that writes the hook block into an agent's settings after showing
  the change and asking. Today every install command only prints.
- A winget manifest beside the Homebrew formula.
- Restoring the Windows read-only attribute on undo. Unix modes are restored, but Windows
  attributes are not recorded yet.

## Deliberately later

- **ReFS block cloning on Windows.** It needs cluster-aligned extents and a pre-sized sparse
  target, which is a lot of machinery for a pre-image. Windows copies files instead.
- **fanotify with file handles (`FAN_REPORT_FID`).** It would report creates and deletes with
  the process behind them, but resolving the handles needs `CAP_DAC_READ_SEARCH` as well. The
  daemon reads close-after-write events only. inotify supplies the rest.
- **Reading the USN journal from before the daemon started.** The journal keeps changes made
  while nothing was watching, but none of them would have a pre-image. They would add noise
  without anything to undo.
- **A background service.** The daemon runs in the foreground until stopped. Installing it as a
  launchd agent, a systemd unit or a Windows service is left to each user until the daemon has
  had more real use.
- **Policies, dry runs and a team server.** These are ideas from the design, not commitments.
  They wait until the single-machine ledger format is stable.
- **Client libraries for Python and Node.** They would freeze the ledger's wire format, so they
  come after it settles.
