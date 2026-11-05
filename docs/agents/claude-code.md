# Claude Code

Claude Code has a native Footprints adapter. Its hooks call `footprints hook pre` before a tool
runs and `footprints hook post` after it, so each action is stored with the tool name, the
agent's own description of a shell command and Claude Code's `session_id`.

| | |
| --- | --- |
| Route | Native hooks, plus the [MCP server](../mcp.md) if wanted |
| Agent name in the log | `claude-code` |
| Session | The hook's `session_id` |
| Pairing | `tool_use_id` |
| Official docs | [Hooks reference](https://code.claude.com/docs/en/hooks), [settings](https://code.claude.com/docs/en/settings), [MCP](https://code.claude.com/docs/en/mcp) |

## What is captured

| Tool | Recorded as | Undo |
| --- | --- | --- |
| `Write` and `Edit` | A file write or create, with a pre-image | Yes |
| `MultiEdit`, in older Claude Code versions that still have it | The same as `Edit` | Yes |
| `NotebookEdit` | A file write on the notebook, with a pre-image | Yes |
| `Bash` | A command with its `description` as the intent, plus every file it changed in its working directory | The file changes, one by one |
| `PowerShell` | The same as `Bash`, for Claude Code's native Windows shell tool | The file changes, one by one |
| `WebFetch` | A network call to its URL | No, it is recorded to be seen |
| `WebSearch` | A network call with its query | No |

Claude Code fires `PostToolUse` only when a tool succeeds. A tool that fails after it started,
such as a Bash command that exits non-zero partway through, fires `PostToolUseFailure` instead.
Both go to `footprints hook post`, so a failed call still has its file changes recorded. The
error goes in its note. An interrupted call says so too.

## What is not captured

- MCP tool calls (`mcp__<server>__<tool>`) are not recorded by any adapter yet.
- Tools outside the matcher, such as `Read`, `Glob` and `Grep`, are not recorded. They do not
  change files.
- A Bash call with `run_in_background` is only partly captured: only changes made by the time
  the command has started are recorded.
- A Bash snapshot over the [budgets](../how-capture-works.md#snapshots-and-budgets) is abandoned.
  The command is recorded but its file changes are not. Its note says so.
- A pre hook whose tool is then denied leaves a `pending` entry that cannot be undone.

## Setup

1. Make sure `footprints` is on the `PATH` that Claude Code sees:

   ```sh
   command -v footprints
   ```

   A full path to the binary in the hook command works too.

2. Print the hook configuration:

   ```sh
   footprints install claude-code --print
   ```

   ```json
   {
     "hooks": {
       "PreToolUse": [
         {
           "matcher": "Write|Edit|MultiEdit|NotebookEdit|Bash|PowerShell|WebFetch|WebSearch",
           "hooks": [
             {
               "type": "command",
               "command": "footprints hook pre"
             }
           ]
         }
       ],
       "PostToolUseFailure": [
         {
           "matcher": "Write|Edit|MultiEdit|NotebookEdit|Bash|PowerShell|WebFetch|WebSearch",
           "hooks": [
             {
               "type": "command",
               "command": "footprints hook post"
             }
           ]
         }
       ],
       "PostToolUse": [
         {
           "matcher": "Write|Edit|MultiEdit|NotebookEdit|Bash|PowerShell|WebFetch|WebSearch",
           "hooks": [
             {
               "type": "command",
               "command": "footprints hook post"
             }
           ]
         }
       ]
     }
   }
   ```

   The same JSON is in [`hooks/claude-code/settings.example.json`](../../hooks/claude-code/settings.example.json).
   Claude Code reads a matcher made only of names and `|` as a list of exact tool names, so it
   never matches a tool by accident.

3. Merge the `hooks` block into one settings file. If the file already has a `hooks` key, add
   these entries to its event arrays rather than replacing it.

   | File | Scope |
   | --- | --- |
   | `~/.claude/settings.json` | Every project on the machine |
   | `.claude/settings.json` | One project, shared through the repository |
   | `.claude/settings.local.json` | One project, kept out of the repository |

4. Start a new Claude Code session. `/hooks` inside Claude Code lists the hooks it loaded and
   which file each came from.

The hooks print nothing on standard output and always exit 0, so they never block or alter a
tool call. Errors go to standard error starting with `footprints: hook error:`.

## Check it works

Ask Claude Code to create or edit a file, then in a terminal:

```sh
footprints log --agent claude-code
```

Each tool call appears with its tool name. Each Bash call has its description in
`footprints show <id>`. `footprints diff <id>` shows what a write changed.

If nothing appears, check that the agent and the terminal see the same `FOOTPRINTS_HOME` and that
`footprints` is on Claude Code's `PATH`. Starting Claude Code with `claude --debug` prints hook
errors.

## Let Claude Code read and undo its own work

The [MCP server](../mcp.md) lets Claude Code list its actions and undo one, with the conflict
check and no force option:

```sh
claude mcp add footprints -- footprints mcp
```

That adds it for the current project only. `--scope user` adds it for every project.
`--scope project` writes it to `.mcp.json` for everyone who works on the repository:

```json
{
  "mcpServers": {
    "footprints": {
      "type": "stdio",
      "command": "footprints",
      "args": ["mcp"]
    }
  }
}
```

`claude mcp list` or `/mcp` inside a session confirms it is connected.

## Remove it

1. Delete the three Footprints entries from the `hooks` block of the settings file.
2. Run `claude mcp remove footprints` if the MCP server was added.
3. Start a new session. `/hooks` confirms nothing is left.

What was already recorded stays in the ledger until the data directory is deleted.

## Links

- [Claude Code hooks reference](https://code.claude.com/docs/en/hooks)
- [Claude Code settings files](https://code.claude.com/docs/en/settings)
- [Claude Code MCP servers](https://code.claude.com/docs/en/mcp)
- [How the hook adapters work](../how-capture-works.md#hook-adapters)
- [What can and cannot be undone](../undo.md#what-can-and-cannot-be-undone)
