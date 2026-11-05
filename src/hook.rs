//! capture tier 1: the Claude Code PreToolUse, PostToolUse and PostToolUseFailure hook adapter.
//!
//! the pre hook stores the pre-image before a tool runs and leaves a pending action. The post
//! hook, which also handles failed calls, finds that action, stores the post-image and marks
//! it recorded.

use crate::snapshot::{self, Before, Manifest};
use crate::store::{Action, Kind, Status, Store, now_ms};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const DEFAULT_AGENT: &str = "claude-code";

/// the documented hook payload. Fields not used here are ignored so new versions of the agent
/// do not break the adapter.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: Value,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    /// set on PostToolUseFailure
    #[serde(default)]
    pub error: Option<String>,
    /// set on PostToolUseFailure when the call was aborted rather than failing by itself
    #[serde(default)]
    pub is_interrupt: Option<bool>,
}

enum Target {
    File(PathBuf),
    Command(String),
    Network(String),
}

impl HookInput {
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("hook input is not the expected JSON payload")
    }

    fn session(&self) -> &str {
        self.session_id.as_deref().unwrap_or("unknown")
    }

    fn str_field(&self, name: &str) -> Option<&str> {
        self.tool_input
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    fn intent(&self) -> Option<String> {
        self.str_field("description").map(str::to_string)
    }

    fn target(&self) -> Option<Target> {
        match self.tool_name.as_str() {
            "Write" | "Edit" | "MultiEdit" => self.str_field("file_path").map(|p| self.file(p)),
            "NotebookEdit" => self.str_field("notebook_path").map(|p| self.file(p)),
            "Bash" => self
                .str_field("command")
                .map(|c| Target::Command(c.to_string())),
            "WebFetch" => self
                .str_field("url")
                .map(|u| Target::Network(u.to_string())),
            "WebSearch" => self
                .str_field("query")
                .map(|q| Target::Network(format!("search: {q}"))),
            _ => None,
        }
    }

    /// a relative path is resolved against the payload's cwd. Without one it is resolved against
    /// this process's own directory, so the ledger never stores a path whose meaning depends on
    /// where `undo` is later run.
    fn file(&self, p: &str) -> Target {
        let path = PathBuf::from(p);
        let abs = if path.is_absolute() {
            path
        } else {
            let base = match &self.cwd {
                Some(c) if Path::new(c).is_absolute() => PathBuf::from(c),
                _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            };
            base.join(path)
        };
        Target::File(snapshot::normalise(&abs))
    }

    fn target_string(&self) -> Option<String> {
        self.target().map(|t| match t {
            Target::File(p) => p.to_string_lossy().into_owned(),
            Target::Command(c) | Target::Network(c) => c,
        })
    }

    fn failed(&self) -> bool {
        self.hook_event_name.as_deref() == Some("PostToolUseFailure") || self.error.is_some()
    }

    /// a sentence about a failed call, for the action's note.
    fn failure_note(&self) -> Option<String> {
        if !self.failed() {
            return None;
        }
        if self.is_interrupt == Some(true) {
            return Some("the tool call was interrupted".into());
        }
        let first = self
            .error
            .as_deref()
            .and_then(|e| e.lines().next())
            .unwrap_or("no error message");
        let mut first: String = first.chars().take(200).collect();
        if first.is_empty() {
            first = "no error message".into();
        }
        Some(format!("the tool call failed: {first}"))
    }
}

fn join_note(parts: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let v: Vec<String> = parts.into_iter().flatten().collect();
    if v.is_empty() {
        None
    } else {
        Some(v.join("; "))
    }
}

/// handles a PreToolUse payload. Returns the id of the pending action. A tool that changes
/// nothing on disk gives none.
pub fn pre(store: &Store, input: &HookInput, agent: &str) -> Result<Option<i64>> {
    let Some(target) = input.target() else {
        return Ok(None);
    };
    let mut a = match &target {
        Target::File(path) => {
            let img = snapshot::capture(store, path)?;
            let mut a = Action::new(
                agent,
                input.session(),
                Kind::for_file(img.exists, true),
                &path.to_string_lossy(),
            );
            a.pre_hash = img.hash.clone();
            a.pre_size = if img.exists { img.size } else { None };
            a.pre_mode = img.mode;
            a.reversible = img.recoverable();
            if let Some(why) = &img.unstored {
                a.note = Some(format!("the old contents were not stored because {why}"));
            }
            a
        }
        Target::Command(cmd) => {
            let mut a = Action::new(agent, input.session(), Kind::Command, cmd);
            a.note = Some(
                "a shell command cannot be undone as a whole; file changes it made inside the \
                 working directory are listed as separate actions"
                    .into(),
            );
            if let Some(cwd) = &input.cwd {
                let root = Path::new(cwd)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(cwd));
                let m = snapshot::build_manifest(store, &root, &snapshot::exclusions(store))?;
                if m.truncated {
                    a.note = Some(format!(
                        "the working directory is too large to snapshot in one hook call (over {} \
                         files, {} MiB of new content or {} seconds), so file changes made by this \
                         command were not recorded; later calls get further as the cache fills, \
                         and FOOTPRINTS_MAX_FILES, FOOTPRINTS_SNAPSHOT_BUDGET_BYTES and \
                         FOOTPRINTS_SNAPSHOT_SECONDS raise the limits",
                        snapshot::max_files(),
                        snapshot::snapshot_budget() / (1024 * 1024),
                        snapshot::snapshot_time_budget().as_secs()
                    ));
                } else {
                    a.manifest_hash = Some(store.blobs.put(&serde_json::to_vec(&m)?)?);
                }
            }
            a
        }
        Target::Network(what) => {
            let mut a = Action::new(agent, input.session(), Kind::Network, what);
            a.note = Some(format!(
                "network call sent by {}; it cannot be undone",
                input.tool_name
            ));
            a
        }
    };
    a.status = Status::Pending;
    a.tool = Some(input.tool_name.clone());
    a.tool_use_id = input.tool_use_id.clone();
    a.cwd = input.cwd.clone();
    a.intent = input.intent();
    Ok(Some(store.insert(&a)?))
}

/// handles a PostToolUse or PostToolUseFailure payload. Returns the id of the recorded
/// action. A tool that turned out to change nothing gives none.
pub fn post(store: &Store, input: &HookInput, agent: &str) -> Result<Option<i64>> {
    let Some(target) = input.target() else {
        return Ok(None);
    };
    let target_str = input.target_string();
    let pending = store.find_pending(
        input.session_id.as_deref(),
        input.tool_use_id.as_deref(),
        &input.tool_name,
        target_str.as_deref(),
    )?;
    match target {
        Target::File(path) => post_file(store, input, agent, &path, pending),
        Target::Command(cmd) => post_command(store, input, agent, &cmd, pending),
        Target::Network(what) => {
            let mut a = match pending {
                Some(a) => a,
                None => {
                    let mut a = Action::new(agent, input.session(), Kind::Network, &what);
                    a.tool = Some(input.tool_name.clone());
                    a.tool_use_id = input.tool_use_id.clone();
                    a.note = Some("network call; it cannot be undone".into());
                    a.id = store.insert(&a)?;
                    a
                }
            };
            a.note = join_note([a.note.take(), input.failure_note()]);
            a.status = Status::Recorded;
            a.end_time_ms = Some(now_ms());
            store.save(&a)?;
            Ok(Some(a.id))
        }
    }
}

fn post_file(
    store: &Store,
    input: &HookInput,
    agent: &str,
    path: &Path,
    pending: Option<Action>,
) -> Result<Option<i64>> {
    let img = snapshot::capture(store, path)?;
    let Some(mut a) = pending else {
        // without a pre-image the change is visible but cannot be reversed
        let mut a = Action::new(
            agent,
            input.session(),
            if img.exists {
                Kind::FileWrite
            } else {
                Kind::FileDelete
            },
            &path.to_string_lossy(),
        );
        a.post_hash = img.hash;
        a.post_size = if img.exists { img.size } else { None };
        a.tool = Some(input.tool_name.clone());
        a.tool_use_id = input.tool_use_id.clone();
        a.cwd = input.cwd.clone();
        a.intent = input.intent();
        a.note = join_note([
            Some("no pre-image: the pre hook did not run for this tool call".into()),
            input.failure_note(),
        ]);
        a.end_time_ms = Some(now_ms());
        return Ok(Some(store.insert(&a)?));
    };
    let existed = a.pre_size.is_some();
    if existed == img.exists && a.pre_hash.is_some() && a.pre_hash == img.hash {
        // the tool failed or wrote identical bytes, so there is nothing to undo
        store.delete(a.id)?;
        return Ok(None);
    }
    if !existed && !img.exists {
        store.delete(a.id)?;
        return Ok(None);
    }
    a.kind = Kind::for_file(existed, img.exists);
    a.post_hash = img.hash.clone();
    a.post_size = if img.exists { img.size } else { None };
    let post_note = img
        .unstored
        .as_ref()
        .map(|why| format!("the new contents were not stored because {why}"));
    a.reversible = a.reversible && img.recoverable();
    a.note = join_note([a.note.take(), post_note, input.failure_note()]);
    a.status = Status::Recorded;
    a.end_time_ms = Some(now_ms());
    store.save(&a)?;
    Ok(Some(a.id))
}

fn post_command(
    store: &Store,
    input: &HookInput,
    agent: &str,
    cmd: &str,
    pending: Option<Action>,
) -> Result<Option<i64>> {
    let mut a = match pending {
        Some(a) => a,
        None => {
            let mut a = Action::new(agent, input.session(), Kind::Command, cmd);
            a.tool = Some(input.tool_name.clone());
            a.tool_use_id = input.tool_use_id.clone();
            a.cwd = input.cwd.clone();
            a.intent = input.intent();
            a.note = Some(
                "no pre-image: the pre hook did not run, so file changes were not recorded".into(),
            );
            a.id = store.insert(&a)?;
            a
        }
    };
    // the post hook for a background command fires once it has started, so anything it does
    // later happens after the comparison
    let background = (input
        .tool_input
        .get("run_in_background")
        .and_then(Value::as_bool)
        == Some(true))
    .then(|| {
        "the command was started in the background, so only changes made before this hook ran \
         were recorded"
            .to_string()
    });
    if let Some(mh) = a.manifest_hash.clone() {
        let manifest: Manifest = serde_json::from_slice(&store.blobs.get(&mh)?)?;
        let (n, gaps) = record_changes(store, &a, &manifest)?;
        let mut parts = vec![Some(format!(
            "{n} file change(s) recorded inside {}; changes outside it were not captured",
            manifest.root.display()
        ))];
        parts.extend(gaps.into_iter().map(Some));
        parts.push(background);
        a.note = join_note(parts);
    } else {
        a.note = join_note([a.note.take(), background]);
    }
    a.note = join_note([a.note.take(), input.failure_note()]);
    a.status = Status::Recorded;
    a.end_time_ms = Some(now_ms());
    store.save(&a)?;
    Ok(Some(a.id))
}

/// compares the working directory with the manifest taken before the command and records each
/// changed path as a child of the command. Returns the count and any reasons the record is
/// incomplete, so the command's note never claims more than was seen.
fn record_changes(store: &Store, parent: &Action, m: &Manifest) -> Result<(usize, Vec<String>)> {
    let after = snapshot::walk(&m.root, &snapshot::exclusions(store), snapshot::max_files())?;
    let mut gaps = Vec::new();
    if after.truncated {
        gaps.push(format!(
            "the directory grew past {} files, so files beyond that were not checked",
            snapshot::max_files()
        ));
    }
    let opaque_moved = snapshot::opaque_changed(&m.opaque, &after.opaque);
    // the post hook shares the agent's hook timeout, so a command that unpacked a huge tree is
    // recorded as far as time allows and the rest is reported as missing
    let deadline = std::time::Instant::now() + snapshot::snapshot_time_budget() * 3;
    let candidates = snapshot::candidates(&m.metas(), &m.links, &after);
    let total = candidates.len();
    let mut n = 0;
    for (i, path) in candidates.into_iter().enumerate() {
        if std::time::Instant::now() > deadline {
            gaps.push(format!(
                "{} changed path(s) were not recorded because the hook ran out of time",
                total - i
            ));
            break;
        }
        let before = if let Some(e) = m.files.get(&path) {
            Before::File {
                meta: e.meta,
                stored: e.hash.clone(),
            }
        } else if let Some(t) = m.links.get(&path) {
            Before::Link(t.clone())
        } else {
            Before::Absent
        };
        if let Some(c) = snapshot::change_action(store, parent, &path, before, opaque_moved)? {
            store.insert(&c)?;
            n += 1;
        }
    }
    Ok((n, gaps))
}
