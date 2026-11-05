# Footprints documentation

The [README](../README.md) is the short version. These pages hold the detail.

## Using Footprints

| Page | What it covers |
| --- | --- |
| [Getting started](getting-started.md) | Installing on macOS, Linux and Windows, a first recorded run and where data lives |
| [How capture works](how-capture-works.md) | The three capture tiers: hooks, the `run` wrapper and the daemon, plus snapshots, budgets and the blob store |
| [Undo](undo.md) | Single and session undo, the conflict check, `--force`, `recover` and the full table of what can and cannot be undone |
| [Commands](commands.md) | Every subcommand, flag, exit status and environment variable, from the real `--help` output |
| [MCP server](mcp.md) | The three MCP tools, the protocol details and where each agent registers the server |
| [OpenTelemetry export](export.md) | The span layout and attributes of `footprints export --otel` |
| [Data and privacy](data-and-privacy.md) | What is stored, file permissions, settings, secrets in the blob store and pruning or restoring history |
| [Menu bar app](menu-bar-app.md) | The tray app in `app/` for macOS, Windows and Linux: installing, building, starting at login and its limits |

## Agents

| Page | What it covers |
| --- | --- |
| [Agent guides](agents/README.md) | Every agent, the route each uses and what each route sees and misses |
| [Claude Code](agents/claude-code.md) | Native hooks and MCP |
| [Cursor](agents/cursor.md) | Native hooks and MCP |
| [OpenAI Codex CLI](agents/codex.md) | Native hooks, hook trust and MCP |
| [Gemini CLI](agents/gemini.md) | Native hooks and MCP, plus Antigravity CLI |
| [Any other agent](agents/other-agents.md) | The wrapper, the daemon and MCP for agents with no adapter |
| [Aider](agents/aider.md) | The wrapper |
| [Cline](agents/cline.md) | The daemon or the wrapper, plus MCP |
| [GitHub Copilot](agents/copilot.md) | The Copilot CLI and Copilot in VS Code |
| [Windsurf and Devin](agents/windsurf.md) | Devin Desktop, Cascade and the Devin CLI |
| [OpenCode](agents/opencode.md) | The wrapper plus MCP |
| [Amp](agents/amp.md) | The wrapper plus MCP |
| [Goose](agents/goose.md) | The wrapper or the daemon, plus MCP as an extension |
| [Continue](agents/continue.md) | The daemon or the wrapper, plus MCP |
| [Zed](agents/zed.md) | The daemon plus MCP |
| [Local models with Ollama](agents/ollama.md) | Agents that drive a local model and home-made loops |

## Project

| Page | What it covers |
| --- | --- |
| [Roadmap](../ROADMAP.md) | What is done, what is next and what is deliberately later |
| [Changelog](../CHANGELOG.md) | What each release shipped |
| [Contributing](../CONTRIBUTING.md) | How to propose a change |
| [Accessibility](../ACCESSIBILITY.md) | How the CLI output and the app stay readable without colour |
| [Security](../SECURITY.md) | How to report a vulnerability |
| [Workflows](../.github/workflows/README.md) | What CI and the release workflow run |
