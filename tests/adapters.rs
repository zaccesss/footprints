//! sample payloads for the Cursor, Codex CLI and Gemini CLI hook adapters, shaped after each
//! agent's own hook documentation.

mod common;

use common::{Env, id};
use serde_json::{Value, json};
use std::fs;

/// runs one hook and returns what it printed, checking it never fails the agent's call.
fn hook(env: &Env, from: &str, phase: &str, body: &Value) -> String {
    let out = env.with_stdin(&["hook", phase, "--from", from], &body.to_string());
    assert!(out.status.success(), "hooks must never fail the tool call");
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

// Cursor

fn cursor(env: &Env, event: &str, extra: Value) -> Value {
    let mut v = json!({
        "conversation_id": "conv-1",
        "generation_id": "gen-1",
        "hook_event_name": event,
        "workspace_roots": [env.dir()],
        "cursor_version": "2.0.0",
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

#[test]
fn cursor_shell_hooks_record_the_command_and_its_file_changes() {
    let env = Env::new();
    fs::write(env.path("app.cfg"), "port = 80\n").unwrap();
    let before = cursor(
        &env,
        "beforeShellExecution",
        json!({ "command": "sed -i s/80/8080/ app.cfg", "cwd": env.dir(), "sandbox": false }),
    );
    let reply = hook(&env, "cursor", "pre", &before);
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(
        reply["permission"], "allow",
        "Cursor needs an explicit allow"
    );

    fs::write(env.path("app.cfg"), "port = 8080\n").unwrap();
    // afterShellExecution has no cwd, so pairing relies on the conversation and command
    let after = cursor(
        &env,
        "afterShellExecution",
        json!({ "command": "sed -i s/80/8080/ app.cfg", "output": "", "duration": 12 }),
    );
    assert_eq!(hook(&env, "cursor", "post", &after), "");

    let log = env.log();
    assert_eq!(log[0]["kind"], "command");
    assert_eq!(log[0]["agent"], "cursor");
    assert_eq!(log[0]["session"], "conv-1");
    assert_eq!(log[0]["status"], "recorded");
    let edit = env.find("file_write", &env.path("app.cfg"));
    assert_eq!(edit["parent_id"], log[0]["id"]);
    env.ok(&["undo", &id(&edit)]);
    assert_eq!(
        fs::read_to_string(env.path("app.cfg")).unwrap(),
        "port = 80\n"
    );
}

#[test]
fn cursor_file_edit_rebuilds_the_pre_image_when_it_is_exact() {
    let env = Env::new();
    let f = env.path("main.rs");
    fs::write(&f, "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    // Cursor reports the edit only after it is on disk
    fs::write(&f, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
    let ev = cursor(
        &env,
        "afterFileEdit",
        json!({
            "file_path": f,
            "edits": [{ "old_string": "println!(\"hi\");", "new_string": "println!(\"hello\");" }]
        }),
    );
    hook(&env, "cursor", "post", &ev);
    let a = env.find("file_write", &f);
    assert_eq!(a["reversible"], true, "{}", a["note"]);
    let diff = env.ok(&["diff", &id(&a)]);
    assert!(diff.contains("-    println!(\"hi\");"), "{diff}");
    env.ok(&["undo", &id(&a)]);
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "fn main() {\n    println!(\"hi\");\n}\n"
    );
}

#[test]
fn cursor_file_edit_that_cannot_be_reversed_exactly_is_not_reversible() {
    let env = Env::new();
    let f = env.path("dup.txt");
    // the new text appears twice, so which copy was edited is unknown
    fs::write(&f, "b\nb\n").unwrap();
    let ev = cursor(
        &env,
        "afterFileEdit",
        json!({ "file_path": f, "edits": [{ "old_string": "a", "new_string": "b" }] }),
    );
    hook(&env, "cursor", "post", &ev);
    let a = env.find("file_write", &f);
    assert_eq!(a["reversible"], false);
    assert!(a["note"].as_str().unwrap().contains("no pre-image"));
}

#[test]
fn cursor_pre_hook_still_allows_after_bad_input() {
    let env = Env::new();
    let out = env.with_stdin(&["hook", "pre", "--from", "cursor"], "not json");
    assert!(out.status.success());
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["permission"], "allow");
}

// Codex CLI

fn codex(env: &Env, event: &str, tool: &str, input: Value, tu: &str) -> Value {
    json!({
        "session_id": "thr_1",
        "turn_id": "turn_1",
        "transcript_path": null,
        "cwd": env.dir(),
        "hook_event_name": event,
        "model": "gpt-5-codex",
        "permission_mode": "default",
        "tool_name": tool,
        "tool_input": input,
        "tool_use_id": tu,
    })
}

#[test]
fn codex_apply_patch_records_every_file_in_the_patch() {
    let env = Env::new();
    fs::write(env.path("keep.txt"), "old line\n").unwrap();
    fs::write(env.path("gone.txt"), "bye\n").unwrap();
    fs::write(env.path("from.txt"), "moving\n").unwrap();
    let patch = "*** Begin Patch\n\
                 *** Update File: keep.txt\n@@\n-old line\n+new line\n\
                 *** Add File: src/added.txt\n+fresh\n\
                 *** Delete File: gone.txt\n\
                 *** Update File: from.txt\n*** Move to: to.txt\n@@\n moving\n\
                 *** End Patch\n";
    let input = json!({ "command": patch });
    assert_eq!(
        hook(
            &env,
            "codex",
            "pre",
            &codex(&env, "PreToolUse", "apply_patch", input.clone(), "call_1")
        ),
        ""
    );
    // the test plays the part of apply_patch
    fs::write(env.path("keep.txt"), "new line\n").unwrap();
    fs::create_dir_all(env.path("src")).unwrap();
    fs::write(env.path("src/added.txt"), "fresh\n").unwrap();
    fs::remove_file(env.path("gone.txt")).unwrap();
    fs::rename(env.path("from.txt"), env.path("to.txt")).unwrap();
    hook(
        &env,
        "codex",
        "post",
        &codex(&env, "PostToolUse", "apply_patch", input, "call_1"),
    );

    let log = env.log();
    assert!(log.iter().all(|a| a["agent"] == "codex"));
    assert!(log.iter().all(|a| a["status"] == "recorded"), "{log:?}");
    env.find("file_write", &env.path("keep.txt"));
    env.find("file_create", &env.path("src/added.txt"));
    env.find("file_delete", &env.path("gone.txt"));
    env.find("file_delete", &env.path("from.txt"));
    env.find("file_create", &env.path("to.txt"));

    let out = env.ok(&["undo", "--session", "thr_1"]);
    assert!(out.contains("5 action(s) undone"), "{out}");
    assert_eq!(
        fs::read_to_string(env.path("keep.txt")).unwrap(),
        "old line\n"
    );
    assert_eq!(fs::read_to_string(env.path("gone.txt")).unwrap(), "bye\n");
    assert_eq!(
        fs::read_to_string(env.path("from.txt")).unwrap(),
        "moving\n"
    );
    assert!(!env.path("to.txt").exists());
    assert!(!env.path("src/added.txt").exists());
}

#[test]
fn codex_bash_records_file_changes_like_claude_code() {
    let env = Env::new();
    let input = json!({ "command": "echo hi > out.txt" });
    hook(
        &env,
        "codex",
        "pre",
        &codex(&env, "PreToolUse", "Bash", input.clone(), "call_2"),
    );
    fs::write(env.path("out.txt"), "hi\n").unwrap();
    hook(
        &env,
        "codex",
        "post",
        &codex(&env, "PostToolUse", "Bash", input, "call_2"),
    );
    let made = env.find("file_create", &env.path("out.txt"));
    assert_eq!(made["agent"], "codex");
    assert_eq!(made["reversible"], true);
}

#[test]
fn codex_mcp_tools_are_ignored() {
    let env = Env::new();
    hook(
        &env,
        "codex",
        "pre",
        &codex(
            &env,
            "PreToolUse",
            "mcp__github__create_issue",
            json!({ "title": "x" }),
            "call_3",
        ),
    );
    assert!(env.log().is_empty());
}

// Gemini CLI

fn gemini(env: &Env, event: &str, tool: &str, input: Value, response: Option<Value>) -> Value {
    let mut v = json!({
        "session_id": "gem-1",
        "transcript_path": "/tmp/gem.json",
        "cwd": env.dir(),
        "hook_event_name": event,
        "timestamp": "2026-11-05T10:00:00Z",
        "tool_name": tool,
        "tool_input": input,
    });
    if let Some(r) = response {
        v["tool_response"] = r;
    }
    v
}

#[test]
fn gemini_write_file_and_replace_are_recorded_and_undone() {
    let env = Env::new();
    let f = env.path("notes.md");
    let input = json!({ "file_path": f, "content": "# notes\n" });
    let reply = hook(
        &env,
        "gemini",
        "pre",
        &gemini(&env, "BeforeTool", "write_file", input.clone(), None),
    );
    assert_eq!(reply.trim(), "{}", "Gemini CLI parses stdout as JSON");
    fs::write(&f, "# notes\n").unwrap();
    hook(
        &env,
        "gemini",
        "post",
        &gemini(
            &env,
            "AfterTool",
            "write_file",
            input,
            Some(json!({ "llmContent": "ok", "returnDisplay": "ok" })),
        ),
    );
    let created = env.find("file_create", &f);
    assert_eq!(created["agent"], "gemini-cli");
    assert_eq!(created["status"], "recorded");

    let input = json!({ "file_path": f, "old_string": "notes", "new_string": "plans", "instruction": "rename" });
    hook(
        &env,
        "gemini",
        "pre",
        &gemini(&env, "BeforeTool", "replace", input.clone(), None),
    );
    fs::write(&f, "# plans\n").unwrap();
    hook(
        &env,
        "gemini",
        "post",
        &gemini(&env, "AfterTool", "replace", input, Some(json!({}))),
    );
    let edit = env.find("file_write", &f);
    env.ok(&["undo", &id(&edit)]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "# notes\n");
}

#[test]
fn gemini_shell_uses_dir_path_and_a_failure_is_noted() {
    let env = Env::new();
    fs::create_dir(env.path("pkg")).unwrap();
    let input = json!({ "command": "make", "description": "build it", "dir_path": "pkg" });
    hook(
        &env,
        "gemini",
        "pre",
        &gemini(&env, "BeforeTool", "run_shell_command", input.clone(), None),
    );
    fs::write(env.path("pkg/out.o"), "obj").unwrap();
    hook(
        &env,
        "gemini",
        "post",
        &gemini(
            &env,
            "AfterTool",
            "run_shell_command",
            input,
            Some(json!({ "error": { "message": "make: *** [all] Error 2" } })),
        ),
    );
    let cmd = &env.log()[0];
    assert_eq!(cmd["intent"], "build it");
    let note = cmd["note"].as_str().unwrap();
    assert!(note.contains("Error 2"), "{note}");
    env.find("file_create", &env.path("pkg/out.o"));
}

#[test]
fn gemini_web_tools_are_network_calls() {
    let env = Env::new();
    for (tool, input) in [
        (
            "web_fetch",
            json!({ "prompt": "summarise https://example.com/" }),
        ),
        ("google_web_search", json!({ "query": "rust fanotify" })),
    ] {
        hook(
            &env,
            "gemini",
            "pre",
            &gemini(&env, "BeforeTool", tool, input.clone(), None),
        );
        hook(
            &env,
            "gemini",
            "post",
            &gemini(&env, "AfterTool", tool, input, Some(json!({}))),
        );
    }
    let log = env.log();
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|a| a["kind"] == "network"));
    assert_eq!(log[0]["target"], "fetch: summarise https://example.com/");
    assert_eq!(log[1]["target"], "search: rust fanotify");
}

// install snippets

#[test]
fn every_install_snippet_is_valid_json_and_only_prints() {
    let env = Env::new();
    for (agent, check) in [
        ("cursor", "/hooks/beforeShellExecution/0/command"),
        ("codex", "/hooks/PreToolUse/0/hooks/0/command"),
        ("gemini", "/hooks/BeforeTool/0/hooks/0/command"),
        ("claude-code", "/hooks/PreToolUse/0/hooks/0/command"),
    ] {
        let out = env.ok(&["install", agent, "--print"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        let cmd = v.pointer(check).and_then(Value::as_str).unwrap();
        assert!(cmd.starts_with("footprints hook pre"), "{agent}: {cmd}");
        let refused = env.run(&["install", agent]);
        assert!(
            !refused.status.success(),
            "{agent} without --print must refuse"
        );
    }
    assert_eq!(
        fs::read_dir(env.dir()).unwrap().count(),
        0,
        "nothing is written"
    );
}

// regression tests for the second review

#[test]
fn cursor_write_and_delete_pair_by_call_id_with_a_real_pre_image() {
    let env = Env::new();
    let f = env.path("doc.md");
    fs::write(&f, "first\n").unwrap();
    let tool = |event: &str, name: &str, tu: &str| {
        cursor(
            &env,
            event,
            json!({
                "tool_name": name,
                "tool_input": { "file_path": f, "content": "second\n" },
                "tool_use_id": tu,
                "cwd": env.dir(),
            }),
        )
    };
    let reply = hook(&env, "cursor", "pre", &tool("preToolUse", "Write", "tu_w"));
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap()["permission"],
        "allow"
    );
    fs::write(&f, "second\n").unwrap();
    assert_eq!(
        hook(
            &env,
            "cursor",
            "post",
            &tool("postToolUse", "Write", "tu_w")
        ),
        ""
    );
    // Cursor also reports the same edit through afterFileEdit, which must not add a row
    let after = cursor(
        &env,
        "afterFileEdit",
        json!({ "file_path": f, "edits": [{ "old_string": "first", "new_string": "second" }] }),
    );
    hook(&env, "cursor", "post", &after);
    let writes: Vec<Value> = env
        .log()
        .into_iter()
        .filter(|a| a["target"] == *f.to_string_lossy())
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0]["tool_use_id"], "tu_w");
    assert_eq!(writes[0]["reversible"], true);

    hook(&env, "cursor", "pre", &tool("preToolUse", "Delete", "tu_d"));
    fs::remove_file(&f).unwrap();
    hook(
        &env,
        "cursor",
        "post",
        &tool("postToolUse", "Delete", "tu_d"),
    );
    let gone = env.find("file_delete", &f);
    env.ok(&["undo", &id(&gone)]);
    assert_eq!(fs::read_to_string(&f).unwrap(), "second\n");
}

#[test]
fn gemini_replace_never_adopts_a_pre_image_from_a_denied_call() {
    let env = Env::new();
    let f = env.path("cfg.txt");
    fs::write(&f, "mode = a\n").unwrap();
    let input = json!({ "file_path": f, "old_string": "mode = b", "new_string": "mode = c" });
    // a BeforeTool whose call was denied leaves a pending row holding "mode = a"
    hook(
        &env,
        "gemini",
        "pre",
        &gemini(&env, "BeforeTool", "replace", input.clone(), None),
    );
    // the person edits the file, then a later call's BeforeTool never runs
    fs::write(&f, "mode = b\n").unwrap();
    fs::write(&f, "mode = c\n").unwrap();
    hook(
        &env,
        "gemini",
        "post",
        &gemini(&env, "AfterTool", "replace", input, Some(json!({}))),
    );
    let a = env.find("file_write", &f);
    assert_eq!(a["reversible"], false, "{a}");
    let out = env.run(&["undo", &id(&a)]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(fs::read_to_string(&f).unwrap(), "mode = c\n");
}

#[test]
fn codex_patch_under_a_matcher_alias_is_still_read_as_a_patch() {
    let env = Env::new();
    fs::write(env.path("a.txt"), "x\n").unwrap();
    let patch = "  *** Begin Patch\n  *** Update File: a.txt\n@@\n-x\n+y\n  *** End Patch\n";
    let input = json!({ "command": patch });
    hook(
        &env,
        "codex",
        "pre",
        &codex(&env, "PreToolUse", "Edit", input.clone(), "call_alias"),
    );
    fs::write(env.path("a.txt"), "y\n").unwrap();
    hook(
        &env,
        "codex",
        "post",
        &codex(&env, "PostToolUse", "Edit", input, "call_alias"),
    );
    let a = env.find("file_write", &env.path("a.txt"));
    env.ok(&["undo", &id(&a)]);
    assert_eq!(fs::read_to_string(env.path("a.txt")).unwrap(), "x\n");
}

#[cfg(windows)]
#[test]
fn windows_spellings_of_one_file_pair_into_one_entry() {
    let env = Env::new();
    let f = env.path("Notes.txt");
    fs::write(&f, "v1\n").unwrap();
    let upper = f.to_string_lossy().to_uppercase();
    let slashed = f.to_string_lossy().replace('\\', "/") + ".";
    hook(
        &env,
        "gemini",
        "pre",
        &gemini(
            &env,
            "BeforeTool",
            "write_file",
            json!({ "file_path": upper }),
            None,
        ),
    );
    fs::write(&f, "v2\n").unwrap();
    hook(
        &env,
        "gemini",
        "post",
        &gemini(
            &env,
            "AfterTool",
            "write_file",
            json!({ "file_path": slashed }),
            Some(json!({})),
        ),
    );
    let rows = env.log();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["status"], "recorded");
    assert_eq!(rows[0]["reversible"], true);
}
