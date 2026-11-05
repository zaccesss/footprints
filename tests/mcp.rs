mod common;

use common::Env;
use serde_json::{Value, json};
use std::fs;

fn session(env: &Env, messages: &[Value]) -> Vec<Value> {
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let out = env.with_stdin(&["mcp"], &input);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn init() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" }
        }
    })
}

#[test]
fn initialize_and_tools_list() {
    let env = Env::new();
    let replies = session(
        &env,
        &[
            init(),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ],
    );
    assert_eq!(replies.len(), 2, "notifications get no reply");

    let init = &replies[0];
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "footprints");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let tools = replies[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["footprints_log", "footprints_diff", "footprints_undo"]
    );
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn tools_call_log_diff_and_undo() {
    let env = Env::new();
    fs::write(env.path("a.txt"), "before\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo after > a.txt"]);
    let replies = session(
        &env,
        &[
            init(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": "footprints_log", "arguments": {} } }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "footprints_diff", "arguments": { "id": 2 } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": { "name": "footprints_undo", "arguments": { "id": 1 } } }),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                    "params": { "name": "footprints_undo", "arguments": { "id": 2 } } }),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "no/such/method" }),
        ],
    );
    let text = |r: &Value| {
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let log: Value = serde_json::from_str(&text(&replies[1])).unwrap();
    assert_eq!(log.as_array().unwrap().len(), 2);
    assert!(text(&replies[2]).contains("+after"));
    assert_eq!(
        replies[3]["result"]["isError"], true,
        "a command cannot be undone"
    );
    assert_eq!(replies[4]["result"]["isError"], false);
    assert_eq!(fs::read_to_string(env.path("a.txt")).unwrap(), "before\n");
    assert_eq!(replies[5]["error"]["code"], -32601);
}

#[test]
fn malformed_json_gets_a_parse_error() {
    let env = Env::new();
    let out = env.with_stdin(&["mcp"], "{not json\n");
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply["error"]["code"], -32700);
}

#[test]
fn undo_over_mcp_cannot_be_forced() {
    let env = Env::new();
    fs::write(env.path("a.txt"), "before\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo after > a.txt"]);
    fs::write(env.path("a.txt"), "newer work\n").unwrap();
    let replies = session(
        &env,
        &[
            init(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "footprints_undo", "arguments": { "id": 2, "force": true } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": { "name": "footprints_undo", "arguments": { "id": 2 } } }),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                    "params": { "name": "no_such_tool", "arguments": {} } }),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                    "params": { "name": "footprints_diff", "arguments": { "id": 999 } } }),
        ],
    );
    let undo_tool = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "footprints_undo")
        .unwrap()
        .clone();
    assert!(
        undo_tool["inputSchema"]["properties"]
            .get("force")
            .is_none()
    );
    assert_eq!(replies[2]["error"]["code"], -32602);
    assert_eq!(replies[3]["result"]["isError"], true);
    let msg = replies[3]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(msg.contains("conflict"), "{msg}");
    assert!(
        !msg.contains("--force"),
        "an agent is never pointed at force: {msg}"
    );
    assert_eq!(replies[4]["error"]["code"], -32602);
    assert_eq!(replies[5]["result"]["isError"], true);
    assert_eq!(
        fs::read_to_string(env.path("a.txt")).unwrap(),
        "newer work\n"
    );
}
