# Data and privacy

Everything Footprints records stays on the machine. Nothing is sent anywhere: there is no
telemetry, no account and no network code outside what an agent itself does. This page lists
what is stored, where and who can read it.

## Contents

- [The data directory](#the-data-directory)
- [What is stored](#what-is-stored)
- [Permissions](#permissions)
- [Settings](#settings)
- [Secrets in the blob store](#secrets-in-the-blob-store)
- [Removing data](#removing-data)

## The data directory

| Platform | Default |
| --- | --- |
| macOS and Linux | `~/.local/share/footprints` |
| Windows | `%LOCALAPPDATA%\footprints` |

`FOOTPRINTS_HOME` replaces the default. Every command, every hook, the daemon and the
[menu bar app](menu-bar-app.md) read it, so an agent's hooks need the same value as the terminal
that reads the log. A wrapper or daemon root inside the data directory is refused.

## What is stored

| Path | Contents |
| --- | --- |
| `ledger.db` | A SQLite database in WAL mode, with the `-wal` and `-shm` files beside it while open. One row per action: time, agent, session, kind, target path, command text or URL, tool, call id, the agent's stated intent, notes, the hashes, sizes and mode of the pre-image and post-image, whether it can be undone and links to its parent command and any undo. A `file_cache` table maps a path, size and times to the hash of its contents, so later snapshots skip unchanged files |
| `blobs/` | Every file image once under its BLAKE3 hash, compressed with zstd. Identical contents across actions take the space of one copy |
| `staging/` | The wrapper's copies of a watched directory while a run is in progress. Recorded runs are cleaned up. An orphan is removed after an hour |
| `locks/` | One lock per running wrapper, so two runs never watch the same directory |

Shell commands are stored as their full command text, which can include anything typed on the
command line. Web calls are stored as their URL or search query. For Gemini CLI's `web_fetch`
that is the whole prompt, which holds the URLs.

## Permissions

On macOS and Linux Footprints creates its files with a `077` umask. The data directory is set to
`0700` when its ledger is first created. `blobs/` and `staging/` are always `0700`. The ledger
and every blob are `0600`, so copies of a private file are never readable by other accounts.

On Windows the default data directory sits under `%LOCALAPPDATA%`, inside the user profile, whose
access list already limits it to that account, SYSTEM and administrators. Footprints does not
change access lists itself. A `FOOTPRINTS_HOME` outside the profile inherits that folder's access
list, so it should be a private folder.

## Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `FOOTPRINTS_HOME` | `~/.local/share/footprints` on macOS and Linux, `%LOCALAPPDATA%\footprints` on Windows | The data directory |
| `FOOTPRINTS_MAX_FILE_BYTES` | `33554432` (32 MiB) | Files above this are recorded but their contents are not stored |
| `FOOTPRINTS_MAX_FILES` | `10000` | The most files one snapshot walks |
| `FOOTPRINTS_SNAPSHOT_BUDGET_BYTES` | `268435456` (256 MiB) | The most new content one Bash hook snapshot reads |
| `FOOTPRINTS_SNAPSHOT_SECONDS` | `10` | The longest one Bash hook snapshot runs |
| `NO_COLOR` | unset | Any value turns colour off |

How the budgets shape what is recorded is in
[snapshots and budgets](how-capture-works.md#snapshots-and-budgets).

## Secrets in the blob store

> [!WARNING]
> The blob store keeps copies of every file an agent touched, so a `.env` file or a key that an
> agent edited is in there too. A shell hook snapshot also copies every file in the command's
> working directory. Keep the data directory private to the user account and out of backups that
> other people can read.

`.git`, `target` and `node_modules` are never copied. Lowering `FOOTPRINTS_MAX_FILE_BYTES`
keeps large files out. Running the wrapper with a narrow `--dir` keeps unrelated folders out.

## Removing data

There is no pruning command yet, so the data directory only grows. Pruning and a size budget are
[next on the roadmap](../ROADMAP.md#next). Until then, deleting the data directory removes the
whole history, including every stored copy of every file. After that nothing recorded so far can
be undone.

Vulnerabilities in how Footprints stores data are reported as [SECURITY.md](../SECURITY.md)
describes.
