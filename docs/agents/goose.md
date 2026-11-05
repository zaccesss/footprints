# Goose

[Goose](https://goose-docs.ai/) is an open source agent with a terminal CLI and a desktop app,
now maintained under the Agentic AI Foundation. It has no Footprints adapter. The CLI is
recorded with [the wrapper](other-agents.md#the-wrapper) and the desktop app with
[the daemon](other-agents.md#the-daemon). Both can use the [MCP server](../mcp.md) as a Goose
extension.

| | |
| --- | --- |
| Route | The `run` wrapper for the CLI, the daemon for the desktop app, MCP as an extension |
| Native adapter | No |
| Agent name in the log | Whatever `--agent` says under the wrapper. `unknown` under the daemon |
| Official docs | [CLI commands](https://goose-docs.ai/docs/guides/goose-cli-commands/), [extensions](https://goose-docs.ai/docs/getting-started/using-extensions), [config files](https://goose-docs.ai/docs/guides/config-files), [hooks](https://goose-docs.ai/docs/guides/context-engineering/hooks/) |

## What is captured

Every file Goose or the commands it runs created, changed or deleted under the watched folder,
with a pre-image.

## What is not captured

- Which Goose tool made each change.
- Commands as separate entries. Their file changes are still recorded.
- Files outside the watched folder.

Goose has its own hooks, following the Open Plugins hooks format, in a plugin's
`hooks/hooks.json` under `~/.agents/plugins/` or a project's `.agents/plugins/`. Its events
include `PreToolUse`, `PostToolUse`, `AfterFileEdit` and `AfterShellExecution`, but its payload
differs from every Footprints adapter. Do not point them at `footprints hook` in this version.

## Setup for the CLI

```sh
cd ~/projects/site
footprints run --agent goose -- goose session
```

## Setup for the desktop app

```sh
footprints daemon --root ~/projects/site --snapshot
```

## Let Goose read and undo its own work

For one session only:

```sh
goose session --with-extension "footprints mcp"
```

That combines with the wrapper:

```sh
footprints run --agent goose -- goose session --with-extension "footprints mcp"
```

To keep it, run `goose configure`, choose **Add Extension**, then **Command-line Extension**,
name it `footprints` and give `footprints mcp` as the command. That writes an entry to
`~/.config/goose/config.yaml` (`%APPDATA%\Block\goose\config\config.yaml` on Windows) like this:

```yaml
extensions:
  footprints:
    name: footprints
    type: stdio
    cmd: footprints
    args: [mcp]
    enabled: true
    timeout: 300
```

## Check it works

```sh
footprints log --agent goose
```

## Remove it

Run `goose session` without the prefix and remove the extension through `goose configure` or by
deleting it from `config.yaml`.

## Links

- [Goose CLI commands](https://goose-docs.ai/docs/guides/goose-cli-commands/)
- [Goose extensions](https://goose-docs.ai/docs/getting-started/using-extensions)
- [Goose config files](https://goose-docs.ai/docs/guides/config-files)
- [Goose hooks](https://goose-docs.ai/docs/guides/context-engineering/hooks/)
- [Goose providers, including Ollama](https://goose-docs.ai/docs/getting-started/providers)
