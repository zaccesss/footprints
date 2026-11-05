# OpenTelemetry export

`footprints export --otel` writes the ledger as OTLP JSON spans that follow the
[OpenTelemetry GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/).
The output loads into any tool that reads OTLP JSON, such as [Jaeger](https://www.jaegertracing.io/)
or [Langfuse](https://langfuse.com/).

## Usage

```sh
footprints export --otel > spans.json
footprints export --otel --session run-bcc287bf -o run.json
footprints export --otel --agent claude-code -o claude.json
```

`--session` and `--agent` filter the same way as in `footprints log`. `-o` writes to a file
instead of standard output. The flags are listed in [the command reference](commands.md#export).

## Layout

- One trace per session and one span per action.
- File changes under a command are child spans of it.
- Each span is named `execute_tool <tool>`, with kind `1` (internal).
- The resource carries `service.name` set to `footprints` and `service.version` set to the
  Footprints version. The scope is named `footprints` with the same version.

## Attributes

| Attribute | Value |
| --- | --- |
| `gen_ai.operation.name` | Always `execute_tool` |
| `gen_ai.tool.name` | The tool, such as `Write`, `Bash`, `apply_patch` or `run` |
| `gen_ai.agent.name` | The agent name |
| `gen_ai.conversation.id` | The session |
| `gen_ai.tool.call.id` | The agent's call id, when it gave one |
| `footprints.action.id` | The action's id in the ledger |
| `footprints.action.kind` | `file_write`, `file_create`, `file_delete`, `command` or `network` |
| `footprints.action.status` | The recorded status |
| `footprints.reversible` | Whether the action can be undone |
| `footprints.intent` | The intent, when one was given |
| `footprints.pre_image` and `footprints.post_image` | The BLAKE3 hash of each image, when stored |
| `file.path` | The path, for a file action |
| `process.command_line` | The command, for a command action |
| `url.full` | The URL or query, for a network action |

A trimmed example of the start of the output:

```json
{
  "resourceSpans": [
    {
      "resource": {
        "attributes": [
          { "key": "service.name", "value": { "stringValue": "footprints" } },
          { "key": "service.version", "value": { "stringValue": "0.2.0" } }
        ]
      },
      "scopeSpans": [
        {
          "scope": { "name": "footprints", "version": "0.2.0" },
          "spans": [
            {
              "attributes": [
                { "key": "gen_ai.operation.name", "value": { "stringValue": "execute_tool" } }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

File contents are never exported, only their hashes. Command text and URLs are exported as
recorded, so check the output before sending it to a shared collector.
