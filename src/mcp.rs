//! a minimal MCP server over stdio, so an agent can read its own ledger and undo its own steps.
//!
//! messages are newline-delimited JSON-RPC 2.0, as the MCP stdio transport specifies. It is
//! written by hand because it needs three tools and no resources, prompts or sampling, which
//! does not justify an SDK and its async runtime.

use crate::diff;
use crate::store::{Filter, Store};
use crate::undo::{self, Refused};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub fn serve(store: &Store, input: impl BufRead, mut out: impl Write) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(store, &msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            serde_json::to_writer(&mut out, &r)?;
            out.write_all(b"\n")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn handle(store: &Store, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str);
    // a message without an id is a notification and must never get a reply
    let id = msg.get("id").cloned()?;
    let Some(method) = method else {
        return Some(error(id, -32600, "invalid request: missing method"));
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, initialize(&params)),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => match call(store, &params) {
            Ok(result) => ok(id, result),
            Err(CallError::BadParams(m)) => error(id, -32602, &m),
            Err(CallError::Internal(e)) => error(id, -32603, &format!("internal error: {e:#}")),
        },
        other => error(id, -32601, &format!("method not found: {other}")),
    })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|v| SUPPORTED_VERSIONS.contains(v))
        .unwrap_or(SUPPORTED_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "footprints",
            "title": "Footprints",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Footprints records file writes, shell commands and network calls made by agents on this machine. Use footprints_log to see recent actions, footprints_diff to inspect a file change and footprints_undo to reverse one. Undo refuses when the file changed since it was recorded.",
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "footprints_log",
            "title": "List recorded actions",
            "description": "List recent actions recorded in the Footprints ledger, oldest first. Each entry has an id, time, agent, session, kind (file_write, file_create, file_delete, command or network), target and whether it can be undone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": { "type": "string", "description": "Only actions from this session." },
                    "agent": { "type": "string", "description": "Only actions from this agent." },
                    "limit": { "type": "integer", "minimum": 1, "description": "Most recent N actions, default 50." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "footprints_diff",
            "title": "Show a file change",
            "description": "Show the unified diff of one recorded file action, from its pre-image to its post-image.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The action id from footprints_log." } },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "footprints_undo",
            "title": "Undo a recorded action",
            "description": "Undo one file action by id or every file action in a session in reverse order. Refuses and reports a conflict when the file changed after it was recorded, so newer work is never overwritten. There is no way to force an undo from here: that is only possible from the command line. Commands and network calls cannot be undone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The action to undo." },
                    "session": { "type": "string", "description": "Undo every file action in this session instead." }
                },
                "additionalProperties": false
            },
            "annotations": { "destructiveHint": true, "readOnlyHint": false }
        }
    ])
}

fn text(s: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": s }], "isError": is_error })
}

/// a protocol error: bad arguments or an internal fault. A tool that itself failed is not an
/// error here but an Ok result with its `isError` flag set.
enum CallError {
    BadParams(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for CallError {
    fn from(e: anyhow::Error) -> Self {
        CallError::Internal(e)
    }
}

impl From<serde_json::Error> for CallError {
    fn from(e: serde_json::Error) -> Self {
        CallError::Internal(e.into())
    }
}

fn bad(m: impl Into<String>) -> CallError {
    CallError::BadParams(m.into())
}

fn call(store: &Store, params: &Value) -> std::result::Result<Value, CallError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("tools/call needs a tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    if !args.is_object() {
        return Err(bad("arguments must be an object"));
    }
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let id = args.get("id").and_then(Value::as_i64);
    match name {
        "footprints_log" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let actions = store.list(&Filter {
                session: s("session"),
                agent: s("agent"),
                kind: None,
                limit: Some(limit.max(1)),
            })?;
            Ok(text(serde_json::to_string_pretty(&actions)?, false))
        }
        "footprints_diff" => {
            let id = id.ok_or_else(|| bad("footprints_diff needs an integer id"))?;
            match store.get(id)? {
                Some(a) => match diff::diff(store, &a) {
                    Ok(d) => Ok(text(d, false)),
                    Err(e) => Ok(text(format!("diff failed: {e:#}"), true)),
                },
                None => Ok(text(format!("no action with id {id}"), true)),
            }
        }
        "footprints_undo" => {
            if args.get("force").is_some() {
                return Err(bad(
                    "footprints_undo has no force option: an agent can never overwrite newer work. \
                     Forcing is only available from the command line",
                ));
            }
            let opts = undo::UndoOpts::agent();
            let result = match (id, s("session")) {
                (Some(id), None) => undo::undo_one(store, id, &opts).map(|u| u.to_string()),
                (None, Some(sess)) => undo::undo_session(store, &sess, &opts).map(|r| {
                    let mut lines: Vec<String> = r.done.iter().map(|u| u.to_string()).collect();
                    lines.extend(r.skipped.iter().map(|(_, why)| format!("skipped: {why}")));
                    lines.join("\n")
                }),
                _ => return Err(bad("footprints_undo needs either id or session, not both")),
            };
            Ok(match result {
                Ok(msg) => text(msg, false),
                Err(e) if e.is::<Refused>() => text(e.to_string(), true),
                Err(e) => text(format!("undo failed: {e:#}"), true),
            })
        }
        other => Err(bad(format!("unknown tool {other}"))),
    }
}
