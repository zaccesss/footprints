# Amp

[Amp](https://ampcode.com/) has a terminal CLI. It has no Footprints adapter, so it is recorded
with [the wrapper](other-agents.md#the-wrapper). It can also use the [MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The `run` wrapper, plus MCP |
| Native adapter | No |
| Agent name in the log | Whatever `--agent` says, `amp` by default |
| Official docs | [CLI](https://ampcode.com/docs/cli), [MCP](https://ampcode.com/docs/customize/mcp), [plugin API](https://ampcode.com/docs/plugin-api) |

## What is captured

Every file Amp or the commands it runs created, changed or deleted under the project directory,
with a pre-image.

## What is not captured

- Which Amp tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the project directory.

Amp has TypeScript plugins with `tool.call` and `tool.result` events. Footprints has no plugin
for them in this version.

## Setup

```sh
cd ~/projects/site
footprints run --agent amp -- amp
```

## Let Amp read and undo its own work

```sh
amp mcp add footprints -- footprints mcp
```

Or add it to `~/.config/amp/settings.json` for the user. A workspace's `.amp/settings.json`
works too. Amp asks for approval before it runs a server from a workspace file:

```json
{
  "amp.mcpServers": {
    "footprints": {
      "command": "footprints",
      "args": ["mcp"]
    }
  }
}
```

## Check it works

```sh
footprints log --agent amp
```

## Remove it

Run `amp` without the prefix and delete the `footprints` entry from `amp.mcpServers`.

## Links

- [Amp CLI](https://ampcode.com/docs/cli)
- [Amp MCP servers](https://ampcode.com/docs/customize/mcp)
- [Amp plugin API](https://ampcode.com/docs/plugin-api)
