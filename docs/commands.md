# Commands

Every subcommand and flag of `footprints` 0.2.0, taken from its own `--help` output.
`footprints help <command>` or `footprints <command> --help` prints the same text in a terminal.

## Contents

- [Global options](#global-options)
- [Exit statuses](#exit-statuses)
- [log](#log)
- [show](#show)
- [diff](#diff)
- [undo](#undo)
- [recover](#recover)
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
value, when `TERM=dumb` and whenever standard output is not a terminal. The colours used and
what carries the meaning without them are in [ACCESSIBILITY.md](../ACCESSIBILITY.md).

## Exit statuses

| Status | Meaning |
| --- | --- |
| 0 | Success |
| 1 | An error. The message starts with `footprints: error:` |
| 2 | A usage error, such as an unknown flag |
| 3 | An undo refused on purpose, such as a conflict. The message starts with `footprints: refused:` |

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
| `NO_COLOR` | unset | Any value turns colour off |
| `FOOTPRINTS_SESSION` | set by `footprints run` | The session name, given to the wrapped command |
