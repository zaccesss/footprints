# Continue

[Continue](https://www.continue.dev/) runs as an editor extension and as the `cn` terminal CLI.
It has no hook system and no Footprints adapter. The extension is recorded with
[the daemon](other-agents.md#the-daemon) and the CLI with [the wrapper](other-agents.md#the-wrapper).
Continue's agent mode can use the [MCP server](../mcp.md).

| | |
| --- | --- |
| Route | The daemon for the extension, the `run` wrapper for `cn`, MCP in agent mode |
| Native adapter | No |
| Agent name in the log | `unknown` under the daemon. Whatever `--agent` says under the wrapper |
| Official docs | [MCP](https://docs.continue.dev/customize/deep-dives/mcp), [CLI quick start](https://docs.continue.dev/cli/quickstart), [Ollama](https://docs.continue.dev/customize/model-providers/top-level/ollama) |

## What is captured

Every file created, changed or deleted under the watched folder, with a pre-image.

## What is not captured

- Which Continue tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the watched folder.

## Setup for the extension

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Setup for the CLI

```sh
cd ~/projects/site
footprints run --agent continue -- cn
```

## Let Continue read and undo its own work

MCP tools are available in Continue's agent mode. Add the server to `mcpServers` in
`~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: footprints
    type: stdio
    command: footprints
    args: [mcp]
```

A file of its own in a project's `.continue/mcpServers/` folder works too. Such a file needs
`name`, `version` and `schema: v1` at the top before its `mcpServers` list, as Continue's MCP
page shows.

## Check it works

```sh
footprints log --agent unknown
footprints log --agent continue
```

## Remove it

Stop the daemon, run `cn` without the prefix and delete the `footprints` entry from
`config.yaml`.

## Links

- [Continue MCP](https://docs.continue.dev/customize/deep-dives/mcp)
- [Continue CLI quick start](https://docs.continue.dev/cli/quickstart)
- [Continue with Ollama](https://docs.continue.dev/customize/model-providers/top-level/ollama)
