# Cursor

Cursor has a native Footprints adapter for its agent. Cursor's hooks call
`footprints hook pre --from cursor` and `footprints hook post --from cursor`. Each action is
stored under Cursor's `conversation_id`.

| | |
| --- | --- |
| Route | Native hooks, plus the [MCP server](../mcp.md) if wanted |
| Agent name in the log | `cursor` |
| Session | The hook's `conversation_id` |
| Pairing | `tool_use_id` for file tools. Conversation and command text for shell commands, since the shell hooks carry no call id |
| Official docs | [Hooks](https://cursor.com/docs/hooks), [MCP](https://cursor.com/docs/context/mcp) |

## What is captured

| Hook | Recorded as | Undo |
| --- | --- | --- |
| `preToolUse` and `postToolUse` on `Write` and `Delete` | A file write, create or delete, with a real pre-image | Yes |
| `postToolUseFailure` on `Write` and `Delete` | The failed call, with Cursor's `error_message` in its note | Whatever the call changed before it failed |
| `beforeShellExecution` and `afterShellExecution` | A command, stored as tool `Shell`, plus every file it changed in its working directory | The file changes, one by one |
| `afterFileEdit` | A fallback for an edit no `preToolUse` saw, stored as tool `afterFileEdit` | Only when the pre-image could be rebuilt exactly |

`afterFileEdit` fires after the file is already written, so there is no pre-image to read.
Footprints rebuilds one by reversing Cursor's `edits` list of old and new text on the current
file. It keeps that pre-image only when applying the edits again gives back the file byte for
byte. An ambiguous edit, such as one whose new text appears twice, is recorded as not reversible
rather than guessed.

## What is not captured

- Web calls. Cursor has no web tool hook.
- MCP tool calls. Cursor has `beforeMCPExecution` and `afterMCPExecution` hooks, but no adapter
  records MCP tools yet.
- Tab completions. `afterTabFileEdit` is not registered.
- File reads (`beforeReadFile`), since they change nothing.
- Edits by Cursor's editor features outside the agent. [The daemon](../how-capture-works.md#the-daemon)
  records those as agent `unknown`.

## Setup

1. Make sure `footprints` is on the `PATH` that Cursor sees. On macOS an app started from the
   Dock may not see a shell profile's `PATH`, so a full path to the binary in each `command` is
   the safe choice:

   ```sh
   command -v footprints
   ```

2. Print the hook configuration:

   ```sh
   footprints install cursor --print
   ```

   ```json
   {
     "version": 1,
     "hooks": {
       "preToolUse": [{ "command": "footprints hook pre --from cursor", "matcher": "Write|Delete" }],
       "postToolUse": [{ "command": "footprints hook post --from cursor", "matcher": "Write|Delete" }],
       "postToolUseFailure": [{ "command": "footprints hook post --from cursor", "matcher": "Write|Delete" }],
       "beforeShellExecution": [{ "command": "footprints hook pre --from cursor" }],
       "afterShellExecution": [{ "command": "footprints hook post --from cursor" }],
       "afterFileEdit": [{ "command": "footprints hook post --from cursor" }]
     }
   }
   ```

   The same JSON is in [`hooks/cursor/hooks.example.json`](../../hooks/cursor/hooks.example.json).
   Cursor reads `matcher` as a regular expression on the tool name.

3. Merge it into one hooks file. If the file already has hooks, add these entries to its event
   arrays.

   | File | Scope |
   | --- | --- |
   | `~/.cursor/hooks.json` | Every project |
   | `<project>/.cursor/hooks.json` | One project |

4. Restart Cursor so it loads the file.

The before hooks always print `{"permission":"allow"}` on standard output, even after an error.
Cursor blocks an action whose hook replies with invalid JSON or a reply that does not match the
hook's schema, so an empty reply is not safe. The after hooks print nothing.

> [!WARNING]
> Cursor's documentation does not say whether an `allow` from a hook also skips Cursor's own
> approval prompt or allow list. Cursor staff have said on the
> [Cursor forum](https://forum.cursor.com/t/beforeshellexecution-hook-permissions-allow-ask-ignored-allow-list-takes-precedence/144244)
> that it does not today. Check this in the Cursor version in use before relying on that prompt.

## Check it works

Ask Cursor's agent to create a file and run a shell command, then in a terminal:

```sh
footprints log --agent cursor
```

File rows show tool `Write` or `Delete`. Shell rows show tool `Shell`. A row with tool
`afterFileEdit` came from the fallback path.

## Cursor CLI

Cursor's documentation says only that hooks run "in the Cursor desktop app and CLI" for one
event. Reports on the Cursor forum say the CLI reads the same `hooks.json` and fires the shell
and `afterFileEdit` hooks. Neither confirms `preToolUse` or `postToolUseFailure` there. Running
the CLI under [the wrapper](other-agents.md) as well records every file change under the
project whichever hooks fire.

## Let Cursor read and undo its own work

Register the [MCP server](../mcp.md) in `~/.cursor/mcp.json` for every project or
`.cursor/mcp.json` for one:

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

Cursor's MCP settings page lists the server once it has started.

## Remove it

1. Delete the six Footprints entries from `hooks.json`.
2. Delete the `footprints` entry from `mcp.json` if it was added.
3. Restart Cursor.

## Links

- [Cursor hooks](https://cursor.com/docs/hooks)
- [Cursor MCP](https://cursor.com/docs/context/mcp)
- [How the hook adapters work](../how-capture-works.md#hook-adapters)
