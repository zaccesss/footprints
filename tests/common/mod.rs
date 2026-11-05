//! shared helpers: every test gets its own data directory and working directory in a temp
//! folder, so no test can read or change a real ledger.

#![allow(dead_code)]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub struct Env {
    pub home: tempfile::TempDir,
    pub work: tempfile::TempDir,
}

impl Env {
    pub fn new() -> Self {
        Env {
            home: tempfile::tempdir().unwrap(),
            work: tempfile::tempdir().unwrap(),
        }
    }

    /// the working directory with symlinks resolved, matching the paths the ledger stores
    /// (on macOS the temp folder sits behind the /var to /private/var link)
    pub fn dir(&self) -> PathBuf {
        self.work.path().canonicalize().unwrap()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir().join(rel)
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_footprints"));
        c.env("FOOTPRINTS_HOME", self.home.path())
            .env("NO_COLOR", "1")
            .current_dir(self.dir());
        c
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "footprints {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn with_stdin(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .cmd()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    pub fn log(&self) -> Vec<Value> {
        let out = self.ok(&["log", "--all", "--json"]);
        serde_json::from_str::<Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }

    /// the newest action of `kind` whose target is `path`
    pub fn find(&self, kind: &str, path: &Path) -> Value {
        let p = path.to_string_lossy();
        self.log()
            .into_iter()
            .rev()
            .find(|a| a["kind"] == kind && a["target"] == *p)
            .unwrap_or_else(|| panic!("no {kind} action for {p}"))
    }
}

pub fn id(a: &Value) -> String {
    a["id"].as_i64().unwrap().to_string()
}
