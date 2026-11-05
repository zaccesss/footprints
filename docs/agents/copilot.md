# GitHub Copilot

This page covers two agents: the GitHub Copilot CLI (`copilot`) in a terminal and Copilot agent
mode in VS Code. Neither has a Footprints adapter. The CLI is recorded with
[the wrapper](other-agents.md#the-wrapper) and VS Code with [the daemon](other-agents.md#the-daemon).
Both can use the [MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The `run` wrapper for the CLI, the daemon for VS Code, MCP for both |
| Native adapter | No |
| Agent name in the log | Whatever `--agent` says under the wrapper. `unknown` under the daemon |
| Official docs | [Copilot CLI](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli), [CLI MCP servers](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers), [CLI hooks](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-hooks), [VS Code MCP](https://code.visualstudio.com/docs/copilot/customization/mcp-servers), [VS Code hooks](https://code.visualstudio.com/docs/copilot/customization/hooks) |

## What is captured

Every file created, changed or deleted under the watched folder, with a pre-image.

## What is not captured

- Which Copilot tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the watched folder.

Both the CLI and VS Code have hook systems of their own, with `preToolUse` and `postToolUse`
events in `.github/hooks/*.json` or `~/.copilot/hooks/`. Their payloads differ from every agent
Footprints has an adapter for, so they should not be pointed at `footprints hook`.

> [!NOTE]
> VS Code's hook documentation says it also reads hooks from `.claude/settings.json` and
> `.claude/settings.local.json`. A project that already has the [Claude Code](claude-code.md)
> Footprints block there may see those hooks run under Copilot in VS Code too. That combination
> has not been tested, so treat any rows it produces with care.

## Setup for the Copilot CLI

Run it in the project folder under the wrapper:

```sh
cd ~/projects/site
footprints run --agent copilot -- copilot
```

## Setup for VS Code

Start the daemon on the workspace folder before using agent mode:

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Let Copilot read and undo its own work

For the CLI:

```sh
copilot mcp add footprints -- footprints mcp
```

That writes to `~/.copilot/mcp-config.json`. `/mcp add` inside a session does the same. The
file entry looks like this:

```json
{
  "mcpServers": {
    "footprints": {
      "type": "local",
      "command": "footprints",
      "args": ["mcp"],
      "tools": ["*"]
    }
  }
}
```

For VS Code, add `.vscode/mcp.json` to the workspace. VS Code uses `servers`, not `mcpServers`:

```json
{
  "servers": {
    "footprints": {
      "type": "stdio",
      "command": "footprints",
      "args": ["mcp"]
    }
  }
}
```

**MCP: Open User Configuration** in the Command Palette adds it for every workspace instead.

## Check it works

```sh
footprints log --agent copilot
footprints log --agent unknown
```

The first is for the CLI and the second for VS Code under the daemon.

## Remove it

Run `copilot` without the prefix, stop the daemon and delete the `footprints` entry from
`~/.copilot/mcp-config.json` or `.vscode/mcp.json`.

## Links

- [Using the Copilot CLI](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli)
- [Adding MCP servers to the Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers)
- [Copilot CLI hooks](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-hooks)
- [MCP servers in VS Code](https://code.visualstudio.com/docs/copilot/customization/mcp-servers)
- [Agent hooks in VS Code](https://code.visualstudio.com/docs/copilot/customization/hooks)
