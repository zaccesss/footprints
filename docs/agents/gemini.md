# Gemini CLI

Gemini CLI has a native Footprints adapter. Its hooks call `footprints hook pre --from gemini`
before a tool and `footprints hook post --from gemini` after it. Each action is stored with
Gemini CLI's `session_id`.

| | |
| --- | --- |
| Route | Native hooks, plus the [MCP server](../mcp.md) if wanted |
| Agent name in the log | `gemini-cli` |
| Session | The hook's `session_id` |
| Pairing | Session, tool and target, since Gemini CLI gives no call id. A pair must be within one minute |
| Official docs | [Hooks](https://geminicli.com/docs/hooks/), [hooks reference](https://geminicli.com/docs/hooks/reference/), [configuration](https://geminicli.com/docs/reference/configuration/) |

> [!NOTE]
> Gemini CLI's own hooks page says: "Unpaid tier and Google One users: Gemini CLI was replaced by
> Antigravity CLI on June 18th, 2026." Antigravity CLI uses a different hook format (see
> [Antigravity CLI](#antigravity-cli) below), so this adapter does not work with it.

## What is captured

| Tool | Recorded as | Undo |
| --- | --- | --- |
| `write_file` | A file write or create, with a pre-image | Yes |
| `replace` | A file write, with a pre-image | Yes |
| `run_shell_command` | A command, plus every file it changed in its working directory. Its `dir_path` is used when given | The file changes, one by one |
| `web_fetch` | A network call. Its target is the prompt, which holds the URLs | No |
| `google_web_search` | A network call with its query | No |

A failure in `tool_response` is noted on the action.

## What is not captured

- MCP tool calls, which no adapter records yet.
- Other built-in tools, such as file reads, which change nothing.
- A shell snapshot over the [budgets](../how-capture-works.md#snapshots-and-budgets) is
  abandoned. The command is recorded but its file changes are not.

## Setup

1. Make sure `footprints` is on the `PATH` that Gemini CLI sees:

   ```sh
   command -v footprints
   ```

2. Print the hook configuration:

   ```sh
   footprints install gemini --print
   ```

   ```json
   {
     "hooks": {
       "BeforeTool": [
         {
           "matcher": "write_file|replace|run_shell_command|web_fetch|google_web_search",
           "hooks": [
             {
               "name": "footprints-pre",
               "type": "command",
               "command": "footprints hook pre --from gemini",
               "timeout": 60000
             }
           ]
         }
       ],
       "AfterTool": [
         {
           "matcher": "write_file|replace|run_shell_command|web_fetch|google_web_search",
           "hooks": [
             {
               "name": "footprints-post",
               "type": "command",
               "command": "footprints hook post --from gemini",
               "timeout": 60000
             }
           ]
         }
       ]
     }
   }
   ```

   The same JSON is in [`hooks/gemini-cli/settings.example.json`](../../hooks/gemini-cli/settings.example.json).
   Gemini CLI reads `matcher` as a regular expression for tool events. `timeout` is in
   milliseconds. 60000 is also Gemini CLI's default.

3. Merge the `hooks` block into one settings file. If the file already has a `hooks` key, add
   these entries to its event arrays.

   | File | Scope |
   | --- | --- |
   | `~/.gemini/settings.json` | Every project |
   | `.gemini/settings.json` | One project |

4. Hooks are on by default. `hooksConfig.enabled` in the same file turns the whole hook system
   off when set to `false`, so check it is not.

Both hooks print `{}` on standard output, since Gemini CLI parses standard output as JSON. They
always exit 0.

## Check it works

Ask Gemini CLI to write a file, then in a terminal:

```sh
footprints log --agent gemini-cli
```

## Let Gemini CLI read and undo its own work

Add the [MCP server](../mcp.md) under `mcpServers` in the same settings file:

```json
{
  "mcpServers": {
    "footprints": {
      "command": "footprints",
      "args": ["mcp"]
    }
  }
}
```

`/mcp` inside Gemini CLI lists the connected servers.

## Antigravity CLI

Antigravity CLI has hooks, but in a different format from Gemini CLI. Its hooks file is
`.agents/hooks.json` in a workspace or `~/.gemini/config/hooks.json` globally. Its events are
`PreToolUse` and `PostToolUse`. Its payload is camelCase with a `toolCall` object. Its tool names
differ (`write_to_file`, `replace_file_content`, `run_command`) and a `PreToolUse` reply must
carry a `decision`. None of that matches any Footprints adapter in this version. See the
[Antigravity hooks documentation](https://antigravity.google/docs/hooks?tab=cli).

Until there is an adapter, Antigravity CLI works with [the wrapper](other-agents.md), which
records every file change under the project:

```sh
footprints run --agent antigravity -- <antigravity command>
```

Replace `<antigravity command>` with the command that starts Antigravity CLI on the machine.

## Remove it

1. Delete the two Footprints entries from the `hooks` block.
2. Delete `mcpServers.footprints` if it was added.
3. Restart Gemini CLI.

## Links

- [Gemini CLI hooks](https://geminicli.com/docs/hooks/)
- [Gemini CLI hooks reference](https://geminicli.com/docs/hooks/reference/)
- [Gemini CLI configuration, including MCP servers](https://geminicli.com/docs/reference/configuration/)
- [Gemini CLI shell tool](https://geminicli.com/docs/tools/shell/)
- [Antigravity CLI hooks](https://antigravity.google/docs/hooks?tab=cli)
- [How the hook adapters work](../how-capture-works.md#hook-adapters)
