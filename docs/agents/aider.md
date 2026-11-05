# Aider

[Aider](https://aider.chat/) is a terminal coding agent. It has no hook system and no MCP
support in its documented options, so Footprints records it with [the wrapper](other-agents.md#the-wrapper).

| | |
| --- | --- |
| Route | The `run` wrapper |
| Native adapter | No |
| MCP | Not supported by Aider |
| Agent name in the log | Whatever `--agent` says, `aider` by default |
| Official docs | [Options reference](https://aider.chat/docs/config/options.html), [Ollama models](https://aider.chat/docs/llms/ollama.html) |

## What is captured

Every file Aider creates, edits or deletes under the directory it runs in, each with a pre-image
so it can be undone. That includes files changed by commands Aider runs, such as its
`--lint-cmd` or `--test-cmd`.

## What is not captured

- Which of Aider's edits or commands made each change. The whole session is one command with
  the file changes under it.
- Files outside the watched directory.
- The commits Aider makes. `.git` is skipped, so `git log` stays the record of those. An undo
  restores files but does not remove a commit.

## Setup

Run Aider in the project folder under the wrapper:

```sh
cd ~/projects/site
footprints run --agent aider -- aider
```

Aider's own arguments go after the `--`:

```sh
footprints run --agent aider --intent "fix the failing test" -- aider tests/test_api.py
```

An alias makes it the default:

```sh
alias aider='footprints run --agent aider -- aider'
```

`footprints run` starts the program itself rather than through the shell, so the alias does not
call itself again.

## Check it works

After Aider exits:

```sh
footprints log --agent aider
footprints diff <id>
```

## Remove it

Run `aider` without the `footprints run` prefix and remove the alias.

## Links

- [Aider options](https://aider.chat/docs/config/options.html)
- [Aider with Ollama](https://aider.chat/docs/llms/ollama.html)
- [Local models with Ollama](ollama.md)
