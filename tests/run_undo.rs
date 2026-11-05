mod common;

use common::{Env, id};
use std::fs;

#[test]
fn run_records_edit_create_and_delete_and_undo_restores_bytes() {
    let env = Env::new();
    let original = b"line one\nline two\nline three\n".to_vec();
    let binary: Vec<u8> = (0u8..=255).collect();
    fs::write(env.path("edit.txt"), &original).unwrap();
    fs::write(env.path("doomed.bin"), &binary).unwrap();
    fs::create_dir_all(env.path("skip/.git")).unwrap();
    fs::write(env.path("skip/.git/HEAD"), "ref").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(env.path("tool.sh"), "#!/bin/sh\necho hi\n").unwrap();
        fs::set_permissions(env.path("tool.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        env.path("agent.sh"),
        "printf 'line one\\nline 2\\nline three\\n' > edit.txt\n\
         mkdir -p sub && printf 'brand new\\n' > sub/created.txt\n\
         rm doomed.bin\n\
         rm -f tool.sh\n\
         echo changed > skip/.git/HEAD\n",
    )
    .unwrap();

    let out = env.run(&["run", "--agent", "test-agent", "--", "sh", "agent.sh"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!env.path("doomed.bin").exists());
    assert_eq!(
        fs::read(env.path("sub/created.txt")).unwrap(),
        b"brand new\n"
    );

    let log = env.log();
    assert_eq!(log[0]["kind"], "command");
    assert_eq!(log[0]["agent"], "test-agent");
    assert!(
        !log.iter()
            .any(|a| a["target"].as_str().unwrap().contains(".git")),
        ".git must be skipped"
    );

    let edit = env.find("file_write", &env.path("edit.txt"));
    let created = env.find("file_create", &env.path("sub/created.txt"));
    let deleted = env.find("file_delete", &env.path("doomed.bin"));
    for a in [&edit, &created, &deleted] {
        assert_eq!(a["reversible"], true);
        assert_eq!(a["parent_id"], log[0]["id"]);
    }

    let diff = env.ok(&["diff", &id(&edit)]);
    assert!(diff.contains("-line two"), "{diff}");
    assert!(diff.contains("+line 2"), "{diff}");

    env.ok(&["undo", &id(&edit)]);
    assert_eq!(fs::read(env.path("edit.txt")).unwrap(), original);
    env.ok(&["undo", &id(&created)]);
    assert!(!env.path("sub/created.txt").exists());
    env.ok(&["undo", &id(&deleted)]);
    assert_eq!(fs::read(env.path("doomed.bin")).unwrap(), binary);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tool = env.find("file_delete", &env.path("tool.sh"));
        env.ok(&["undo", &id(&tool)]);
        let mode = fs::metadata(env.path("tool.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "file mode is restored");
    }

    // a second undo of the same action is refused
    let again = env.run(&["undo", &id(&edit)]);
    assert_eq!(again.status.code(), Some(3));
}

#[test]
fn undo_refuses_after_outside_edit_and_force_keeps_the_overwritten_copy() {
    let env = Env::new();
    fs::write(env.path("notes.txt"), "v1\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo v2 > notes.txt"]);
    let edit = env.find("file_write", &env.path("notes.txt"));

    // someone edits the file after the agent did
    fs::write(env.path("notes.txt"), "human edit\n").unwrap();

    let out = env.run(&["undo", &id(&edit)]);
    assert_eq!(out.status.code(), Some(3), "a conflict exits with status 3");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("conflict"), "{err}");
    assert!(err.contains("--force"), "{err}");
    assert_eq!(
        fs::read_to_string(env.path("notes.txt")).unwrap(),
        "human edit\n"
    );

    let forced = env.ok(&["undo", &id(&edit), "--force"]);
    assert!(forced.contains("forced"), "{forced}");
    assert_eq!(fs::read_to_string(env.path("notes.txt")).unwrap(), "v1\n");

    // the forced undo is itself in the ledger, so the human edit can be brought back
    let undo_action = env
        .log()
        .into_iter()
        .find(|a| a["undo_of"] == edit["id"])
        .unwrap();
    env.ok(&["undo", &id(&undo_action)]);
    assert_eq!(
        fs::read_to_string(env.path("notes.txt")).unwrap(),
        "human edit\n"
    );
}

#[test]
fn session_undo_runs_in_reverse_order() {
    let env = Env::new();
    fs::write(env.path("f.txt"), "v1\n").unwrap();
    env.ok(&[
        "run",
        "--session",
        "s1",
        "--",
        "sh",
        "-c",
        "echo v2 > f.txt",
    ]);
    env.ok(&[
        "run",
        "--session",
        "s1",
        "--",
        "sh",
        "-c",
        "echo v3 > f.txt; echo x > g.txt",
    ]);
    assert_eq!(fs::read_to_string(env.path("f.txt")).unwrap(), "v3\n");

    let out = env.ok(&["undo", "--session", "s1"]);
    assert!(out.contains("3 action(s) undone"), "{out}");
    assert_eq!(fs::read_to_string(env.path("f.txt")).unwrap(), "v1\n");
    assert!(!env.path("g.txt").exists());
}

#[test]
fn session_undo_changes_nothing_when_any_step_conflicts() {
    let env = Env::new();
    fs::write(env.path("a.txt"), "a1\n").unwrap();
    fs::write(env.path("b.txt"), "b1\n").unwrap();
    env.ok(&[
        "run",
        "--session",
        "s2",
        "--",
        "sh",
        "-c",
        "echo a2 > a.txt; echo b2 > b.txt",
    ]);
    fs::write(env.path("b.txt"), "outside\n").unwrap();

    let out = env.run(&["undo", "--session", "s2"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(fs::read_to_string(env.path("a.txt")).unwrap(), "a2\n");
    assert_eq!(fs::read_to_string(env.path("b.txt")).unwrap(), "outside\n");
}

#[test]
fn run_passes_through_the_exit_status() {
    let env = Env::new();
    let out = env.run(&["run", "--", "sh", "-c", "exit 7"]);
    assert_eq!(out.status.code(), Some(7));
    let log = env.log();
    assert!(log[0]["note"].as_str().unwrap().contains("exit status 7"));
}

#[test]
fn commands_cannot_be_undone() {
    let env = Env::new();
    env.ok(&["run", "--", "true"]);
    let log = env.log();
    let out = env.run(&["undo", &id(&log[0])]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("shell command"));
}

#[test]
fn export_otel_emits_genai_spans() {
    let env = Env::new();
    env.ok(&["run", "--", "sh", "-c", "echo hi > out.txt"]);
    let out = env.ok(&["export", "--otel"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let spans = v["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0]["traceId"].as_str().unwrap().len(), 32);
    assert_eq!(spans[1]["parentSpanId"], spans[0]["spanId"]);
    let attrs = spans[0]["attributes"].as_array().unwrap();
    assert!(attrs.iter().any(
        |a| a["key"] == "gen_ai.operation.name" && a["value"]["stringValue"] == "execute_tool"
    ));
}

#[test]
fn log_is_plain_text_with_no_color() {
    let env = Env::new();
    env.ok(&["run", "--", "sh", "-c", "echo hi > out.txt"]);
    let out = env.ok(&["log"]);
    assert!(!out.contains('\x1b'), "no escape codes under NO_COLOR");
    assert!(out.contains("file_create"));
    assert!(out.contains("reversible"));
}

#[test]
fn a_command_that_cannot_start_leaves_no_entry() {
    let env = Env::new();
    let out = env.run(&["run", "--", "definitely-not-a-real-program-xyz"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(env.log().is_empty());
}

#[test]
fn files_moved_out_of_a_skipped_directory_are_never_deleted_by_undo() {
    let env = Env::new();
    fs::create_dir_all(env.path("target")).unwrap();
    fs::write(env.path("target/precious.txt"), "only copy\n").unwrap();
    env.ok(&["run", "--session", "mv", "--", "mv", "target", "deploy"]);
    let moved = env.find("file_create", &env.path("deploy/precious.txt"));
    assert_eq!(moved["reversible"], false);

    let out = env.run(&["undo", "--session", "mv"]);
    assert!(out.status.success());
    assert_eq!(
        fs::read_to_string(env.path("deploy/precious.txt")).unwrap(),
        "only copy\n"
    );
}

#[test]
fn a_same_size_edit_that_keeps_the_mtime_is_still_a_conflict() {
    let env = Env::new();
    fs::write(env.path("f.txt"), "v0\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo v1 > f.txt"]);
    let edit = env.find("file_write", &env.path("f.txt"));
    // the user rewrites the file with the same size and puts the old mtime back
    let ok = std::process::Command::new("sh")
        .current_dir(env.dir())
        .args([
            "-c",
            "cp -p f.txt ref && echo U9 > f.txt && touch -r ref f.txt && rm ref",
        ])
        .status()
        .unwrap();
    assert!(ok.success());
    let out = env.run(&["undo", &id(&edit)]);
    assert_eq!(out.status.code(), Some(3), "the change must be detected");
    assert_eq!(fs::read_to_string(env.path("f.txt")).unwrap(), "U9\n");
}

#[cfg(unix)]
#[test]
fn files_in_a_directory_that_becomes_readable_are_not_treated_as_new() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    fs::create_dir_all(env.path("locked")).unwrap();
    fs::write(env.path("locked/data.txt"), "keep\n").unwrap();
    fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(env.path("locked")).is_ok() {
        // running as root, where permissions do not apply
        fs::set_permissions(env.path("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    env.ok(&["run", "--session", "ch", "--", "chmod", "755", "locked"]);
    let seen = env.find("file_create", &env.path("locked/data.txt"));
    assert_eq!(seen["reversible"], false);
    env.ok(&["undo", "--session", "ch"]);
    assert_eq!(
        fs::read_to_string(env.path("locked/data.txt")).unwrap(),
        "keep\n"
    );
}

// regression tests for the second review round

#[test]
fn force_refuses_when_the_current_file_cannot_be_stored_first() {
    let env = Env::new();
    fs::write(env.path("notes.txt"), "v1\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo v2 > notes.txt"]);
    let edit = env.find("file_write", &env.path("notes.txt"));
    // the user replaces the file with something larger than the store limit
    let big = "u".repeat(5000);
    fs::write(env.path("notes.txt"), &big).unwrap();
    let out = env
        .cmd()
        .args(["undo", &id(&edit), "--force"])
        .env("FOOTPRINTS_MAX_FILE_BYTES", "1000")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--force is not available"), "{err}");
    assert_eq!(fs::read_to_string(env.path("notes.txt")).unwrap(), big);
}

#[test]
fn force_refuses_over_a_directory() {
    let env = Env::new();
    fs::write(env.path("d"), "file\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo changed > d"]);
    let edit = env.find("file_write", &env.path("d"));
    fs::remove_file(env.path("d")).unwrap();
    fs::create_dir(env.path("d")).unwrap();
    fs::write(env.path("d/inside"), "keep\n").unwrap();
    let out = env.run(&["undo", &id(&edit), "--force"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(fs::read_to_string(env.path("d/inside")).unwrap(), "keep\n");
}

#[cfg(unix)]
#[test]
fn a_symlink_replaced_by_a_file_is_never_undone_as_a_delete() {
    let env = Env::new();
    fs::write(env.path("real.conf"), "real\n").unwrap();
    std::os::unix::fs::symlink("real.conf", env.path("conf")).unwrap();
    env.ok(&[
        "run",
        "--session",
        "ln",
        "--",
        "sh",
        "-c",
        "c=$(cat conf); rm conf; echo \"$c plus edit\" > conf",
    ]);
    let a = env.find("file_write", &env.path("conf"));
    assert_eq!(a["reversible"], false);
    assert!(a["note"].as_str().unwrap().contains("symlink"));
    env.ok(&["undo", "--session", "ln"]);
    assert_eq!(
        fs::read_to_string(env.path("conf")).unwrap(),
        "real plus edit\n"
    );
}

#[test]
fn session_undo_removes_a_file_that_replaced_a_directory_before_restoring_under_it() {
    let env = Env::new();
    fs::create_dir(env.path("d")).unwrap();
    fs::write(env.path("d/x"), "inside\n").unwrap();
    fs::write(env.path("a"), "a1\n").unwrap();
    env.ok(&[
        "run",
        "--session",
        "flat",
        "--",
        "sh",
        "-c",
        "echo a2 > a; rm -r d; echo flat > d",
    ]);
    let out = env.ok(&["undo", "--session", "flat"]);
    assert!(out.contains("3 action(s) undone"), "{out}");
    assert_eq!(fs::read_to_string(env.path("d/x")).unwrap(), "inside\n");
    assert_eq!(fs::read_to_string(env.path("a")).unwrap(), "a1\n");
}

#[test]
fn session_undo_with_a_missing_pre_image_changes_nothing() {
    let env = Env::new();
    fs::write(env.path("a"), "a1\n").unwrap();
    fs::write(env.path("b"), "b1\n").unwrap();
    env.ok(&[
        "run",
        "--session",
        "m",
        "--",
        "sh",
        "-c",
        "echo a2 > a; echo b2 > b",
    ]);
    // remove the stored pre-image of b
    let b = env.find("file_write", &env.path("b"));
    let h = b["pre_hash"].as_str().unwrap();
    fs::remove_file(env.home.path().join("blobs").join(&h[..2]).join(&h[2..])).unwrap();
    let out = env.run(&["undo", "--session", "m"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing from the blob store"));
    assert_eq!(fs::read_to_string(env.path("a")).unwrap(), "a2\n");
    assert_eq!(fs::read_to_string(env.path("b")).unwrap(), "b2\n");
}

#[cfg(unix)]
#[test]
fn copies_of_a_private_file_are_not_readable_by_others() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    fs::write(env.path(".env"), "TOKEN=secret\n").unwrap();
    fs::set_permissions(env.path(".env"), fs::Permissions::from_mode(0o600)).unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo EXTRA=1 >> .env"]);
    let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let blobs = env.home.path().join("blobs");
    assert_eq!(mode(&blobs), 0o700);
    let mut checked = 0;
    for shard in fs::read_dir(&blobs).unwrap() {
        let shard = shard.unwrap().path();
        assert_eq!(mode(&shard), 0o700);
        for f in fs::read_dir(&shard).unwrap() {
            assert_eq!(mode(&f.unwrap().path()), 0o600);
            checked += 1;
        }
    }
    assert!(checked >= 2);
    assert_eq!(mode(&env.home.path().join("staging")), 0o700);
    assert_eq!(mode(&env.home.path().join("ledger.db")), 0o600);
}

#[cfg(unix)]
#[test]
fn a_run_killed_outright_is_recovered_later() {
    let env = Env::new();
    fs::write(env.path("f"), "before\n").unwrap();
    // the child kills the wrapper with SIGKILL, which no handler can catch
    let out = env.run(&[
        "run",
        "--session",
        "k",
        "--",
        "sh",
        "-c",
        "echo after > f; kill -9 $PPID",
    ]);
    assert!(!out.status.success());
    assert_eq!(env.log()[0]["status"], "pending");

    let rec = env.ok(&["recover"]);
    assert!(rec.contains("recovered"), "{rec}");
    let f = env.find("file_write", &env.path("f"));
    assert_eq!(f["reversible"], true);
    env.ok(&["undo", &id(&f)]);
    assert_eq!(fs::read_to_string(env.path("f")).unwrap(), "before\n");
}

#[cfg(unix)]
#[test]
fn hangup_and_terminate_do_not_stop_the_record() {
    let env = Env::new();
    fs::write(env.path("f"), "before\n").unwrap();
    for sig in ["HUP", "TERM"] {
        let script = format!("trap '' {sig}; kill -{sig} $PPID; echo {sig} > f");
        let out = env.run(&["run", "--", "sh", "-c", &script]);
        assert!(
            out.status.success(),
            "{sig}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let log = env.log();
    assert!(log.iter().all(|a| a["status"] == "recorded"));
    assert_eq!(log.iter().filter(|a| a["kind"] == "file_write").count(), 2);
}

#[test]
fn overlapping_runs_are_refused() {
    let env = Env::new();
    fs::create_dir(env.path("sub")).unwrap();
    let mut first = env.cmd().args(["run", "--", "sleep", "2"]).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let out = env.run(&["run", "--dir", "sub", "--", "true"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("another footprints run"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(first.wait().unwrap().success());
    // once the first has finished, the same directory is free again
    env.ok(&["run", "--dir", "sub", "--", "true"]);
}

#[test]
fn run_dir_runs_the_child_in_that_directory() {
    let env = Env::new();
    fs::create_dir(env.path("site")).unwrap();
    env.ok(&[
        "run",
        "--dir",
        "site",
        "--",
        "sh",
        "-c",
        "echo here > made.txt",
    ]);
    assert!(env.path("site/made.txt").exists());
    assert!(!env.path("made.txt").exists());
}

#[test]
fn undoing_an_undo_restores_the_change_and_batches_stay_separate() {
    let env = Env::new();
    fs::write(env.path("f"), "v0\n").unwrap();
    fs::write(env.path("g"), "g0\n").unwrap();
    env.ok(&["run", "--", "sh", "-c", "echo v1 > f; echo g1 > g"]);
    let f = env.find("file_write", &env.path("f"));
    let g = env.find("file_write", &env.path("g"));
    let first = env.ok(&["undo", &id(&f)]);
    env.ok(&["undo", &id(&g)]);
    let batch = first
        .split("in session ")
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(')')
        .to_string();
    // reversing the first batch only touches f
    env.ok(&["undo", "--session", &batch]);
    assert_eq!(fs::read_to_string(env.path("f")).unwrap(), "v1\n");
    assert_eq!(fs::read_to_string(env.path("g")).unwrap(), "g0\n");
    // and the original change to f can be undone again
    let again = env.log().into_iter().find(|a| a["id"] == f["id"]).unwrap();
    assert_eq!(again["status"], "recorded");
    env.ok(&["undo", &id(&f)]);
    assert_eq!(fs::read_to_string(env.path("f")).unwrap(), "v0\n");
}

#[test]
fn a_wrapper_killed_from_outside_is_recovered_on_every_platform() {
    let env = Env::new();
    fs::write(env.path("f"), "before\n").unwrap();
    // the child edits f, then waits for a marker so the wrapper can be killed while it runs
    let mut wrapper = env
        .cmd()
        .args([
            "run",
            "--session",
            "killed",
            "--",
            "sh",
            "-c",
            "echo after > f; while [ ! -e go ]; do sleep 0.1; done",
        ])
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while fs::read_to_string(env.path("f")).unwrap_or_default() != "after\n" {
        assert!(start.elapsed().as_secs() < 20, "the child never ran");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // TerminateProcess on Windows and SIGKILL on Unix: no handler runs
    wrapper.kill().unwrap();
    wrapper.wait().unwrap();
    fs::write(env.path("go"), "").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let rec = env.ok(&["recover"]);
    assert!(rec.contains("recovered"), "{rec}");
    let f = env.find("file_write", &env.path("f"));
    assert_eq!(f["reversible"], true);
    env.ok(&["undo", &id(&f)]);
    assert_eq!(fs::read_to_string(env.path("f")).unwrap(), "before\n");
}
