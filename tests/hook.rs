mod common;

use common::{Env, id};
use serde_json::json;
use std::fs;

fn payload(env: &Env, event: &str, tool: &str, input: serde_json::Value, tu: &str) -> String {
    json!({
        "session_id": "sess-hook",
        "transcript_path": "/tmp/transcript.jsonl",
        "cwd": env.dir(),
        "hook_event_name": event,
        "tool_name": tool,
        "tool_input": input,
        "tool_use_id": tu,
    })
    .to_string()
}

fn hook(env: &Env, phase: &str, body: &str) {
    let out = env.with_stdin(&["hook", phase], body);
    assert!(out.status.success(), "hooks must never fail the tool call");
    assert!(out.stdout.is_empty(), "hooks print nothing on stdout");
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn write_tool_creating_a_file_is_recorded_and_undone() {
    let env = Env::new();
    let path = env.path("new.md");
    let input = json!({ "file_path": path, "content": "# hello\n" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Write", input.clone(), "tu_1"),
    );
    let pending = env.find("file_create", &path);
    assert_eq!(pending["status"], "pending");

    fs::write(&path, "# hello\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Write", input, "tu_1"),
    );

    let a = env.find("file_create", &path);
    assert_eq!(a["status"], "recorded");
    assert_eq!(a["agent"], "claude-code");
    assert_eq!(a["session"], "sess-hook");
    assert_eq!(a["reversible"], true);
    env.ok(&["undo", &id(&a)]);
    assert!(!path.exists());
}

#[test]
fn edit_tool_with_a_relative_path_restores_the_pre_image() {
    let env = Env::new();
    let path = env.path("src.rs");
    fs::write(&path, "fn main() {}\n").unwrap();
    let input = json!({ "file_path": "src.rs", "old_string": "{}", "new_string": "{ todo!() }" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Edit", input.clone(), "tu_2"),
    );
    fs::write(&path, "fn main() { todo!() }\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Edit", input, "tu_2"),
    );

    let a = env.find("file_write", &path);
    let diff = env.ok(&["diff", &id(&a)]);
    assert!(diff.contains("-fn main() {}"), "{diff}");
    assert!(diff.contains("+fn main() { todo!() }"), "{diff}");
    env.ok(&["undo", &id(&a)]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "fn main() {}\n");
}

#[test]
fn a_tool_call_that_changes_nothing_leaves_no_entry() {
    let env = Env::new();
    let path = env.path("same.txt");
    fs::write(&path, "same\n").unwrap();
    let input = json!({ "file_path": path, "old_string": "x", "new_string": "y" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Edit", input.clone(), "tu_3"),
    );
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Edit", input, "tu_3"),
    );
    assert!(env.log().is_empty());
}

#[test]
fn bash_records_the_command_and_each_file_it_changed() {
    let env = Env::new();
    fs::write(env.path("config.toml"), "debug = false\n").unwrap();
    let input = json!({ "command": "sed -i '' s/false/true/ config.toml && touch made.txt", "description": "turn on debug" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Bash", input.clone(), "tu_4"),
    );
    // the test plays the part of the shell command
    fs::write(env.path("config.toml"), "debug = true\n").unwrap();
    fs::write(env.path("made.txt"), "").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Bash", input, "tu_4"),
    );

    let log = env.log();
    let cmd = &log[0];
    assert_eq!(cmd["kind"], "command");
    assert_eq!(cmd["intent"], "turn on debug");
    assert_eq!(cmd["reversible"], false);
    let edit = env.find("file_write", &env.path("config.toml"));
    let made = env.find("file_create", &env.path("made.txt"));
    assert_eq!(edit["parent_id"], cmd["id"]);
    assert_eq!(made["parent_id"], cmd["id"]);

    let out = env.ok(&["undo", "--session", "sess-hook"]);
    assert!(out.contains("2 action(s) undone"), "{out}");
    assert_eq!(
        fs::read_to_string(env.path("config.toml")).unwrap(),
        "debug = false\n"
    );
    assert!(!env.path("made.txt").exists());
}

#[test]
fn web_fetch_is_recorded_as_a_network_call_that_cannot_be_undone() {
    let env = Env::new();
    let input = json!({ "url": "https://example.com/", "prompt": "summarise" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "WebFetch", input.clone(), "tu_5"),
    );
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "WebFetch", input, "tu_5"),
    );
    let log = env.log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["kind"], "network");
    assert_eq!(log[0]["reversible"], false);
    assert_eq!(log[0]["target"], "https://example.com/");
    let out = env.run(&["undo", &id(&log[0])]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("network call"));
}

#[test]
fn read_only_tools_are_ignored() {
    let env = Env::new();
    let input = json!({ "file_path": env.path("x") });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Read", input, "tu_6"),
    );
    assert!(env.log().is_empty());
}

#[test]
fn bad_input_never_blocks_the_agent() {
    let env = Env::new();
    let out = env.with_stdin(&["hook", "pre"], "this is not json");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("hook error"));
}

#[test]
fn install_prints_valid_settings_and_writes_nothing() {
    let env = Env::new();
    let out = env.ok(&["install", "claude-code", "--print"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "footprints hook pre"
    );
    let refused = env.run(&["install", "claude-code"]);
    assert!(!refused.status.success());
}

#[test]
fn powershell_is_recorded_like_bash() {
    // Claude Code's native Windows shell tool carries the same `command` field as Bash
    let env = Env::new();
    fs::write(env.path("notes.txt"), "old\n").unwrap();
    let input =
        json!({ "command": "Set-Content notes.txt new", "description": "rewrite the notes" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "PowerShell", input.clone(), "tu_ps"),
    );
    fs::write(env.path("notes.txt"), "new\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "PowerShell", input, "tu_ps"),
    );

    let log = env.log();
    let cmd = &log[0];
    assert_eq!(cmd["kind"], "command");
    assert_eq!(cmd["intent"], "rewrite the notes");
    let edit = env.find("file_write", &env.path("notes.txt"));
    assert_eq!(edit["parent_id"], cmd["id"]);
}

#[test]
fn bash_snapshot_over_budget_is_reported_not_guessed() {
    let env = Env::new();
    fs::write(env.path("big.txt"), "x".repeat(4096)).unwrap();
    let input = json!({ "command": "echo y > big.txt" });
    let body = payload(&env, "PreToolUse", "Bash", input.clone(), "tu_7");
    let out = env
        .cmd()
        .args(["hook", "pre"])
        .env("FOOTPRINTS_SNAPSHOT_BUDGET_BYTES", "100")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(body.as_bytes())?;
            c.wait()
        })
        .unwrap();
    assert!(out.success());
    fs::write(env.path("big.txt"), "y\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Bash", input, "tu_7"),
    );
    let log = env.log();
    assert_eq!(log.len(), 1, "no file actions are guessed");
    assert!(
        log[0]["note"]
            .as_str()
            .unwrap()
            .contains("too large to snapshot")
    );
}

#[test]
fn a_post_hook_never_adopts_another_calls_pre_image() {
    let env = Env::new();
    let path = env.path("f.txt");
    fs::write(&path, "v0\n").unwrap();
    let input = json!({ "file_path": path, "content": "x" });
    // a call that was denied: pre only
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Write", input.clone(), "tu_denied"),
    );
    fs::write(&path, "v1 by the user\n").unwrap();
    // a later call whose pre hook never ran
    fs::write(&path, "v2 by the agent\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Write", input, "tu_later"),
    );

    let log = env.log();
    let later = log.iter().find(|a| a["tool_use_id"] == "tu_later").unwrap();
    assert_eq!(later["reversible"], false);
    let out = env.run(&["undo", &id(later)]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(fs::read_to_string(&path).unwrap(), "v2 by the agent\n");
}

#[test]
fn a_bad_hook_argument_does_not_block_the_tool() {
    let env = Env::new();
    let out = env.with_stdin(&["hook", "pre", "--no-such-flag"], "{}");
    assert_eq!(out.status.code(), Some(0));
}

// regression tests for the second review round

#[test]
fn a_failed_bash_call_still_records_its_file_changes() {
    let env = Env::new();
    fs::write(env.path("out.txt"), "old\n").unwrap();
    let input = json!({ "command": "echo new > out.txt && false" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Bash", input.clone(), "tu_f"),
    );
    fs::write(env.path("out.txt"), "new\n").unwrap();
    let mut body: serde_json::Value =
        serde_json::from_str(&payload(&env, "PostToolUseFailure", "Bash", input, "tu_f")).unwrap();
    body["error"] = json!("Exit code 1\nsomething broke");
    body["is_interrupt"] = json!(false);
    hook(&env, "post", &body.to_string());

    let cmd = &env.log()[0];
    assert_eq!(cmd["status"], "recorded");
    assert!(
        cmd["note"]
            .as_str()
            .unwrap()
            .contains("the tool call failed: Exit code 1")
    );
    let w = env.find("file_write", &env.path("out.txt"));
    env.ok(&["undo", &id(&w)]);
    assert_eq!(fs::read_to_string(env.path("out.txt")).unwrap(), "old\n");
}

#[test]
fn the_settings_snippet_registers_the_failure_hook() {
    let env = Env::new();
    let out = env.ok(&["install", "claude-code", "--print"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"],
        "footprints hook post"
    );
}

#[test]
fn bash_pre_image_is_not_fooled_by_a_same_size_edit_that_keeps_the_mtime() {
    let env = Env::new();
    let f = env.path("f");
    fs::write(&f, "version A\n").unwrap();
    // an old mtime, so the cache is allowed to hold this file
    let sh = |script: &str| {
        assert!(
            std::process::Command::new("sh")
                .current_dir(env.dir())
                .args(["-c", script])
                .status()
                .unwrap()
                .success()
        )
    };
    sh("touch -t 202001010000 f");
    let ls = json!({ "command": "ls" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Bash", ls.clone(), "tu_a"),
    );
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Bash", ls, "tu_a"),
    );
    // the user rewrites f with the same size and puts the mtime back
    sh("cp -p f ref && printf 'version B\\n' > f && touch -r ref f && rm ref");
    let cmd = json!({ "command": "echo version C > f" });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Bash", cmd.clone(), "tu_b"),
    );
    fs::write(&f, "version C\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Bash", cmd, "tu_b"),
    );

    let w = env.find("file_write", &f);
    env.ok(&["undo", &id(&w)]);
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "version B\n",
        "the user's edit is kept"
    );
}

#[test]
fn relative_paths_without_a_cwd_are_stored_absolute_and_normalised() {
    let env = Env::new();
    fs::create_dir(env.path("sub")).unwrap();
    let body = json!({
        "session_id": "s",
        "tool_name": "Write",
        "tool_input": { "file_path": "sub/../rel.txt" },
        "tool_use_id": "tu_r"
    })
    .to_string();
    hook(&env, "pre", &body);
    fs::write(env.path("rel.txt"), "x\n").unwrap();
    hook(&env, "post", &body);
    let a = env.find("file_create", &env.path("rel.txt"));
    assert_eq!(a["status"], "recorded");
    // undo from another directory still finds the file
    let out = env
        .cmd()
        .args(["undo", &id(&a)])
        .current_dir(env.home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!env.path("rel.txt").exists());
}

#[test]
fn a_post_hook_with_a_matching_id_but_another_file_is_not_paired() {
    let env = Env::new();
    fs::write(env.path("p1"), "one\n").unwrap();
    fs::write(env.path("p2"), "two\n").unwrap();
    let i1 = json!({ "file_path": env.path("p1") });
    let i2 = json!({ "file_path": env.path("p2") });
    hook(
        &env,
        "pre",
        &payload(&env, "PreToolUse", "Edit", i1, "tu_same"),
    );
    fs::write(env.path("p2"), "two edited\n").unwrap();
    hook(
        &env,
        "post",
        &payload(&env, "PostToolUse", "Edit", i2, "tu_same"),
    );
    let p2 = env.find("file_write", &env.path("p2"));
    assert_eq!(p2["reversible"], false, "no pre-image was taken for p2");
    let p1 = env.find("file_write", &env.path("p1"));
    assert_eq!(p1["status"], "pending");
}
