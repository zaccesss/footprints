# Commands

Every subcommand and flag of `footprints` 0.3.0, taken from its own `--help` output.
`footprints help <command>` or `footprints <command> --help` prints the same text in a terminal.

## Contents

- [Global options](#global-options)
- [Exit statuses](#exit-statuses)
- [log](#log)
- [show](#show)
- [diff](#diff)
- [undo](#undo)
- [recover](#recover)
- [prune](#prune)
- [import](#import)
- [export](#export)
- [run](#run)
- [daemon](#daemon)
- [hook](#hook)
- [install](#install)
- [mcp](#mcp)
- [Environment variables](#environment-variables)

## Global options

```console
$ footprints --help
See and undo what AI agents do on your machine.

Footprints keeps a local ledger of file writes, shell commands and network calls made by AI agents, with a stored copy of every file before and after each change. Any one file change can be undone. An undo refuses if the file changed since.

Data lives in $FOOTPRINTS_HOME when it is set, otherwise in ~/.local/share/footprints on macOS and Linux and in %LOCALAPPDATA%\footprints on Windows.

Usage: footprints [OPTIONS] <COMMAND>

Commands:
  log      show the timeline of recorded actions, oldest first
  show     show every recorded field of one action
  diff     show a unified diff from an action's pre-image to its post-image
  undo     restore a file's pre-image or delete a file the agent created
  export   export the ledger in another format
  hook     agent hook adapter: reads a hook payload as JSON on standard input
  install  print the configuration that connects an agent to footprints
  run      run any command and record every file it creates, changes or deletes
  mcp      serve the ledger to agents as an MCP server over stdio
  daemon   record changes made by any process under the given roots, until stopped
  recover  record the changes of a `footprints run` that was killed before it could
  prune    remove old actions and the stored file copies only they use
  import   restore the actions in an archive made by `footprints prune`
  help     Print this message or the help of the given subcommand(s)

Options:
      --no-color
          never print colour, even on a terminal (NO_COLOR is also respected)

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

`--no-color` is accepted by every subcommand. Colour is also off when `NO_COLOR` is set to any
value, when `TERM=dumb` and whenever standard output is not a terminal. `CLICOLOR_FORCE` set to
any value other than `0` turns colour on even through a pipe, unless `NO_COLOR` or `--no-color`
turns it off. The colours used and
what carries the meaning without them are in [ACCESSIBILITY.md](../ACCESSIBILITY.md).

## Exit statuses

| Status | Meaning |
| --- | --- |
| 0 | Success |
| 1 | An error. The message starts with `footprints: error:` |
| 2 | A usage error, such as an unknown flag |
| 3 | An undo, prune or import refused on purpose, such as an undo conflict or a prune while the ledger is in use. The message starts with `footprints: refused:` |

Two commands differ. `footprints run` exits with the child's own status once the child has
started, so a 1 or a 3 there may come from the child. `footprints hook` always exits 0, so a
problem in Footprints never blocks an agent's tool call.

## log

```console
$ footprints log --help
show the timeline of recorded actions, oldest first

Usage: footprints log [OPTIONS]

Options:
      --no-color           never print colour, even on a terminal (NO_COLOR is also respected)
      --session <SESSION>  
      --agent <AGENT>      
      --kind <KIND>        file_write, file_create, file_delete, command or network
  -n, --limit <LIMIT>      show the most recent N actions [default: 50]
      --all                show every action, ignoring --limit
      --json               print JSON instead of a table
  -h, --help               Print help
```

`--session` and `--agent` keep only actions with that exact session or agent name. The table
shows the most recent actions, oldest at the top:

```console
$ footprints log
ID  TIME                 AGENT       SESSION        KIND        STATE           TARGET
 1  2026-11-05 12:57:29  demo        run-bcc287bf   command     not reversible  sh agent.sh
 2  2026-11-05 12:57:29  demo        run-bcc287bf   file_write  undone          config.toml (via #1)
 3  2026-11-05 12:57:29  footprints  undo-c41baf35  file_write  reversible      config.toml (undo of #2)
```

`STATE` is `reversible`, `not reversible`, `pending` or `undone`. `(via #1)` marks a file change
made under command 1 and `(undo of #2)` marks an undo. Paths are shown relative to the current
directory where possible. Times are local. Commands longer than 72 characters are clipped with
`...` in the table. `footprints show` has the full text.

## show

```console
$ footprints show --help
show every recorded field of one action

Usage: footprints show [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --json      
      --no-color  never print colour, even on a terminal (NO_COLOR is also respected)
  -h, --help      Print help
```

`<ID>` is the number in the log's `ID` column. For a command it also lists the file changes
recorded under it.

```console
$ footprints show 2
id:         2
time:       2026-11-05 12:57:29
finished:   2026-11-05 12:57:29
agent:      demo
session:    run-bcc287bf
kind:       file_write
state:      reversible
path:       /Users/me/demo/config.toml
tool:       run
pre-image:  blake3 59cdb04736591303789a2f38b61f66c2ca9e52084ebbcb4898eb14c611511793 (13 bytes)
post-image: blake3 0bdd503edfbe2cd4b1d131892cc5eb0754d5bf22d8ee18f20935425f1c514011 (13 bytes)
directory:  /Users/me/demo
part of:    #1
```

`--json` prints every field as JSON.

## diff

```console
$ footprints diff --help
show a unified diff from an action's pre-image to its post-image

Usage: footprints diff [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --no-color  never print colour, even on a terminal (NO_COLOR is also respected)
  -h, --help      Print help
```

```console
$ footprints diff 2
--- a/Users/me/demo/config.toml
+++ b/Users/me/demo/config.toml
@@ -1 +1 @@
-name = "old"
+name = "new"
```

A created file is diffed against `/dev/null`:

```console
$ footprints diff 5
--- /dev/null
+++ b/Users/me/demo/made.txt
@@ -0,0 +1 @@
+hi
```

Every line starts with `+`, `-`, a space or `@@`, so the diff reads the same without colour.

## undo

```console
$ footprints undo --help
restore a file's pre-image or delete a file the agent created

Usage: footprints undo [OPTIONS] [ID]

Arguments:
  [ID]  

Options:
      --no-color           never print colour, even on a terminal (NO_COLOR is also respected)
      --session <SESSION>  undo every file action in a session, newest first
      --force              undo even when the file changed since it was recorded
      --json               
  -h, --help               Print help
```

Give either an `[ID]` or `--session`. `--json` prints the result as JSON. Every check, the
session simulation and the limits of `--force` are in [undo](undo.md).

## recover

```console
$ footprints recover --help
record the changes of a `footprints run` that was killed before it could

Usage: footprints recover [OPTIONS]

Options:
      --no-color  never print colour, even on a terminal (NO_COLOR is also respected)
  -h, --help      Print help
```

Every `footprints run` also does this when it starts. See
[recovering a killed run](undo.md#recovering-a-killed-run).

## prune

```console
$ footprints prune --help
remove old actions and the stored file copies only they use

Without --yes nothing is changed: the actions, sessions and bytes that would go are printed. With --yes the pruned actions and every file copy they use are written as one archive folder, footprints-pruned-<date>-<n>, which is moved to the Trash (the Recycle Bin on Windows). `footprints import` puts an archive back. Selectors combine: an action is pruned only when it matches every one given. A command and the file changes under it are pruned together. Sessions still being recorded are never pruned. Prune refuses while any other footprints process has the ledger open. Prune is not offered over MCP, so an agent can never erase its own trail.

Usage: footprints prune <SELECTOR>... [--yes] [--permanent] [--json]

Options:
      --no-color
          never print colour, even on a terminal (NO_COLOR is also respected)

      --older-than <DURATION>
          actions that finished longer ago than this, such as 30d, 12h or 2w

      --before <DATE>
          actions that finished before this local date (YYYY-MM-DD) or RFC 3339 time

      --session <SESSION>
          actions in this session

      --agent <AGENT>
          actions recorded for this agent

      --all
          every action

      --max-size <SIZE>
          prune the oldest sessions first until the data folder fits in this size, such as 500MB or 2GiB

      --yes
          prune for real; without it nothing is changed

      --permanent
          delete outright instead of moving an archive to the Trash

      --json
          print JSON instead of text

  -h, --help
          Print help (see a summary with '-h')
```

At least one selector is required; without one the command exits 2. Selectors combine with AND:

| Selector | Picks |
| --- | --- |
| `--older-than <DURATION>` | Actions that finished longer ago than this. Units are `s`, `m`, `h`, `d` and `w`, as in `90m`, `12h`, `30d` or `2w` |
| `--before <DATE>` | Actions that finished before local midnight of `YYYY-MM-DD` or before an RFC 3339 time such as `2026-10-01T09:00:00Z` |
| `--session <SESSION>` | Actions in that exact session |
| `--agent <AGENT>` | Actions recorded for that exact agent name |
| `--all` | Every action |
| `--max-size <SIZE>` | The oldest matching sessions, one whole session at a time, until the data directory fits. `KB`, `MB`, `GB` and `TB` are powers of 1000. `KiB`, `MiB`, `GiB` and `TiB` are powers of 1024. A plain number is bytes |

Without `--yes` it prints the plan and changes nothing:

```console
$ footprints prune --agent demo
Would prune 5 action(s) in 2 session(s).

SESSION       AGENT  ACTIONS  FIRST                LAST
run-1f2e3d4c  demo         3  2026-11-05 16:12:38  2026-11-05 16:12:38
run-8a9b0c1d  demo         2  2026-11-05 16:12:38  2026-11-05 16:12:39

Frees 549 B of stored file copies (5 blob(s)) and about 2.4 KB of ledger.
Kept 1 blob(s) that remaining actions still use.
Data folder: 66.4 KB now, about 63.5 KB after.
Nothing was changed. Run again with --yes to prune. The pruned actions then go to the Trash as one archive folder, unless --permanent is given.
```

The ledger figure is an estimate; the file copies are counted exactly. With `--yes` the same
summary starts with `Pruned` and ends with where the archive went. `--json` prints the plan and
the result as JSON. The archive format, what is always kept and how to restore are in
[removing data](data-and-privacy.md#removing-data).

## import

```console
$ footprints import --help
restore the actions in an archive made by `footprints prune`

Every action keeps its original id. The archive is read and left as it is. Import refuses when an id in the archive is already in the ledger, which means it was restored before.

Usage: footprints import [OPTIONS] <ARCHIVE>

Arguments:
  <ARCHIVE>
          the archive folder, taken out of the Trash first

Options:
      --json
          print JSON instead of text

      --no-color
          never print colour, even on a terminal (NO_COLOR is also respected)

  -h, --help
          Print help (see a summary with '-h')
```

```console
$ footprints import ~/Desktop/footprints-pruned-2026-11-05-1
restored 5 action(s) in 2 session(s) and 5 stored file copy(ies) from /Users/me/Desktop/footprints-pruned-2026-11-05-1
```

## export

```console
$ footprints export --help
export the ledger in another format

Usage: footprints export [OPTIONS]

Options:
      --no-color           never print colour, even on a terminal (NO_COLOR is also respected)
      --otel               OpenTelemetry GenAI-style spans as OTLP JSON
      --session <SESSION>  
      --agent <AGENT>      
  -o, --output <OUTPUT>    write to this file instead of standard output
  -h, --help               Print help
```

`--otel` is the only format in this version. The span layout and attributes are in
[OpenTelemetry export](export.md).

## run

```console
$ footprints run --help
run any command and record every file it creates, changes or deletes

Usage: footprints run [OPTIONS] <COMMAND>...

Arguments:
  <COMMAND>...  the command to run, after `--`

Options:
      --agent <AGENT>      agent name for the ledger (defaults to the program name)
      --no-color           never print colour, even on a terminal (NO_COLOR is also respected)
      --session <SESSION>  session name (defaults to a new one per run)
      --dir <DIR>          directory to watch and run the command in (defaults to the current directory)
      --intent <INTENT>    what the run is meant to do, stored with every action
  -h, --help               Print help
```

```sh
footprints run --agent goose -- goose session
footprints run --agent ollama-loop --dir ~/projects/site -- python agent.py
footprints run --agent aider --intent "fix the failing test" -- aider tests/test_api.py
```

Everything after `--` is the command, so its own flags are never read as Footprints flags. The
child sees `FOOTPRINTS_SESSION` in its environment. How the wrapper takes its snapshot, survives
signals and locks its directory is in [the wrapper](how-capture-works.md#the-wrapper).

## daemon

```console
$ footprints daemon --help
record changes made by any process under the given roots, until stopped

Usage: footprints daemon [OPTIONS] --root <ROOTS>

Options:
      --no-color           never print colour, even on a terminal (NO_COLOR is also respected)
      --root <ROOTS>       a directory to watch; repeat for several
      --snapshot           copy every file under the roots first, so the first change to each is reversible
      --seconds <SECONDS>  stop after this many seconds instead of waiting for Ctrl+C
      --fanotify           Linux only: also read fanotify to name the writing process (needs CAP_SYS_ADMIN)
      --usn                Windows only: also read the volume's USN change journal (needs administrator rights)
  -h, --help               Print help
```

```sh
footprints daemon --root ~/projects --root ~/.config --snapshot
sudo footprints daemon --root ~/projects --fanotify
```

On Windows, `--usn` needs a terminal opened with **Run as administrator**. Everything the
daemon records and what it can undo is in [the daemon](how-capture-works.md#the-daemon).

## hook

```console
$ footprints hook --help
footprints: hook error: agent hook adapter: reads a hook payload as JSON on standard input

Usage: footprints hook [OPTIONS] <COMMAND>

Commands:
  pre   PreToolUse: store the pre-image
  post  PostToolUse: store the post-image
  help  Print this message or the help of the given subcommand(s)

Options:
      --no-color       never print colour, even on a terminal (NO_COLOR is also respected)
      --from <FROM>    which agent sent the payload [default: claude-code] [possible values: claude-code, cursor, codex, gemini]
      --agent <AGENT>  the agent name stored with each action (defaults to the --from agent)
  -h, --help           Print help
```

`footprints hook pre` and `footprints hook post` take the same two options. They are meant to be
run by an agent, never by hand: each reads one hook payload as JSON on standard input. The
default agent names are `claude-code`, `cursor`, `codex` and `gemini-cli`.

The hook commands always exit 0, help included. Their help text therefore goes to standard
error after a `footprints: hook error:` prefix. That is the same path any bad hook argument
takes, so a typo in an agent's settings never blocks the agent.

What each agent's hook prints on standard output:

| `--from` | Before hook | After hook |
| --- | --- | --- |
| `claude-code` | Nothing | Nothing |
| `codex` | Nothing | Nothing |
| `cursor` | `{"permission":"allow"}` | Nothing |
| `gemini` | `{}` | `{}` |

## install

```console
$ footprints install --help
print the configuration that connects an agent to footprints

Usage: footprints install [OPTIONS] <COMMAND>

Commands:
  claude-code  Claude Code hooks for settings.json
  cursor       Cursor hooks for hooks.json
  codex        OpenAI Codex CLI hooks for hooks.json
  gemini       Gemini CLI hooks for settings.json
  help         Print this message or the help of the given subcommand(s)

Options:
      --no-color  never print colour, even on a terminal (NO_COLOR is also respected)
  -h, --help      Print help
```

Each target takes `--print`, described as "print the snippet (the only mode in this version;
nothing is written)". Without `--print` the command exits 1 and names the file to merge the
snippet into. Footprints never edits agent settings itself. A version that writes the block
after showing the change and asking is [on the roadmap](../ROADMAP.md#next).

| Command | Prints | Merge into | Same as |
| --- | --- | --- | --- |
| `footprints install claude-code --print` | Claude Code hooks | `~/.claude/settings.json` or `.claude/settings.json` | [`hooks/claude-code/settings.example.json`](../hooks/claude-code/settings.example.json) |
| `footprints install cursor --print` | Cursor hooks | `~/.cursor/hooks.json` or `.cursor/hooks.json` | [`hooks/cursor/hooks.example.json`](../hooks/cursor/hooks.example.json) |
| `footprints install codex --print` | Codex CLI hooks | `~/.codex/hooks.json` or `.codex/hooks.json` | [`hooks/codex/hooks.example.json`](../hooks/codex/hooks.example.json) |
| `footprints install gemini --print` | Gemini CLI hooks | `~/.gemini/settings.json` or `.gemini/settings.json` | [`hooks/gemini-cli/settings.example.json`](../hooks/gemini-cli/settings.example.json) |

The full output of each is on [its agent page](agents/README.md).

## mcp

```console
$ footprints mcp --help
serve the ledger to agents as an MCP server over stdio

Usage: footprints mcp [OPTIONS]

Options:
      --no-color  never print colour, even on a terminal (NO_COLOR is also respected)
  -h, --help      Print help
```

The tools it offers and how to register it are in [MCP server](mcp.md).

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `FOOTPRINTS_HOME` | `~/.local/share/footprints` on macOS and Linux, `%LOCALAPPDATA%\footprints` on Windows | The data directory |
| `FOOTPRINTS_MAX_FILE_BYTES` | `33554432` (32 MiB) | Files above this are recorded but not stored |
| `FOOTPRINTS_MAX_FILES` | `10000` | The most files one snapshot walks |
| `FOOTPRINTS_SNAPSHOT_BUDGET_BYTES` | `268435456` (256 MiB) | The most new content one Bash hook snapshot reads |
| `FOOTPRINTS_SNAPSHOT_SECONDS` | `10` | The longest one Bash hook snapshot runs |
| `FOOTPRINTS_TRASH_DIR` | unset | A folder that `footprints prune` moves its archives into instead of the system Trash |
| `NO_COLOR` | unset | Any value turns colour off |
| `CLICOLOR_FORCE` | unset | Any value other than `0` turns colour on through a pipe. `NO_COLOR` and `--no-color` still win |
| `FOOTPRINTS_SESSION` | set by `footprints run` | The session name, given to the wrapped command |
