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
| `locks/` | One lock per running wrapper, so two runs never watch the same directory, plus `ledger.lock`, which every process holds while it has the ledger open and `footprints prune --yes` takes for itself |

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
| `FOOTPRINTS_TRASH_DIR` | unset | A folder that `footprints prune` moves its archives into instead of the system Trash |
| `NO_COLOR` | unset | Any value turns colour off |
| `CLICOLOR_FORCE` | unset | Any value other than `0` turns colour on even when output is not a terminal. `NO_COLOR` and `--no-color` still win |

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

`footprints prune` removes old history. It picks actions by age (`--older-than 30d`), by date
(`--before 2026-10-01`), by `--session`, by `--agent` or with `--all`. `--max-size 500MB` removes
the oldest sessions first until the data directory fits in that size. Selectors combine, so
`--older-than 30d --agent cursor` removes only Cursor's actions from more than 30 days ago.
Every flag is in [the command reference](commands.md#prune).

A prune changes nothing unless `--yes` is given. Without it the command prints the sessions,
the number of actions and the bytes it would free:

```sh
footprints prune --older-than 30d          # see what would go
footprints prune --older-than 30d --yes    # prune it
```

What a prune keeps and how it removes the rest:

- **A recoverable archive by default.** The pruned actions and every stored file copy they use
  are written as one folder, `footprints-pruned-<date>-<n>`, which goes to the Trash on macOS
  and Linux and to the Recycle Bin on Windows. Nothing is gone for good until the Trash is
  emptied. `--permanent` skips the archive and deletes outright.
- **Stored copies still in use stay.** A blob is deleted only when no remaining action refers to
  it. SQLite's `VACUUM` then gives the ledger's free space back to the disk.
- **Live work is never pruned.** A session is kept whatever the selectors say when it holds a
  pending action from the last 24 hours or a killed run that `footprints recover` can still
  record. A
  command and the file changes recorded under it are always pruned together.
- **No other process may be using the ledger.** With `--yes`, prune refuses while a
  `footprints run`, the daemon, the MCP server, the menu bar app or a hook has the ledger open.
  The refusal exits with status 3. Hooks that start during a prune wait for it to finish.
- **Agents cannot prune.** Prune is not offered through the [MCP server](mcp.md), so an agent can
  never erase its own trail.

When the archive cannot be moved to the Trash, the prune refuses and changes nothing. On Linux
without a desktop the Trash is `~/.local/share/Trash`, a folder nobody may look at. Setting
`FOOTPRINTS_TRASH_DIR` to a folder makes every archive go there instead, which keeps a prune
recoverable on any machine.

### Restoring a pruned archive

Drag the archive folder out of the Trash to any folder, then import it:

```sh
footprints import ~/Desktop/footprints-pruned-2026-11-05-1
```

Every action comes back with its original id. The stored copies come back with it. The
archive is only read, so it can be kept afterwards. An import refuses, changing nothing, when
the archive's actions are already in the ledger.

An archive is laid out like a data directory, so it can also be read without restoring it:

```sh
FOOTPRINTS_HOME=~/Desktop/footprints-pruned-2026-11-05-1 footprints log --all
```

> [!WARNING]
> An archive holds copies of the files the pruned actions touched, secrets included. Keep it as
> private as the data directory. Empty the Trash once it is no longer needed.

### Removing everything

To remove the whole history, quit every agent that has Footprints hooks and stop any
`footprints run` or daemon, then move the data directory to the Trash rather than deleting it
outright. In Finder, choose **Go > Go to Folder** (`Cmd+Shift+G`), enter `~/.local/share` and
move `footprints` to the Trash. From a terminal, `trash` on macOS (built into recent versions,
otherwise [`brew install trash`](https://formulae.brew.sh/formula/trash)) or `gio trash` on a
Linux desktop does the same:

```sh
trash ~/.local/share/footprints        # macOS
gio trash ~/.local/share/footprints    # Linux
```

On Windows, delete `%LOCALAPPDATA%\footprints` in File Explorer, which sends it to the Recycle
Bin. Once the Trash is emptied nothing recorded so far can be undone.

Vulnerabilities in how Footprints stores data are reported as [SECURITY.md](../SECURITY.md)
describes.
