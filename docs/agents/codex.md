# OpenAI Codex CLI

Codex CLI has a native Footprints adapter. Codex runs `footprints hook pre --from codex` before
a shell command or a patch and `footprints hook post --from codex` after it. Each action is
stored with Codex's `session_id` and paired by `tool_use_id`.

| | |
| --- | --- |
| Route | Native hooks, plus the [MCP server](../mcp.md) if wanted |
| Agent name in the log | `codex` |
| Session | The hook's `session_id` |
| Pairing | `tool_use_id` |
| Official docs | [Hooks](https://learn.chatgpt.com/docs/hooks), [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference), [MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) |

OpenAI's Codex documentation moved from `developers.openai.com/codex` to `learn.chatgpt.com/docs`.
The old links redirect.

## What is captured

| Tool | Recorded as | Undo |
| --- | --- | --- |
| `apply_patch` | One file action for every file named in the patch headers, with a pre-image. Added, updated, deleted and moved files are all covered | Yes, each file on its own |
| `Bash` | A command, plus every file it changed in its working directory | The file changes, one by one |

Codex sends both tools' text in `tool_input.command`: the shell command for `Bash` and the whole
patch for `apply_patch`. Footprints reads the file paths from the patch's `*** Add File`,
`*** Update File`, `*** Delete File` and `*** Move to` headers.

Codex has no failure event. Its `PostToolUse` also runs after a shell command that exits with a
non-zero status, so a failed call is recorded like a finished one.

## What is not captured

- Hosted tools such as web search. Codex documents that they "don't use the local function-tool
  hook path", so no hook fires for them and no network call is recorded.
- MCP tool calls. Codex does fire hooks for them, named like `mcp__filesystem__read_file`, but
  the Footprints matcher `^(Bash|apply_patch)$` leaves them out and no adapter records MCP
  tools yet.
- File changes Codex makes outside `apply_patch` and `Bash`. Running Codex under
  [the wrapper](other-agents.md) as well catches anything under the project directory.
- A shell snapshot over the [budgets](../how-capture-works.md#snapshots-and-budgets) is
  abandoned. The command is recorded but its file changes are not.
- The legacy `notify` setting in `config.toml` only reports finished turns. Footprints does not
  use it.

## Setup

1. Make sure `footprints` is on the `PATH` that Codex sees:

   ```sh
   command -v footprints
   ```

2. Print the hook configuration:

   ```sh
   footprints install codex --print
   ```

   ```json
   {
     "hooks": {
       "PreToolUse": [
         {
           "matcher": "^(Bash|apply_patch)$",
           "hooks": [{ "type": "command", "command": "footprints hook pre --from codex" }]
         }
       ],
       "PostToolUse": [
         {
           "matcher": "^(Bash|apply_patch)$",
           "hooks": [{ "type": "command", "command": "footprints hook post --from codex" }]
         }
       ]
     }
   }
   ```

   The same JSON is in [`hooks/codex/hooks.example.json`](../../hooks/codex/hooks.example.json).
   Codex reads `matcher` as a regular expression. Its shell tool and its unified exec tool both
   match as `Bash`.

3. Merge it into one hooks file. If the file already has hooks, add these entries to its event
   arrays.

   | File | Scope |
   | --- | --- |
   | `~/.codex/hooks.json` | Every project |
   | `<repo>/.codex/hooks.json` | One project. Codex loads it only when the project's `.codex/` layer is trusted |

   Codex also reads hooks from inline `[hooks]` tables in `~/.codex/config.toml` or a project's
   `.codex/config.toml`, with the same event schema. The JSON file is the simpler place to paste
   the printed block.

4. Hooks are on by default in current Codex versions. If they were turned off, enable them in
   `~/.codex/config.toml`:

   ```toml
   [features]
   hooks = true
   ```

   Older versions used `codex_hooks` as the key, which still works as an alias.

5. Start Codex and run `/hooks`. Codex runs no new or changed hook until it has been reviewed
   and trusted there. Trust is tied to the exact hook definition, so editing the block later
   means trusting it again.

The hooks print nothing on standard output and always exit 0. Codex treats that as "no opinion"
and carries on.

## Check it works

Ask Codex to edit a file, then in a terminal:

```sh
footprints log --agent codex
```

An `apply_patch` call shows one row per file. `footprints show <id>` on a patch row shows the
tool and the session. If nothing appears, check that the hooks are trusted in `/hooks`, that
`footprints` is on the `PATH` Codex sees and that both sides use the same `FOOTPRINTS_HOME`.

## Let Codex read and undo its own work

Register the [MCP server](../mcp.md) in `~/.codex/config.toml`. A trusted project's
`.codex/config.toml` works too:

```toml
[mcp_servers.footprints]
command = "footprints"
args = ["mcp"]
```

Or add it from the command line:

```sh
codex mcp add footprints -- footprints mcp
```

`/mcp` inside Codex lists the connected servers.

## Remove it

1. Delete the two Footprints entries from `hooks.json` (or the matching `[hooks]` tables).
2. Delete the `[mcp_servers.footprints]` table from `config.toml` if it was added.
3. Restart Codex. `/hooks` confirms the hooks are gone.

## Links

- [Codex hooks](https://learn.chatgpt.com/docs/hooks)
- [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex advanced configuration, including `notify`](https://learn.chatgpt.com/docs/config-file/config-advanced)
- [Codex MCP servers](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
- [How the hook adapters work](../how-capture-works.md#hook-adapters)
