//! `footprints prune` and `footprints import`. Every test uses its own data directory and sends
//! archives to a folder named by FOOTPRINTS_TRASH_DIR, so the real Trash is never touched.

mod common;

use common::Env;
use footprints::store::{Action, Kind, Status, Store};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    footprints::store::now_ms()
}

struct Ledger {
    env: Env,
    trash: tempfile::TempDir,
}

impl Ledger {
    fn new() -> Self {
        Ledger {
            env: Env::new(),
            trash: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> &Path {
        self.env.home.path()
    }

    fn store(&self) -> Store {
        Store::open(self.home()).unwrap()
    }

    /// a recorded file write with its own pre-image and post-image, `days` days old.
    fn write(&self, session: &str, agent: &str, days: i64, before: &[u8], after: &[u8]) -> i64 {
        let store = self.store();
        let mut a = Action::new(agent, session, Kind::FileWrite, &format!("/demo/{session}"));
        a.time_ms = now_ms() - days * DAY_MS;
        a.end_time_ms = Some(a.time_ms);
        a.pre_hash = Some(store.blobs.put(before).unwrap());
        a.pre_size = Some(before.len() as i64);
        a.post_hash = Some(store.blobs.put(after).unwrap());
        a.post_size = Some(after.len() as i64);
        a.reversible = true;
        store.insert(&a).unwrap()
    }

    fn prune(&self, args: &[&str]) -> Output {
        let mut c = self.env.cmd();
        c.arg("prune")
            .args(args)
            .env("FOOTPRINTS_TRASH_DIR", self.trash.path());
        c.output().unwrap()
    }

    fn prune_ok(&self, args: &[&str]) -> String {
        let out = self.prune(args);
        assert!(
            out.status.success(),
            "prune {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn ids(&self) -> Vec<i64> {
        self.env
            .log()
            .iter()
            .map(|a| a["id"].as_i64().unwrap())
            .collect()
    }

    fn archives(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(self.trash.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        v.sort();
        v
    }
}

fn blob_path(home: &Path, hash: &str) -> PathBuf {
    home.join("blobs").join(&hash[..2]).join(&hash[2..])
}

fn hash(bytes: &[u8]) -> String {
    footprints::blob::hash_bytes(bytes)
}

/// every file under `dir` with its bytes, leaving out the lock files that any open creates.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                if e.file_name() != "locks" {
                    walk(root, &p, out);
                }
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// the blob hashes stored in an archive folder.
fn archive_blobs(archive: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for shard in fs::read_dir(archive.join("blobs")).unwrap() {
        let shard = shard.unwrap();
        for e in fs::read_dir(shard.path()).unwrap() {
            out.insert(format!(
                "{}{}",
                shard.file_name().to_string_lossy(),
                e.unwrap().file_name().to_string_lossy()
            ));
        }
    }
    out
}

/// the ids in an archive, read through footprints itself, which proves it is a usable data
/// directory.
fn archive_ids(archive: &Path) -> Vec<i64> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_footprints"))
        .args(["log", "--all", "--json"])
        .env("FOOTPRINTS_HOME", archive)
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn a_dry_run_changes_nothing() {
    let l = Ledger::new();
    l.write("s1", "demo", 40, b"one", b"two");
    l.write("s2", "demo", 1, b"three", b"four");
    let before = tree(l.home());
    let out = l.prune_ok(&["--all"]);
    assert!(
        out.contains("Would prune 2 action(s) in 2 session(s)"),
        "{out}"
    );
    assert!(out.contains("Nothing was changed"), "{out}");
    assert_eq!(
        tree(l.home()),
        before,
        "the data directory must be byte for byte the same"
    );
    assert!(l.archives().is_empty());
}

#[test]
fn older_than_keeps_recent_actions() {
    let l = Ledger::new();
    let old = l.write("s1", "demo", 40, b"a", b"b");
    let recent = l.write("s2", "demo", 2, b"c", b"d");
    l.prune_ok(&["--older-than", "30d", "--yes"]);
    assert_eq!(l.ids(), [recent]);
    assert_eq!(archive_ids(&l.archives()[0]), [old]);
}

#[test]
fn before_selects_by_date() {
    let l = Ledger::new();
    let old = l.write("s1", "demo", 10, b"a", b"b");
    let recent = l.write("s2", "demo", 1, b"c", b"d");
    let cutoff = chrono::Local::now() - chrono::Duration::days(5);
    let date = cutoff.format("%Y-%m-%d").to_string();
    l.prune_ok(&["--before", &date, "--yes"]);
    assert_eq!(l.ids(), [recent]);
    assert_eq!(archive_ids(&l.archives()[0]), [old]);
}

#[test]
fn session_and_agent_selectors_combine() {
    let l = Ledger::new();
    let a1 = l.write("s1", "alpha", 1, b"a", b"b");
    let b1 = l.write("s1", "beta", 1, b"c", b"d");
    let a2 = l.write("s2", "alpha", 1, b"e", b"f");
    l.prune_ok(&["--session", "s1", "--agent", "alpha", "--yes"]);
    assert_eq!(
        l.ids(),
        [b1, a2],
        "only the action matching both selectors goes"
    );
    assert_eq!(archive_ids(&l.archives()[0]), [a1]);
    l.prune_ok(&["--agent", "alpha", "--yes"]);
    assert_eq!(l.ids(), [b1]);
    l.prune_ok(&["--session", "s1", "--yes"]);
    assert!(l.ids().is_empty());
    assert_eq!(l.archives().len(), 3, "each prune keeps its own archive");
}

#[test]
fn all_removes_everything_and_vacuums() {
    let l = Ledger::new();
    {
        let store = l.store();
        let long = "x".repeat(4000);
        for i in 0..200 {
            let mut a = Action::new("demo", "s1", Kind::Command, &format!("echo {i} {long}"));
            a.intent = Some(long.clone());
            store.insert(&a).unwrap();
        }
    }
    let db = l.home().join("ledger.db");
    let before = fs::metadata(&db).unwrap().len();
    let out = l.prune_ok(&["--all", "--yes"]);
    assert!(
        out.contains("Pruned 200 action(s) in 1 session(s)"),
        "{out}"
    );
    assert!(l.ids().is_empty());
    let after = fs::metadata(&db).unwrap().len();
    assert!(
        after * 10 < before,
        "VACUUM should give the space back: {before} bytes before, {after} after"
    );
}

#[test]
fn max_size_prunes_the_oldest_sessions_first() {
    let l = Ledger::new();
    // content that does not compress, so each session's share of the store is known
    let noise = |seed: u8| -> Vec<u8> {
        let mut x = seed as u64 | 1;
        (0..200_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    };
    let oldest = l.write("old", "demo", 30, &noise(1), &noise(2));
    let middle = l.write("middle", "demo", 20, &noise(3), &noise(4));
    let newest = l.write("new", "demo", 10, &noise(5), &noise(6));
    let size = footprints::prune::data_size(l.home());
    // just under the current size, so removing the oldest session is enough
    let budget = (size - 100_000).to_string();
    l.prune_ok(&["--max-size", &budget, "--yes"]);
    assert_eq!(l.ids(), [middle, newest]);
    assert_eq!(archive_ids(&l.archives()[0]), [oldest]);
    assert!(footprints::prune::data_size(l.home()) <= size - 100_000);
    // already within budget: nothing more goes
    let out = l.prune_ok(&["--max-size", "1GB", "--yes"]);
    assert!(out.contains("Nothing to prune"), "{out}");
    assert_eq!(l.ids(), [middle, newest]);
}

#[test]
fn a_blob_another_action_uses_survives() {
    let l = Ledger::new();
    l.write("s1", "demo", 40, b"shared before\n", b"only old\n");
    let kept = l.write("s2", "demo", 1, b"shared before\n", b"only new\n");
    let home = l.home();
    l.prune_ok(&["--session", "s1", "--yes"]);
    assert!(blob_path(home, &hash(b"shared before\n")).exists());
    assert!(blob_path(home, &hash(b"only new\n")).exists());
    assert!(!blob_path(home, &hash(b"only old\n")).exists());
    // the remaining action still diffs and so still has both of its images
    let diff = l.env.ok(&["diff", &kept.to_string()]);
    assert!(diff.contains("+only new"), "{diff}");
}

#[test]
fn the_archive_holds_exactly_the_pruned_rows_and_their_blobs() {
    let l = Ledger::new();
    let gone = l.write("s1", "demo", 40, b"shared\n", b"old after\n");
    let gone2 = l.write("s1", "demo", 40, b"old two\n", b"old three\n");
    l.write("s2", "demo", 1, b"shared\n", b"new after\n");
    l.prune_ok(&["--session", "s1", "--yes"]);
    let archives = l.archives();
    assert_eq!(archives.len(), 1);
    let archive = &archives[0];
    let name = archive.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("footprints-pruned-") && name.ends_with("-1"),
        "{name}"
    );
    assert_eq!(archive_ids(archive), [gone, gone2]);
    // self-contained: every image the pruned rows name, including one that also stays behind
    let want: BTreeSet<String> = [
        &b"shared\n"[..],
        b"old after\n",
        b"old two\n",
        b"old three\n",
    ]
    .iter()
    .map(|b| hash(b))
    .collect();
    assert_eq!(archive_blobs(archive), want);
    assert!(archive.join("README.txt").exists());
    // the archive reads like any data directory
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_footprints"))
        .args(["diff", &gone.to_string()])
        .env("FOOTPRINTS_HOME", archive)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("+old after"));
}

#[test]
fn permanent_keeps_no_archive() {
    let l = Ledger::new();
    l.write("s1", "demo", 40, b"a\n", b"b\n");
    let out = l.prune_ok(&["--all", "--permanent", "--yes"]);
    assert!(out.contains("No archive was kept"), "{out}");
    assert!(l.ids().is_empty());
    assert!(l.archives().is_empty());
    assert!(!blob_path(l.home(), &hash(b"a\n")).exists());
    assert!(!l.home().join("pruning").exists());
}

#[test]
fn no_selector_is_a_usage_error() {
    let l = Ledger::new();
    l.write("s1", "demo", 40, b"a", b"b");
    let before = tree(l.home());
    for args in [&["--yes"][..], &[][..], &["--permanent", "--yes"][..]] {
        let out = l.prune(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("--older-than"));
    }
    assert_eq!(tree(l.home()), before);
}

#[test]
fn refuses_while_another_process_has_the_ledger_open() {
    let l = Ledger::new();
    let id = l.write("s1", "demo", 40, b"a", b"b");
    let held = l.store();
    let out = l.prune(&["--all", "--yes"]);
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refused: another footprints process"), "{err}");
    // a dry run still answers and says why --yes would fail
    let dry = l.prune_ok(&["--all"]);
    assert!(dry.contains("would be refused"), "{dry}");
    drop(held);
    assert_eq!(l.ids(), [id]);
    assert!(l.archives().is_empty());
}

#[test]
fn a_session_still_being_recorded_is_kept() {
    let l = Ledger::new();
    let done = l.write("s1", "demo", 1, b"a", b"b");
    let live = {
        let store = l.store();
        let mut a = Action::new("demo", "s2", Kind::FileWrite, "/demo/live");
        a.status = Status::Pending;
        store.insert(&a).unwrap()
    };
    let out = l.prune_ok(&["--all", "--yes"]);
    assert!(
        out.contains("Kept 1 session(s) still being recorded: s2"),
        "{out}"
    );
    assert_eq!(l.ids(), [live]);
    assert_eq!(archive_ids(&l.archives()[0]), [done]);
}

#[test]
fn a_command_and_its_changes_go_together() {
    let l = Ledger::new();
    let (cmd, child) = {
        let store = l.store();
        let mut c = Action::new("demo", "s1", Kind::Command, "sh agent.sh");
        c.time_ms = now_ms() - 40 * DAY_MS;
        c.end_time_ms = Some(now_ms() - DAY_MS);
        let cmd = store.insert(&c).unwrap();
        let mut f = Action::new("demo", "s1", Kind::FileCreate, "/demo/new.txt");
        f.time_ms = now_ms() - 40 * DAY_MS;
        f.end_time_ms = Some(f.time_ms);
        f.parent_id = Some(cmd);
        (cmd, store.insert(&f).unwrap())
    };
    // the change is old but the command finished recently, so neither goes
    let out = l.prune_ok(&["--older-than", "30d", "--yes"]);
    assert!(out.contains("Nothing to prune"), "{out}");
    assert!(out.contains("Kept 1 matching action(s)"), "{out}");
    assert_eq!(l.ids(), [cmd, child]);
    l.prune_ok(&["--older-than", "12h", "--yes"]);
    assert!(l.ids().is_empty());
    assert_eq!(archive_ids(&l.archives()[0]), [cmd, child]);
}

#[test]
fn a_failed_archive_changes_nothing() {
    let l = Ledger::new();
    let id = l.write("s1", "demo", 40, b"a", b"b");
    let not_a_dir = l.trash.path().join("file");
    fs::write(&not_a_dir, "x").unwrap();
    let out = l
        .env
        .cmd()
        .args(["prune", "--all", "--yes"])
        .env("FOOTPRINTS_TRASH_DIR", &not_a_dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(l.ids(), [id]);
    assert!(blob_path(l.home(), &hash(b"a")).exists());
    assert!(!l.home().join("pruning").exists());
}

#[test]
fn import_restores_an_archive_once() {
    let l = Ledger::new();
    let old = l.write("s1", "demo", 40, b"before\n", b"after\n");
    let new = l.write("s2", "demo", 1, b"x\n", b"y\n");
    let original = l.env.log();
    l.prune_ok(&["--session", "s1", "--yes"]);
    assert_eq!(l.ids(), [new]);
    let archive = l.archives().remove(0);
    let archive = archive.to_str().unwrap();
    let out = l.env.ok(&["import", archive]);
    assert!(
        out.contains("restored 1 action(s) in 1 session(s)"),
        "{out}"
    );
    assert_eq!(l.env.log(), original, "every field and id comes back");
    let diff = l.env.ok(&["diff", &old.to_string()]);
    assert!(diff.contains("+after"), "{diff}");
    let again = l.env.run(&["import", archive]);
    assert_eq!(again.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&again.stderr).contains("restored already"));
    assert_eq!(l.env.log(), original);
}

#[test]
fn prune_is_not_offered_over_mcp() {
    let l = Ledger::new();
    let id = l.write("s1", "demo", 40, b"a", b"b");
    let input: String = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "footprints_prune", "arguments": { "all": true } } }),
    ]
    .iter()
    .map(|m| format!("{m}\n"))
    .collect();
    let out = l.env.with_stdin(&["mcp"], &input);
    let text = String::from_utf8_lossy(&out.stdout);
    let replies: Vec<Value> = text
        .lines()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
    let names: Vec<&str> = replies[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        !names
            .iter()
            .any(|n| n.contains("prune") || n.contains("import"))
    );
    assert!(
        replies[1].get("error").is_some() || replies[1]["result"]["isError"] == true,
        "{}",
        replies[1]
    );
    assert_eq!(l.ids(), [id]);
}
