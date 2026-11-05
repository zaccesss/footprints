# Agent guides

Footprints records any agent through one of four routes. Four agents have native hook
adapters. Every other agent is recorded by the wrapper or the daemon, which watch the files
rather than the agent.

## Routes

| Route | How it connects | Sees | Misses |
| --- | --- | --- | --- |
| Native hooks | The agent runs `footprints hook pre` and `footprints hook post` around each tool call | The tool, the agent's session, the stated intent of shell commands, file writes with pre-images, the files each shell command changed and web calls where the agent has a hook for them | MCP tool calls. Tools outside the matcher. Changes made outside the agent's tools |
| [The `run` wrapper](../how-capture-works.md#the-wrapper) | `footprints run -- <agent command>` | Every file the agent or its subprocesses changed under the watched directory | Which tool made each change. Web calls. Files outside the directory. Agents that are not started from a terminal |
| [The daemon](../how-capture-works.md#the-daemon) | `footprints daemon --root <dir>` | Every file any process changed under the roots | The agent's name, tools and intent. The first change to a file unless `--snapshot` is used |
| [MCP](../mcp.md) | The agent starts `footprints mcp` as a stdio server | Nothing by itself. It lets the agent read the ledger and undo its own changes | Recording. Pair it with another route |

Hooks and the wrapper can run together. The hooks give the intent behind each call and the
wrapper catches anything the hooks miss.

## Agents

| Agent | Native adapter | Route | MCP | Guide |
| --- | --- | --- | --- | --- |
| Claude Code | Yes | Native hooks | Yes | [Claude Code](claude-code.md) |
| Cursor | Yes | Native hooks | Yes | [Cursor](cursor.md) |
| OpenAI Codex CLI | Yes | Native hooks | Yes | [Codex CLI](codex.md) |
| Gemini CLI | Yes | Native hooks | Yes | [Gemini CLI](gemini.md) |
| Antigravity CLI | No | The `run` wrapper | Not covered | [Gemini CLI](gemini.md#antigravity-cli) |
| Aider | No | The `run` wrapper | No | [Aider](aider.md) |
| Cline | No | The daemon for the extension, the wrapper for the CLI | Yes | [Cline](cline.md) |
| GitHub Copilot CLI | No | The `run` wrapper | Yes | [GitHub Copilot](copilot.md) |
| Copilot agent mode in VS Code | No | The daemon | Yes | [GitHub Copilot](copilot.md) |
| Windsurf and Devin Desktop | No | The daemon for the app, the wrapper for the Devin CLI | Yes | [Windsurf and Devin](windsurf.md) |
| OpenCode | No | The `run` wrapper | Yes | [OpenCode](opencode.md) |
| Amp | No | The `run` wrapper | Yes | [Amp](amp.md) |
| Goose | No | The wrapper for the CLI, the daemon for the app | Yes | [Goose](goose.md) |
| Continue | No | The daemon for the extension, the wrapper for `cn` | Yes, in agent mode | [Continue](continue.md) |
| Zed's agent | No | The daemon | Yes | [Zed](zed.md) |
| A local model through Ollama | No | Whichever route its agent uses | Depends on the agent | [Local models with Ollama](ollama.md) |
| Anything else | No | The wrapper or the daemon | If the agent supports stdio MCP | [Any other agent](other-agents.md) |

## Hook coverage per native adapter

Each adapter is built from the agent's own hook documentation and only uses events and fields
the documentation describes.

| Agent | File writes and edits | Shell commands | Web calls | Pairing a post hook with its pre hook |
| --- | --- | --- | --- | --- |
| Claude Code | `Write`, `Edit`, `MultiEdit` and `NotebookEdit`, with a pre-image | `Bash` and `PowerShell`, with every file changed in its working directory. Failed calls too, through `PostToolUseFailure` | `WebFetch` and `WebSearch` | `tool_use_id` |
| Cursor | `Write` and `Delete` through `preToolUse` and `postToolUse`, with a pre-image. `afterFileEdit` is a fallback for an edit no `preToolUse` saw: its pre-image is rebuilt from the edit's old and new text and kept only when applying the edit again gives back the file exactly. A failed call is noted through `postToolUseFailure` | `beforeShellExecution` and `afterShellExecution`, with every file changed in the working directory | Not available: Cursor has no web tool hook | `tool_use_id` for file tools. Conversation and command text for shell commands, since the shell hooks carry no call id |
| Codex CLI | `apply_patch`, with a pre-image for every file named in the patch headers, moves included | `Bash`, with every file changed in its working directory. A failed call is recorded like a finished one, since Codex has no failure hook | Not available: Codex documents that its hosted tools such as web search do not run hooks | `tool_use_id` |
| Gemini CLI | `write_file` and `replace`, with a pre-image | `run_shell_command`, using its `dir_path` when given. A failure in `tool_response` is noted | `web_fetch` (its prompt, which holds the URLs) and `google_web_search` | Session, tool and target, since Gemini CLI gives no call id |

MCP tool calls are not recorded by any adapter yet.

## Common rules for every adapter

- Every `install` command only prints a snippet. Footprints never edits agent settings itself.
- `footprints` must be on the `PATH` the agent sees. A full path to the binary works too.
- The hooks always exit with status 0, so a problem in Footprints never blocks the agent's tool
  call. Errors go to standard error.
- The agent and the terminal that reads the log must see the same `FOOTPRINTS_HOME`.
- `--agent <name>` on a hook command changes the agent name stored in the ledger.
