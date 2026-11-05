# Zed

[Zed's](https://zed.dev/) agent panel runs inside the editor. Zed has no hook system and no
Footprints adapter, so the agent is recorded with [the daemon](other-agents.md#the-daemon). It
can use the [MCP server](../mcp.md) as a context server.

| | |
| --- | --- |
| Route | The daemon, plus MCP |
| Native adapter | No |
| Agent name in the log | `unknown` |
| Official docs | [Agent panel](https://zed.dev/docs/ai/agent-panel), [MCP](https://zed.dev/docs/ai/mcp), [local models](https://zed.dev/docs/ai/use-a-local-model) |

## What is captured

Every file created, changed or deleted under the daemon's roots, with a pre-image when
`--snapshot` is used. That includes edits made by hand in Zed, since the daemon cannot tell the
agent from the person typing.

## What is not captured

- Which agent tool made each change. Rows are stored as agent `unknown`.
- The agent's terminal commands as separate entries. Their file changes are still recorded.
- Files outside the roots.

## Setup

Start the daemon on the project folder before working in Zed:

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Let the agent read and undo its own work

Open the settings file with **zed: open settings file** from the command palette and add:

```json
{
  "context_servers": {
    "footprints": {
      "command": "footprints",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

Zed's settings also offer **Add Server** then **Add Local Server** under the AI section's MCP
servers, which writes the same entry.

## Check it works

```sh
footprints log --agent unknown
```

## Remove it

Stop the daemon and delete `context_servers.footprints` from Zed's settings.

## Links

- [Zed agent panel](https://zed.dev/docs/ai/agent-panel)
- [Zed MCP](https://zed.dev/docs/ai/mcp)
- [Zed with a local model](https://zed.dev/docs/ai/use-a-local-model)
