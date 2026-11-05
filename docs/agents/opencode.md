# OpenCode

[OpenCode](https://opencode.ai/) is a terminal coding agent. It has no Footprints adapter, so it
is recorded with [the wrapper](other-agents.md#the-wrapper). It can also use the
[MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The `run` wrapper, plus MCP |
| Native adapter | No |
| Agent name in the log | Whatever `--agent` says, `opencode` by default |
| Official docs | [MCP servers](https://opencode.ai/docs/mcp-servers/), [config](https://opencode.ai/docs/config/), [plugins](https://opencode.ai/docs/plugins/) |

## What is captured

Every file OpenCode or the commands it runs created, changed or deleted under the project
directory, with a pre-image.

## What is not captured

- Which OpenCode tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the project directory.

OpenCode has JavaScript and TypeScript plugins with `tool.execute.before`, `tool.execute.after`
and `file.edited` events. Footprints has no plugin for them in this version.

## Setup

```sh
cd ~/projects/site
footprints run --agent opencode -- opencode
```

## Let OpenCode read and undo its own work

Add an `mcp` entry to `opencode.json` at the project root. `~/.config/opencode/opencode.json`
applies it to every project. OpenCode takes the command and its arguments as one array:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "footprints": {
      "type": "local",
      "command": ["footprints", "mcp"],
      "enabled": true
    }
  }
}
```

`opencode mcp list` shows whether it connected.

## Check it works

```sh
footprints log --agent opencode
```

## Remove it

Run `opencode` without the prefix and delete `mcp.footprints` from `opencode.json`.

## Links

- [OpenCode MCP servers](https://opencode.ai/docs/mcp-servers/)
- [OpenCode configuration](https://opencode.ai/docs/config/)
- [OpenCode plugins](https://opencode.ai/docs/plugins/)
- [OpenCode providers, including Ollama](https://opencode.ai/docs/providers/)
