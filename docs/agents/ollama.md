# Local models with Ollama

[Ollama](https://ollama.com/) runs models locally. It is not an agent itself: an agent sends
prompts to it and makes the file changes. Footprints records the agent, so the route depends on
which agent drives the model.

| Agent driving the model | Route | Page |
| --- | --- | --- |
| Aider (`aider --model ollama_chat/<model>`) | The `run` wrapper | [Aider](aider.md) |
| OpenCode with an Ollama provider | The `run` wrapper, plus MCP | [OpenCode](opencode.md) |
| Goose with the Ollama provider | The `run` wrapper or the daemon, plus MCP | [Goose](goose.md) |
| Cline with the Ollama provider | The daemon or the wrapper, plus MCP | [Cline](cline.md) |
| Continue with an Ollama model | The daemon or the wrapper, plus MCP | [Continue](continue.md) |
| Zed with an Ollama model | The daemon, plus MCP | [Zed](zed.md) |
| GitHub Copilot CLI with a local provider | The `run` wrapper, plus MCP | [GitHub Copilot](copilot.md) |
| A home-made loop or script | The `run` wrapper | Below |

Each agent's own documentation explains how to point it at Ollama. The links are on each page.

## A home-made agent loop

A script that sends prompts to a local model and writes files with the answers is recorded like
any other command:

```sh
footprints run --agent ollama-loop --dir ~/projects/site -- python agent.py
```

- `--dir` is both the directory the script runs in and the one that is watched.
- `--intent "rename the config keys"` stores what the run was for.
- Several runs can share one `--session`, so `footprints undo --session <name>` takes them all
  back together.
- The script can read `FOOTPRINTS_SESSION` from its environment, for example to log it.

The same works for llama.cpp, LM Studio or any other local runtime, since the wrapper only
watches the files.

## Giving a local model the MCP tools

If the agent supports MCP servers, the [MCP server](../mcp.md) lets it list and undo its own
changes. Smaller local models can struggle to call tools reliably. Some agents also need tool use
turned on for the model: Zed, for example, needs `"supports_tools": true` on it.

## Check it works

```sh
footprints log --agent ollama-loop
```

## Links

- [Ollama integrations](https://docs.ollama.com/integrations)
- [Aider with Ollama](https://aider.chat/docs/llms/ollama.html)
- [Goose providers](https://goose-docs.ai/docs/getting-started/providers)
- [OpenCode providers](https://opencode.ai/docs/providers/)
- [Continue with Ollama](https://docs.continue.dev/customize/model-providers/top-level/ollama)
- [Zed with a local model](https://zed.dev/docs/ai/use-a-local-model)
