# Any other agent

An agent without a native adapter can still be recorded. Two routes see file changes whatever
made them. The MCP server also works with any agent that can start a stdio MCP server.

| Route | Works for | Sees | Misses |
| --- | --- | --- | --- |
| [The wrapper](#the-wrapper) | Any agent started from a terminal | Every file the agent or its subprocesses created, changed or deleted under the watched directory, with a pre-image | Which tool made each change and why. Shell commands as separate entries. Web calls. Anything outside the directory |
| [The daemon](#the-daemon) | Anything, including editor extensions and desktop apps | Every file any process changed under its roots | The agent's name (stored as `unknown`). Tools and intent. The first change to each file unless `--snapshot` is used |
| [MCP](#mcp) | Agents that can start a stdio MCP server | Nothing by itself: it lets the agent read the ledger and undo | Recording. Pair it with the wrapper or the daemon |

## The wrapper

Put `footprints run --agent <name> --` in front of the command that starts the agent:

```sh
footprints run --agent aider -- aider
footprints run --agent goose -- goose session
footprints run --agent ollama-loop --dir ~/projects/site -- python agent.py
```

- The agent runs inside the watched directory, which is the current directory or `--dir`.
- `--agent` sets the name in the log. It defaults to the program name.
- Each run gets a new session. `--session <name>` groups several runs, so
  `footprints undo --session <name>` takes all of them back at once.
- `--intent "fix the failing test"` stores a sentence with every action.
- The agent sees `FOOTPRINTS_SESSION` in its environment.
- `Ctrl+C` stops the agent but not Footprints, so the changes made up to that point are still
  recorded.

The whole run is one `command` entry with every file change under it:

```console
$ footprints run --agent demo -- sh agent.sh
footprints: recorded command #1 in session run-bcc287bf with 1 file change(s). See `footprints log --session run-bcc287bf`
$ footprints log --session run-bcc287bf
ID  TIME                 AGENT  SESSION       KIND        STATE           TARGET
 1  2026-11-05 12:57:29  demo   run-bcc287bf  command     not reversible  sh agent.sh
 2  2026-11-05 12:57:29  demo   run-bcc287bf  file_write  reversible      config.toml (via #1)
```

The changes are recorded when the agent exits, so a long interactive session shows up in the log
at the end. A directory over 10,000 files is refused, since a partial snapshot could not be
trusted. Pick a narrower `--dir` or raise `FOOTPRINTS_MAX_FILES`. The details are in
[the wrapper](../how-capture-works.md#the-wrapper).

A shell alias keeps it automatic:

```sh
alias aider='footprints run --agent aider -- aider'
```

## The daemon

For an agent that runs inside an editor or a desktop app, start the daemon on the project folder
before working:

```sh
footprints daemon --root ~/projects/site --snapshot
```

`--snapshot` copies every file under the root first, so the first change to each is reversible.
The daemon runs in the foreground until `Ctrl+C`. It records everything under the root, so edits
made by hand in the same folder are recorded too. Changes are stored as agent `unknown` in a
session named `daemon-` followed by eight characters:

```sh
footprints log --agent unknown
```

What the daemon can and cannot undo is in [the daemon](../how-capture-works.md#the-daemon).

## MCP

The [MCP server](../mcp.md) gives an agent three tools: `footprints_log`, `footprints_diff` and
`footprints_undo`. The agent can then list what it did and undo a step, with the same conflict
check as the CLI and no force option. Most agents take a stdio server as a command and its
arguments:

| Field | Value |
| --- | --- |
| Command | `footprints` |
| Arguments | `mcp` |
| Transport | stdio |

Each agent page below has the exact file and snippet for that agent.

## Agent pages

| Agent | Native adapter | Page |
| --- | --- | --- |
| Aider | No | [Aider](aider.md) |
| Cline | No | [Cline](cline.md) |
| GitHub Copilot CLI and Copilot in VS Code | No | [GitHub Copilot](copilot.md) |
| Windsurf and Devin Desktop | No | [Windsurf and Devin](windsurf.md) |
| OpenCode | No | [OpenCode](opencode.md) |
| Amp | No | [Amp](amp.md) |
| Goose | No | [Goose](goose.md) |
| Continue | No | [Continue](continue.md) |
| Zed's agent | No | [Zed](zed.md) |
| A local model through Ollama | No | [Local models with Ollama](ollama.md) |

Several of these agents have hook or plugin systems of their own. Footprints has no adapter for
their payloads in this version, so their hooks should not be pointed at `footprints hook`. Each
page says what the agent's own hooks are and links to them.

## Check it works

```sh
footprints log --agent <name>
footprints diff <id>
```

## Remove it

Stop prefixing the agent's command with `footprints run` (or remove the alias) and stop the
daemon. Remove the MCP entry from the agent's settings. What was recorded stays in the ledger.
