//! the whole-machine daemon, run for a few seconds over a temporary root.

mod common;

use common::{Env, id};
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};

/// starts the daemon and waits until its watchers are in place.
fn start(env: &Env, root: &str, extra: &[&str], seconds: u64) -> Child {
    let secs = seconds.to_string();
    let mut args = vec!["daemon", "--root", root, "--seconds", &secs];
    args.extend_from_slice(extra);
    let mut child = env
        .cmd()
        .args(&args)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            err.read_line(&mut line).unwrap() > 0,
            "the daemon exited before it was ready"
        );
        if line.contains("daemon recording changes") {
            break;
        }
    }
    // keep reading so the daemon never blocks on a full pipe
    std::thread::spawn(move || for _ in err.lines() {});
    // FSEvents can deliver events from just before the stream started, so changes begin a
    // moment later
    std::thread::sleep(std::time::Duration::from_millis(500));
    child
}

fn finish(mut child: Child) {
    assert!(child.wait().unwrap().success());
}

#[test]
fn daemon_with_a_snapshot_records_any_process_and_undo_restores_bytes() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    fs::write(env.path("root/edit.txt"), "before\n").unwrap();
    fs::write(env.path("root/doomed.bin"), [0u8, 1, 2, 255]).unwrap();
    let d = start(&env, "root", &["--snapshot"], 5);
    fs::write(env.path("root/edit.txt"), "after\n").unwrap();
    fs::write(env.path("root/new.txt"), "fresh\n").unwrap();
    fs::remove_file(env.path("root/doomed.bin")).unwrap();
    finish(d);

    let log = env.log();
    assert_eq!(log[0]["kind"], "command");
    assert_eq!(log[0]["status"], "recorded");
    assert!(log.iter().all(|a| a["agent"] == "unknown"));
    let edit = env.find("file_write", &env.path("root/edit.txt"));
    let new = env.find("file_create", &env.path("root/new.txt"));
    let gone = env.find("file_delete", &env.path("root/doomed.bin"));
    for a in [&edit, &new, &gone] {
        assert_eq!(a["reversible"], true, "{a}");
        assert_eq!(a["parent_id"], log[0]["id"]);
    }
    env.ok(&["undo", &id(&edit)]);
    env.ok(&["undo", &id(&new)]);
    env.ok(&["undo", &id(&gone)]);
    assert_eq!(
        fs::read_to_string(env.path("root/edit.txt")).unwrap(),
        "before\n"
    );
    assert!(!env.path("root/new.txt").exists());
    assert_eq!(
        fs::read(env.path("root/doomed.bin")).unwrap(),
        [0u8, 1, 2, 255]
    );
}

#[test]
fn without_a_snapshot_only_a_second_change_is_reversible() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    fs::write(env.path("root/f.txt"), "v0\n").unwrap();
    let d = start(&env, "root", &[], 6);
    fs::write(env.path("root/f.txt"), "v1\n").unwrap();
    // long enough for the daemon to record the first change on its own
    std::thread::sleep(std::time::Duration::from_millis(2000));
    fs::write(env.path("root/f.txt"), "v2\n").unwrap();
    finish(d);
    let changes: Vec<_> = env
        .log()
        .into_iter()
        .filter(|a| a["kind"] == "file_write")
        .collect();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(changes[0]["reversible"], false);
    assert!(
        changes[0]["note"]
            .as_str()
            .unwrap()
            .contains("no pre-image")
    );
    assert_eq!(changes[1]["reversible"], true);
    env.ok(&["undo", &id(&changes[1])]);
    assert_eq!(fs::read_to_string(env.path("root/f.txt")).unwrap(), "v1\n");
}

#[test]
fn a_change_the_wrapper_records_is_joined_not_duplicated() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    fs::write(env.path("root/a.txt"), "one\n").unwrap();
    let d = start(&env, "root", &["--snapshot"], 5);
    env.ok(&[
        "run",
        "--dir",
        "root",
        "--agent",
        "demo",
        "--",
        "sh",
        "-c",
        "echo two > a.txt",
    ]);
    finish(d);
    let log = env.log();
    let writes: Vec<_> = log.iter().filter(|a| a["kind"] == "file_write").collect();
    assert_eq!(writes.len(), 1, "{log:?}");
    assert_eq!(writes[0]["agent"], "demo");
    let daemon = log.iter().find(|a| a["tool"] == "daemon").unwrap();
    assert!(
        daemon["note"]
            .as_str()
            .unwrap()
            .contains("1 already recorded"),
        "{}",
        daemon["note"]
    );
}

#[test]
fn a_second_daemon_on_the_same_ledger_is_refused() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    let d = start(&env, "root", &[], 3);
    let out = env.run(&["daemon", "--root", "root", "--seconds", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already recording"));
    finish(d);
}

#[test]
fn platform_only_sources_are_refused_elsewhere() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    let flag = if cfg!(target_os = "linux") {
        "--usn"
    } else {
        "--fanotify"
    };
    let out = env.run(&["daemon", "--root", "root", "--seconds", "1", flag]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("only available"));
}

#[cfg(windows)]
#[test]
fn usn_journal_reader_runs_when_the_volume_can_be_opened() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    // CI runners are administrators; elsewhere opening the volume fails and that is reported
    let probe = env.run(&["daemon", "--root", "root", "--seconds", "1", "--usn"]);
    if !probe.status.success() {
        let err = String::from_utf8_lossy(&probe.stderr);
        assert!(
            std::env::var_os("GITHUB_ACTIONS").is_none(),
            "GitHub runners are administrators, so the journal must open there: {err}"
        );
        assert!(err.contains("administrator"), "{err}");
        return;
    }
    let d = start(&env, "root", &["--snapshot", "--usn"], 5);
    fs::write(env.path("root/u.txt"), "usn\n").unwrap();
    finish(d);
    let a = env.find("file_create", &env.path("root/u.txt"));
    assert_eq!(a["reversible"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn fanotify_without_the_capability_is_reported() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    let out = env.run(&["daemon", "--root", "root", "--seconds", "1", "--fanotify"]);
    if out.status.success() {
        // running with CAP_SYS_ADMIN, where fanotify works
        return;
    }
    assert!(String::from_utf8_lossy(&out.stderr).contains("CAP_SYS_ADMIN"));
}

#[test]
fn a_file_moved_in_from_outside_survives_a_session_undo() {
    let env = Env::new();
    fs::create_dir(env.path("root")).unwrap();
    fs::write(env.path("outside.txt"), "the only copy\n").unwrap();
    // an existing file, last written an hour ago
    fs::File::options()
        .write(true)
        .open(env.path("outside.txt"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
    let d = start(&env, "root", &["--snapshot"], 4);
    fs::rename(env.path("outside.txt"), env.path("root/moved.txt")).unwrap();
    finish(d);
    let moved = env.find("file_create", &env.path("root/moved.txt"));
    assert_eq!(moved["reversible"], false, "{moved}");
    let session = moved["session"].as_str().unwrap().to_string();
    env.ok(&["undo", "--session", &session]);
    assert_eq!(
        fs::read_to_string(env.path("root/moved.txt")).unwrap(),
        "the only copy\n"
    );
}
