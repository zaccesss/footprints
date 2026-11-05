# MCP server

`footprints mcp` is a [Model Context Protocol](https://modelcontextprotocol.io/) server over
standard input and output. An agent connected to it can read its own ledger and undo its own
mistakes. It records nothing by itself, so it is used beside a hook adapter, the wrapper or the
daemon.

## Tools

| Tool | Arguments | Result |
| --- | --- | --- |
| `footprints_log` | `session`, `agent` and `limit`, all optional. `limit` defaults to 50 | The matching actions as JSON, oldest first |
| `footprints_diff` | `id` | The unified diff of one file action |
| `footprints_undo` | `id` or `session` | What was undone. A conflict comes back as an error result |

`footprints_log` and `footprints_diff` are marked read-only. `footprints_undo` is marked
destructive, so clients that ask before destructive tools ask before it.

An agent can never force an undo through MCP, so it can never overwrite newer work. An undo
that would conflict is refused exactly as `footprints undo` refuses it without `--force`.
Commands and network calls cannot be undone here either.

## Protocol

- Transport: stdio, one JSON-RPC message per line.
- Protocol versions: `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`. The server
  answers with the version the client asks for when it is one of these, otherwise the newest.
- Methods: `initialize`, `ping`, `tools/list` and `tools/call`.
- A tool that fails returns a result with `isError` set. JSON-RPC errors are kept for bad
  arguments (`-32602`), internal faults (`-32603`) and unknown methods (`-32601`).
- The server sends short `instructions` on `initialize` that describe the three tools.

A real exchange, trimmed:

```console
$ printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"footprints_log","arguments":{"limit":2}}}' \
  | footprints mcp
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{"listChanged":false}},"instructions":"Footprints records file writes, shell commands and network calls made by agents on this machine. ...","protocolVersion":"2025-06-18","serverInfo":{"name":"footprints","title":"Footprints","version":"0.2.0"}}}
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"[\n  {\n    \"id\": 4, ...","type":"text"}],"isError":false}}
```

## Registering it

Most clients take a JSON server list in this shape:

```json
{
  "mcpServers": {
    "footprints": { "command": "footprints", "args": ["mcp"] }
  }
}
```

Where each agent keeps that list and its exact format:

| Agent | Where | Guide |
| --- | --- | --- |
| Claude Code | `claude mcp add footprints -- footprints mcp` or `.mcp.json` | [Claude Code](agents/claude-code.md#let-claude-code-read-and-undo-its-own-work) |
| Cursor | `~/.cursor/mcp.json` or `.cursor/mcp.json` | [Cursor](agents/cursor.md#let-cursor-read-and-undo-its-own-work) |
| Codex CLI | `[mcp_servers.footprints]` in `~/.codex/config.toml` | [Codex CLI](agents/codex.md#let-codex-read-and-undo-its-own-work) |
| Gemini CLI | `mcpServers` in `~/.gemini/settings.json` | [Gemini CLI](agents/gemini.md#let-gemini-cli-read-and-undo-its-own-work) |
| Cline | `cline_mcp_settings.json` | [Cline](agents/cline.md#let-cline-read-and-undo-its-own-work) |
| GitHub Copilot CLI | `~/.copilot/mcp-config.json` | [GitHub Copilot](agents/copilot.md#let-copilot-read-and-undo-its-own-work) |
| VS Code | `servers` in `.vscode/mcp.json` | [GitHub Copilot](agents/copilot.md#let-copilot-read-and-undo-its-own-work) |
| Windsurf and Devin Desktop | `mcp_config.json` | [Windsurf and Devin](agents/windsurf.md#let-the-agent-read-and-undo-its-own-work) |
| OpenCode | `mcp` in `opencode.json` | [OpenCode](agents/opencode.md#let-opencode-read-and-undo-its-own-work) |
| Amp | `amp.mcpServers` in `~/.config/amp/settings.json` | [Amp](agents/amp.md#let-amp-read-and-undo-its-own-work) |
| Goose | An extension in `~/.config/goose/config.yaml` | [Goose](agents/goose.md#let-goose-read-and-undo-its-own-work) |
| Continue | `mcpServers` in `~/.continue/config.yaml` | [Continue](agents/continue.md#let-continue-read-and-undo-its-own-work) |
| Zed | `context_servers` in Zed's settings | [Zed](agents/zed.md#let-the-agent-read-and-undo-its-own-work) |

The server reads `FOOTPRINTS_HOME` like every other command. A client that starts it with a
different environment from the hooks sees a different ledger. Set the variable in the server's
`env` when it is not the default.

## Limits

- MCP tool calls themselves, Footprints' own included, are not recorded by any hook adapter yet.
- Undo results go through the same checks as the CLI. The full list is in [undo](undo.md).
