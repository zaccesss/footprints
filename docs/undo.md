# Undo

Every file change Footprints records keeps a copy of the file before and after it, so any one
step can be shown as a diff and put back. An undo refuses when the file has changed since the
agent touched it, so it never overwrites later work without being told to.

## Contents

- [Undo one action](#undo-one-action)
- [Undo a whole session](#undo-a-whole-session)
- [The conflict check](#the-conflict-check)
- [Forcing an undo](#forcing-an-undo)
- [Undoing an undo](#undoing-an-undo)
- [Recovering a killed run](#recovering-a-killed-run)
- [What can and cannot be undone](#what-can-and-cannot-be-undone)
- [Further limits](#further-limits)
- [Windows](#windows)

## Undo one action

Find the action in the log, check what it changed, then undo it:

```sh
footprints log
footprints diff 2
footprints undo 2
```

```console
$ footprints undo 2
undid #2: restored /Users/me/demo/config.toml (recorded as #3 in session undo-c41baf35)
```

What the undo does depends on the action:

| Action | Undo |
| --- | --- |
| `file_write` | Writes the pre-image back, file mode included |
| `file_create` | Deletes the file |
| `file_delete` | Recreates the file with its contents and mode |

The restored copy is written beside the file and renamed into place, so the file is never left
half written. `--json` prints the result as JSON.

## Undo a whole session

```sh
footprints log --session run-bcc287bf
footprints undo --session run-bcc287bf
```

`undo --session` undoes every file action in the session, newest first. Before touching any file
it simulates the whole session:

- every step must match the disk at that point
- every pre-image must be in the blob store
- every parent directory must be creatable

A file that replaced a directory is removed before anything is restored under that path. One
problem in the simulation means nothing changes.

Once the real undo starts it is not a transaction. If the disk changes underneath it or a write
fails, it stops at that step. It names the step and the path, lists what was already undone and
prints the `footprints undo --session` command that reverses those steps.

## The conflict check

An undo only goes ahead when the file still matches the recorded post-image, by BLAKE3 hash.
Otherwise it prints the conflict and exits with status 3:

```console
$ footprints undo 3
footprints: refused: conflict: /Users/me/demo/config.toml changed after #3 was recorded
  recorded post-image: blake3 59cdb0473659 (13 bytes)
  current file:        blake3 9bc08c50d0a6 (16 bytes)
Nothing was changed. Run `footprints diff 3` to see the recorded change. Run `footprints undo 3 --force` to overwrite it with the pre-image. The current contents are stored first, so that undo can itself be undone
```

The file is checked again just before the restored copy is renamed into place. A change in that
moment makes the undo stop and leave the file alone.

Exit statuses are 0 for an undo that went ahead, 1 for an error, 2 for a usage error and 3 for an
undo refused on purpose. Refusals start with `footprints: refused:` on standard error.

## Forcing an undo

`--force` overrides the conflict check on the command line, but only when the current contents
can be stored first. Those contents become the pre-image of the undo, so a forced undo can
itself be undone.

`--force` still refuses when the current file is too large to store, a directory or a symlink,
since overwriting it would destroy it. The [MCP server](mcp.md) and the
[menu bar app](menu-bar-app.md) have no force option at all, so an agent or a stray click can
never overwrite newer work.

## Undoing an undo

Each undo command records its own actions in a new session named `undo-` followed by eight
characters, under agent `footprints`. So one undo can be reversed without touching any other:

```console
$ footprints log
ID  TIME                 AGENT       SESSION        KIND        STATE           TARGET
 1  2026-11-05 12:57:29  demo        run-bcc287bf   command     not reversible  sh agent.sh
 2  2026-11-05 12:57:29  demo        run-bcc287bf   file_write  undone          config.toml (via #1)
 3  2026-11-05 12:57:29  footprints  undo-c41baf35  file_write  reversible      config.toml (undo of #2)
```

Undoing action 3 puts the agent's change back and makes action 2 undoable again.

## Recovering a killed run

If a `footprints run` wrapper is killed before it can record (`SIGKILL`, Task Manager or a
console closed mid-record), its staging area is still on disk.

```sh
footprints recover
```

`recover` records those changes as if the run had finished. The next `footprints run` does the
same automatically before it starts.

## What can and cannot be undone

| Recorded | Undo | Why |
| --- | --- | --- |
| A file write or edit | Restores the pre-image, file mode included | Both images are stored |
| A file the agent created | Deletes it | The pre-image is "did not exist" |
| A file the agent deleted | Recreates it with its contents and mode | The pre-image is stored |
| A shell command | Not as a whole | A command can do anything. The file changes it made inside its working directory are recorded as separate actions under it. Each of those can be undone |
| A network call | Never | The data has already left the machine. It is recorded so it can be seen |
| A file over 32 MiB | Not reversible | Its contents are not stored. The size limit is `FOOTPRINTS_MAX_FILE_BYTES` |
| A symlink created, removed, repointed or replaced by a file | Not reversible | Only regular files are restored, so an undo never deletes what replaced the link |
| A directory or an extended attribute | Not restored | Only regular files and their mode are stored |
| A change to a file's mode alone, such as `chmod +x` | Not recorded | The contents did not change |
| A file whose name is not valid UTF-8 | Not recorded | The ledger stores paths as UTF-8 |
| A changed file under a skipped or unreadable directory | Not reversible | It may be moved existing data rather than a new file |
| The first daemon change to a file without `--snapshot` | Not reversible | There was no pre-image yet |
| A file moved in while the daemon watched | Not reversible | It may be the only copy of existing data |
| A Bash call whose snapshot ran out of budget | Not recorded | The snapshot was abandoned so the agent was not held up |
| A pre hook whose tool was then denied | Not reversible | It stays `pending`, since the tool never ran |
| Changes outside the watched directory or roots | Not recorded by the wrapper or the daemon | Neither tier watches there. A hook still records a file tool's own target |
| An MCP tool call | Not recorded | No adapter records MCP tools in this version |

## Further limits

- A Bash hook compares the working directory before and after the command. A file another tool
  writes in that directory at the same moment is attributed to the command.
- The first Bash hook in a directory reads every file there. A snapshot over 256 MiB of new
  content or 10 seconds is abandoned and its note says so. Each later call gets further as the
  cache fills. [Snapshots and budgets](how-capture-works.md#snapshots-and-budgets) has the
  settings.
- The wrapper refuses to start when the watched directory holds more than 10,000 files, since a
  partial snapshot could not be trusted. Raise `FOOTPRINTS_MAX_FILES` or pick a narrower `--dir`.
- Files that appear while a skipped or unreadable directory changes (for example
  `mv target build` or `chmod 755 private`) may be moved existing data rather than new files.
  They are recorded as not reversible, so an undo never deletes them.
- A Bash call with `run_in_background` is only partly captured. Its post hook fires once the
  command has started, so only changes made by then are recorded. The command's note says so.
- There is no pruning command yet. Every pre-image stays in the blob store until the data
  directory is deleted. Pruning is [next on the roadmap](../ROADMAP.md#next).

## Windows

- An undo fails with "access denied" while another program holds the file open without delete
  sharing, as some editors do. Nothing is changed and the undo can be run again once the file is
  closed.
- File attributes such as read-only are not restored. Restoring them is
  [on the roadmap](../ROADMAP.md#next).
- Paths are compared without regard to case, folding ASCII letters only. The full rules are in
  [how capture works](how-capture-works.md#paths).
