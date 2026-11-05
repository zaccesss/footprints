//! OpenTelemetry export in OTLP JSON form, using the GenAI semantic conventions for tool
//! execution (`gen_ai.operation.name = execute_tool`), so tools such as Jaeger and Langfuse can
//! read a ledger without a custom parser.

use crate::store::{Action, Kind};
use serde_json::{Value, json};

fn hex_id(seed: &str, bytes: usize) -> String {
    let h = blake3::hash(seed.as_bytes()).to_hex().to_string();
    h[..bytes * 2].to_string()
}

fn attr(key: &str, v: Value) -> Value {
    let value = match v {
        Value::Bool(b) => json!({ "boolValue": b }),
        // OTLP JSON carries 64-bit integers as strings to survive JavaScript parsers
        Value::Number(n) if n.is_i64() => json!({ "intValue": n.to_string() }),
        Value::String(s) => json!({ "stringValue": s }),
        other => json!({ "stringValue": other.to_string() }),
    };
    json!({ "key": key, "value": value })
}

fn span(a: &Action) -> Value {
    let tool = a.tool.clone().unwrap_or_else(|| a.kind.to_string());
    let start = a.time_ms as i128 * 1_000_000;
    let end = a.end_time_ms.unwrap_or(a.time_ms) as i128 * 1_000_000;
    let mut attrs = vec![
        attr("gen_ai.operation.name", json!("execute_tool")),
        attr("gen_ai.tool.name", json!(tool)),
        attr("gen_ai.agent.name", json!(a.agent)),
        attr("gen_ai.conversation.id", json!(a.session)),
        attr("footprints.action.id", json!(a.id)),
        attr("footprints.action.kind", json!(a.kind.as_str())),
        attr("footprints.action.status", json!(a.status.as_str())),
        attr("footprints.reversible", json!(a.reversible)),
    ];
    match a.kind {
        Kind::Command => attrs.push(attr("process.command_line", json!(a.target))),
        Kind::Network => attrs.push(attr("url.full", json!(a.target))),
        _ => attrs.push(attr("file.path", json!(a.target))),
    }
    if let Some(id) = &a.tool_use_id {
        attrs.push(attr("gen_ai.tool.call.id", json!(id)));
    }
    if let Some(i) = &a.intent {
        attrs.push(attr("footprints.intent", json!(i)));
    }
    if let Some(h) = &a.pre_hash {
        attrs.push(attr("footprints.pre_image", json!(h)));
    }
    if let Some(h) = &a.post_hash {
        attrs.push(attr("footprints.post_image", json!(h)));
    }
    let mut s = json!({
        "traceId": hex_id(&format!("session:{}", a.session), 16),
        "spanId": hex_id(&format!("action:{}", a.id), 8),
        "name": format!("execute_tool {tool}"),
        "kind": 1,
        "startTimeUnixNano": start.to_string(),
        "endTimeUnixNano": end.to_string(),
        "attributes": attrs,
    });
    if let Some(p) = a.parent_id {
        s["parentSpanId"] = json!(hex_id(&format!("action:{p}"), 8));
    }
    s
}

pub fn otel(actions: &[Action]) -> Value {
    json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [
                    attr("service.name", json!("footprints")),
                    attr("service.version", json!(env!("CARGO_PKG_VERSION"))),
                ]
            },
            "scopeSpans": [{
                "scope": { "name": "footprints", "version": env!("CARGO_PKG_VERSION") },
                "spans": actions.iter().map(span).collect::<Vec<_>>(),
            }]
        }]
    })
}
