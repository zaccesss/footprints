# Windsurf and Devin Desktop

Windsurf was renamed Devin Desktop on 2 June 2026. The Windsurf documentation now redirects to
[docs.devin.ai](https://docs.devin.ai/). Cascade is now the legacy agent in Devin Desktop and the
current local agent is Devin Local. None of them has a Footprints adapter. The desktop app is
recorded with [the daemon](other-agents.md#the-daemon), the Devin CLI with
[the wrapper](other-agents.md#the-wrapper) and both can use the [MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The daemon for the desktop app, the `run` wrapper for the Devin CLI, MCP for both |
| Native adapter | No |
| Agent name in the log | `unknown` under the daemon. Whatever `--agent` says under the wrapper |
| Official docs | [Devin Desktop FAQ](https://docs.devin.ai/desktop/devin-desktop-faq), [Cascade MCP](https://docs.devin.ai/windsurf/plugins/cascade/mcp.md), [Devin Local customisations](https://docs.devin.ai/desktop/devin-local/customizations.md), [Devin CLI hooks](https://docs.devin.ai/cli/extensibility/hooks/overview.md) |

## What is captured

Every file created, changed or deleted under the watched folder, with a pre-image. Under the
daemon that includes edits made by hand in the same folder.

## What is not captured

- Which agent tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the watched folder.

Devin Local and the Devin CLI have their own hooks (`PreToolUse`, `PostToolUse` and others) in
`.devin/hooks.v1.json` or a `hooks` key in `.devin/config.json`. Footprints has no adapter for
their payload in this version.

> [!NOTE]
> Devin's hook documentation says it also reads hooks from `.claude/settings.json` and
> `~/.claude/settings.json`. A machine with the [Claude Code](claude-code.md) Footprints block in
> one of those files may see it run under Devin too. That combination has not been tested, so
> treat any rows it produces with care.

## Setup for the desktop app

Start the daemon on the project folder before working:

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Setup for the Devin CLI

```sh
cd ~/projects/site
footprints run --agent devin -- devin
```

## Let the agent read and undo its own work

Open the MCP config file from the Cascade or Devin panel's MCP settings, which opens the right
file for the installed version. The Cascade MCP page names `~/.config/devin/mcp_config.json`
(`%AppData%\devin\mcp_config.json` on Windows). Devin Local also reads a project's
`.devin/mcp_config.json`. Add:

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

The Devin CLI can add it for the user:

```sh
devin mcp add footprints -s user -- footprints mcp
```

## Check it works

```sh
footprints log --agent unknown
footprints log --agent devin
```

## Remove it

Stop the daemon, run `devin` without the prefix and delete the `footprints` entry from
`mcp_config.json`.

## Links

- [Devin Desktop FAQ](https://docs.devin.ai/desktop/devin-desktop-faq)
- [Cascade in Devin Desktop](https://docs.devin.ai/desktop/cascade.md)
- [Cascade MCP servers](https://docs.devin.ai/windsurf/plugins/cascade/mcp.md)
- [Devin Local customisations](https://docs.devin.ai/desktop/devin-local/customizations.md)
- [Devin CLI MCP configuration](https://docs.devin.ai/cli/extensibility/mcp/configuration.md)
- [Devin CLI hooks](https://docs.devin.ai/cli/extensibility/hooks/overview.md)
