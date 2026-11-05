# Cline

[Cline](https://cline.bot/) runs as a VS Code or JetBrains extension and as a terminal CLI. It
has no Footprints adapter. The extension is recorded with [the daemon](other-agents.md#the-daemon)
and the CLI with [the wrapper](other-agents.md#the-wrapper). Both can use the
[MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The daemon for the extension, the `run` wrapper for the CLI, MCP for both |
| Native adapter | No |
| Agent name in the log | `unknown` under the daemon. Whatever `--agent` says under the wrapper |
| Official docs | [MCP servers](https://docs.cline.bot/mcp/configuring-mcp-servers), [CLI](https://docs.cline.bot/usage/cli-overview.md), [plugins](https://docs.cline.bot/customization/plugins.md) |

## What is captured

Every file created, changed or deleted under the watched folder, with a pre-image. Under the
daemon that includes edits made by hand in the same folder.

## What is not captured

- Which Cline tool made each change. No adapter reads Cline's events.
- Commands Cline runs as separate entries. Their file changes are still recorded.
- Files outside the watched folder.

Cline's SDK and CLI have TypeScript plugins with `beforeTool` and `afterTool` events. Cline's
plugin page says they do not apply to the VS Code and JetBrains extensions yet. Footprints has no
plugin for them in this version.

## Setup for the extension

Start the daemon on the project folder before working in the editor:

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Setup for the CLI

Install and sign in as Cline's CLI guide describes, then run it under the wrapper:

```sh
npm i -g cline
cline auth
cd ~/projects/site
footprints run --agent cline -- cline
```

## Let Cline read and undo its own work

In the extension, open the MCP Servers panel, choose **Configure** and then
**Configure MCP Servers**. That opens `cline_mcp_settings.json`. The CLI keeps the same file at
`~/.cline/data/settings/cline_mcp_settings.json`. Add:

```json
{
  "mcpServers": {
    "footprints": {
      "command": "footprints",
      "args": ["mcp"],
      "disabled": false,
      "autoApprove": []
    }
  }
}
```

Or from the CLI:

```sh
cline mcp add footprints --yes -- footprints mcp
```

Leaving `autoApprove` empty keeps Cline asking before each Footprints tool, `footprints_undo`
included.

## Check it works

```sh
footprints log --agent unknown
footprints log --agent cline
```

The first is for the daemon and the second for the wrapper.

## Remove it

Stop the daemon, run `cline` without the prefix and delete the `footprints` entry from
`cline_mcp_settings.json`.

## Links

- [Cline MCP servers](https://docs.cline.bot/mcp/configuring-mcp-servers)
- [Cline CLI](https://docs.cline.bot/usage/cli-overview.md)
- [Cline plugins](https://docs.cline.bot/customization/plugins.md)
- [Cline with Ollama](https://docs.cline.bot/provider-config/ollama)
