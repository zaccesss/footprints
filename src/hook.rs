//! capture tier 1: agent hook adapters.
//!
//! the pre hook stores the pre-image before a tool runs and leaves a pending action. The post
//! hook, which also handles failed calls, finds that action, stores the post-image and marks
//! it recorded.
//!
//! four agents are understood, each from its own documented payload:
//!
//! - Claude Code: PreToolUse, PostToolUse and PostToolUseFailure.
//! - OpenAI Codex CLI: PreToolUse and PostToolUse, the same shape as Claude Code. File edits
//!   arrive as `apply_patch` with the patch text in `tool_input.command`, so the file paths are
//!   read from its headers.
//! - Gemini CLI: BeforeTool and AfterTool. There is no call id, so a post hook is paired with
//!   its pre hook by session, tool and target.
//! - Cursor: preToolUse, postToolUse and postToolUseFailure for its Write and Delete tools,
//!   paired by `tool_use_id`, plus beforeShellExecution and afterShellExecution. afterFileEdit
//!   is a fallback for an edit no preToolUse saw: its pre-image is rebuilt from the edit's old
//!   and new text and only kept when re-applying the edit gives back the file exactly.

use crate::snapshot::{self, Before, Manifest};
use crate::store::{Action, Kind, Status, Store, now_ms};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const DEFAULT_AGENT: &str = "claude-code";

/// which agent wrote the payload. It decides the agent name stored by default and what the
/// hook prints on standard output, since some agents treat a missing reply as a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dialect {
    ClaudeCode,
    Cursor,
    Codex,
    Gemini,
}

impl Dialect {
    pub fn agent(self) -> &'static str {
        match self {
            Dialect::ClaudeCode => "claude-code",
            Dialect::Cursor => "cursor",
            Dialect::Codex => "codex",
            Dialect::Gemini => "gemini-cli",
        }
    }

    /// what the hook writes to standard output, printed whether or not recording worked.
    /// Cursor blocks a before hook whose output is not a valid permission reply. Gemini
    /// CLI parses standard output as JSON, so both get an explicit "carry on". Claude Code and
    /// Codex take an empty output with exit status 0 as no opinion.
    pub fn reply(self, pre: bool) -> &'static str {
        // Cursor staff report that a hook's "allow" does not override Cursor's own approval
        // prompt or allow list today:
        // https://forum.cursor.com/t/beforeshellexecution-hook-permissions-allow-ask-ignored-allow-list-takes-precedence/144244
        match self {
            Dialect::Cursor if pre => "{\"permission\":\"allow\"}\n",
            Dialect::Gemini => "{}\n",
            _ => "",
        }
    }
}

/// the documented hook payload of every supported agent. Fields not used here are ignored so
/// new versions of an agent do not break the adapter.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: Value,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    /// set on PostToolUseFailure
    #[serde(default)]
    pub error: Option<String>,
    /// Cursor's postToolUseFailure names its error this way
    #[serde(default)]
    pub error_message: Option<String>,
    /// set on PostToolUseFailure when the call was aborted rather than failing by itself
    #[serde(default)]
    pub is_interrupt: Option<bool>,
    /// Gemini CLI's AfterTool result, which carries `error` when the tool failed
    #[serde(default)]
    pub tool_response: Option<Value>,
    /// Cursor names the conversation where other agents name the session
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub workspace_roots: Vec<String>,
    /// Cursor's shell hooks give the command at the top level
    #[serde(default)]
    pub command: Option<String>,
    /// Cursor's afterFileEdit gives the path and the edits at the top level
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub old_string: String,
    #[serde(default)]
    pub new_string: String,
}

enum Target {
    File(PathBuf),
    /// an `apply_patch` call, which can touch several files at once
    Files(Vec<PathBuf>),
    Command(String),
    Network(String),
}

/// the longest a pre hook without a call id waits for its post hook. A file tool finishes in
/// moments, so a minute leaves room for an approval prompt while keeping a row left by a
/// denied call from being adopted much later.
const IDLESS_PENDING_MS: i64 = 60_000;

/// the tool name stored for Cursor's shell hooks, which carry no tool name of their own.
const CURSOR_SHELL: &str = "Shell";
/// the tool name stored for Cursor's afterFileEdit.
const CURSOR_EDIT: &str = "afterFileEdit";

impl HookInput {
    pub fn parse(json: &str) -> Result<Self> {
        let mut input: HookInput =
            serde_json::from_str(json).context("hook input is not the expected JSON payload")?;
        input.adapt();
        Ok(input)
    }

    /// fills the common fields from agents that spell them differently, so the rest of the
    /// adapter reads one shape.
    fn adapt(&mut self) {
        if self.session_id.is_none() {
            self.session_id = self.conversation_id.clone();
        }
        if self.cwd.is_none() {
            self.cwd = self.workspace_roots.first().cloned();
        }
        match self.hook_event_name.as_deref() {
            Some("beforeShellExecution" | "afterShellExecution") if self.tool_name.is_empty() => {
                self.tool_name = CURSOR_SHELL.into();
                if let Some(c) = &self.command {
                    self.tool_input = serde_json::json!({ "command": c });
                }
            }
            Some("afterFileEdit") if self.tool_name.is_empty() => {
                self.tool_name = CURSOR_EDIT.into();
            }
            _ => {}
        }
        if self.error.is_none() {
            self.error = self.error_message.clone();
        }
        if self.error.is_none()
            && let Some(e) = self
                .tool_response
                .as_ref()
                .and_then(|r| r.get("error"))
                .filter(|e| !e.is_null())
        {
            self.error = Some(match e {
                Value::String(s) => s.clone(),
                Value::Object(o) => o
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| e.to_string()),
                other => other.to_string(),
            });
        }
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

    /// the shell command as text. Some agents send it as an argument list, which is joined
    /// the way a shell would read it back.
    fn command_text(&self) -> Option<String> {
        if let Some(c) = self.str_field("command") {
            return Some(c.to_string());
        }
        let args: Vec<String> = self
            .tool_input
            .get("command")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<_>>()?;
        (!args.is_empty()).then(|| crate::run::shell_join(&args))
    }

    fn intent(&self) -> Option<String> {
        self.str_field("description").map(str::to_string)
    }

    fn target(&self) -> Option<Target> {
        match self.tool_name.as_str() {
            // Claude Code and Cursor, then Gemini CLI. Cursor's Delete also names its file in
            // `file_path`. Codex selects `apply_patch` with the Write and Edit matchers but
            // reports it by its own name; a patch under another name is still read as one
            "Write" | "Edit" | "MultiEdit" | "Delete" | "write_file" | "replace" => {
                match self.str_field("file_path") {
                    Some(p) => Some(self.file(p)),
                    None => self.patch_target(),
                }
            }
            "NotebookEdit" => self.str_field("notebook_path").map(|p| self.file(p)),
            CURSOR_EDIT => self
                .file_path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| self.file(p)),
            "apply_patch" => self.patch_target(),
            "Bash" | "PowerShell" | "run_shell_command" | CURSOR_SHELL => {
                self.command_text().map(Target::Command)
            }
            "WebFetch" => self
                .str_field("url")
                .map(|u| Target::Network(u.to_string())),
            // Gemini CLI's web_fetch takes a prompt with the URLs inside it
            "web_fetch" => self.str_field("prompt").map(|p| {
                let p: String = p.chars().take(300).collect();
                Target::Network(format!("fetch: {p}"))
            }),
            "WebSearch" | "google_web_search" => self
                .str_field("query")
                .map(|q| Target::Network(format!("search: {q}"))),
            _ => None,
        }
    }

    /// the files named in an `apply_patch` patch carried in `tool_input.command`.
    fn patch_target(&self) -> Option<Target> {
        let patch = self.str_field("command")?;
        if !patch.trim_start().starts_with("*** Begin Patch") {
            return None;
        }
        let files: Vec<PathBuf> = patch_paths(patch)
            .into_iter()
            .map(|p| self.absolute(&p))
            .collect();
        (!files.is_empty()).then_some(Target::Files(files))
    }

    /// a relative path is resolved against the payload's cwd. Without one it is resolved against
    /// this process's own directory, so the ledger never stores a path whose meaning depends on
    /// where `undo` is later run.
    fn file(&self, p: &str) -> Target {
        Target::File(self.absolute(p))
    }

    fn absolute(&self, p: &str) -> PathBuf {
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
        crate::platform::resolve_parent(&abs)
    }

    /// where a shell command runs: Gemini CLI's `dir_path` when given, otherwise the cwd.
    fn command_dir(&self) -> Option<PathBuf> {
        if let Some(d) = self.str_field("dir_path") {
            return Some(self.absolute(d));
        }
        self.cwd.as_deref().map(PathBuf::from)
    }

    fn failed(&self) -> bool {
        matches!(
            self.hook_event_name.as_deref(),
            Some("PostToolUseFailure" | "postToolUseFailure")
        ) || self.error.is_some()
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

/// the files an `apply_patch` patch adds, updates, deletes or moves to, in the order they
/// appear. Codex documents the patch only as text, so the `*** Add File:`, `*** Update File:`,
/// `*** Delete File:` and `*** Move to:` headers are the one reliable source of paths.
pub fn patch_paths(patch: &str) -> Vec<String> {
    const HEADERS: [&str; 4] = [
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        // markers may be indented; context and content lines are never read here
        let line = line.trim_end_matches('\r').trim_start();
        for h in HEADERS {
            if let Some(p) = line.strip_prefix(h) {
                let p = p.trim();
                if !p.is_empty() && !out.iter().any(|x| x == p) {
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

/// how many times `needle` occurs in `hay`, overlapping occurrences included. `str::matches`
/// skips overlaps, so `}}` in `}}}` would count once although it sits at two places. The wrong
/// one could then be replaced.
fn occurrences(hay: &str, needle: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        n += 1;
        let at = from + i;
        // restart one character past the start of this hit
        from = at + hay[at..].chars().next().map_or(1, char::len_utf8);
        if from > hay.len() {
            break;
        }
    }
    n
}

/// rebuilds the file as it was before a Cursor edit by undoing each edit, newest first, then
/// proves the result by applying the edits forward again. Every replacement must be exact and
/// unambiguous. A `new_string` that is empty or appears more than once means no pre-image.
/// So does a forward pass that does not give back the current file byte for byte.
pub fn reverse_edits(current: &str, edits: &[Edit]) -> Option<String> {
    if edits.is_empty() {
        return None;
    }
    let mut text = current.to_string();
    for e in edits.iter().rev() {
        if e.new_string.is_empty() || e.old_string == e.new_string {
            return None;
        }
        if occurrences(&text, &e.new_string) != 1 {
            return None;
        }
        text = text.replacen(e.new_string.as_str(), &e.old_string, 1);
    }
    let mut check = text.clone();
    for e in edits {
        if e.old_string.is_empty() || occurrences(&check, &e.old_string) != 1 {
            return None;
        }
        check = check.replacen(e.old_string.as_str(), &e.new_string, 1);
    }
    (check == current).then_some(text)
}

fn join_note(parts: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let v: Vec<String> = parts.into_iter().flatten().collect();
    if v.is_empty() {
        None
    } else {
        Some(v.join("; "))
    }
}

fn pre_file(store: &Store, input: &HookInput, agent: &str, path: &Path) -> Result<Action> {
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
    Ok(a)
}

/// handles a pre-tool payload. Returns the id of the pending action, the last one for a patch
/// that touches several files. A tool that changes nothing on disk gives none.
pub fn pre(store: &Store, input: &HookInput, agent: &str) -> Result<Option<i64>> {
    let Some(target) = input.target() else {
        return Ok(None);
    };
    let mut actions = match &target {
        // Cursor reports a file edit only once it has happened
        Target::File(_) if input.tool_name == CURSOR_EDIT => return Ok(None),
        Target::File(path) => vec![pre_file(store, input, agent, path)?],
        Target::Files(paths) => paths
            .iter()
            .map(|p| pre_file(store, input, agent, p))
            .collect::<Result<Vec<_>>>()?,
        Target::Command(cmd) => {
            let mut a = Action::new(agent, input.session(), Kind::Command, cmd);
            a.note = Some(
                "a shell command cannot be undone as a whole; file changes it made inside the \
                 working directory are listed as separate actions"
                    .into(),
            );
            if let Some(dir) = input.command_dir() {
                let root =
                    crate::platform::canonical(&dir).unwrap_or_else(|_| snapshot::normalise(&dir));
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
            vec![a]
        }
        Target::Network(what) => {
            let mut a = Action::new(agent, input.session(), Kind::Network, what);
            a.note = Some(format!(
                "network call sent by {}; it cannot be undone",
                input.tool_name
            ));
            vec![a]
        }
    };
    let mut last = None;
    for a in &mut actions {
        a.status = Status::Pending;
        a.tool = Some(input.tool_name.clone());
        a.tool_use_id = input.tool_use_id.clone();
        a.cwd = input.cwd.clone();
        a.intent = input.intent();
        last = Some(store.insert(a)?);
    }
    Ok(last)
}

/// handles a post-tool payload, failed calls included. Returns the id of the recorded action,
/// the last one for a patch. A tool that turned out to change nothing gives none.
pub fn post(store: &Store, input: &HookInput, agent: &str) -> Result<Option<i64>> {
    let Some(target) = input.target() else {
        return Ok(None);
    };
    let pending = |target: &str| -> Result<Option<Action>> {
        let found = store.find_pending(
            input.session_id.as_deref(),
            input.tool_use_id.as_deref(),
            &input.tool_name,
            Some(target),
        )?;
        // without a call id the pairing rests on session, tool and target alone, so a row left
        // by a denied call could be adopted by a later one. A row that is old or that a later
        // record of the same path has overtaken holds a pre-image that may predate other
        // edits, so it is not used
        Ok(found.filter(|a| {
            a.tool_use_id.is_some()
                || (now_ms() - a.time_ms <= IDLESS_PENDING_MS
                    && !store.newer_for_target(&a.target, a.id).unwrap_or(true))
        }))
    };
    match target {
        Target::File(path) if input.tool_name == CURSOR_EDIT => {
            cursor_edit(store, input, agent, &path)
        }
        Target::File(path) => {
            let mut p = pending(&path.to_string_lossy())?;
            if let Some(a) = &p
                && a.tool_use_id.is_none()
                && !replace_explains(store, input, a, &path)
            {
                // the pending pre-image is not the file this call started from
                p = None;
            }
            post_file(store, input, agent, &path, p)
        }
        Target::Files(paths) => {
            let mut last = None;
            for path in paths {
                let p = pending(&path.to_string_lossy())?;
                if let Some(id) = post_file(store, input, agent, &path, p)? {
                    last = Some(id);
                }
            }
            Ok(last)
        }
        Target::Command(cmd) => {
            let p = pending(&cmd)?;
            post_command(store, input, agent, &cmd, p)
        }
        Target::Network(what) => {
            let mut a = match pending(&what)? {
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

/// records a Cursor afterFileEdit. The pre-image is rebuilt from the edits when that can be
/// proved exact. Otherwise the change is recorded as not reversible.
/// for a Gemini CLI `replace` paired without a call id, true only when applying the call's own
/// replacement to the pending pre-image gives exactly the file now on disk. That proves the
/// pre-image is the file the call started from, so a row left by an earlier denied call is
/// never adopted by mistake. Other tools carry nothing to check, so they pass.
fn replace_explains(store: &Store, input: &HookInput, pending: &Action, path: &Path) -> bool {
    if input.tool_name != "replace" {
        return true;
    }
    let (Some(old), Some(new)) = (input.str_field("old_string"), input.str_field("new_string"))
    else {
        return false;
    };
    let Some(before) = pending
        .pre_hash
        .as_deref()
        .and_then(|h| store.blobs.get(h).ok())
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return false;
    };
    let all = input
        .tool_input
        .get("allow_multiple")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let after = if all {
        before.replace(old, new)
    } else {
        if occurrences(&before, old) != 1 {
            return false;
        }
        before.replacen(old, new, 1)
    };
    std::fs::read(path).is_ok_and(|now| now == after.as_bytes())
}

/// how long after a Cursor preToolUse or postToolUse record an afterFileEdit for the same
/// file is taken as the same edit reported twice.
const CURSOR_FALLBACK_MS: i64 = 60_000;

fn cursor_edit(store: &Store, input: &HookInput, agent: &str, path: &Path) -> Result<Option<i64>> {
    // afterFileEdit is only a fallback: when preToolUse and postToolUse already recorded this
    // file for the conversation, the edit has its real pre-image and call id there
    if let Some(prev) = store.latest_for_target(&path.to_string_lossy())?
        && prev.session == input.session()
        && prev.tool.as_deref() != Some(CURSOR_EDIT)
        && now_ms() - prev.end_time_ms.unwrap_or(prev.time_ms) <= CURSOR_FALLBACK_MS
    {
        return Ok(None);
    }
    let img = snapshot::capture(store, path)?;
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
    a.post_hash = img.hash.clone();
    a.post_size = if img.exists { img.size } else { None };
    a.tool = Some(input.tool_name.clone());
    a.cwd = input.cwd.clone();
    let rebuilt = match (&img.hash, img.unstored.is_none()) {
        (Some(h), true) => store
            .blobs
            .get(h)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|cur| reverse_edits(&cur, &input.edits)),
        _ => None,
    };
    match rebuilt {
        Some(before) => {
            a.pre_hash = Some(store.blobs.put(before.as_bytes())?);
            a.pre_size = Some(before.len() as i64);
            a.pre_mode = img.mode;
            a.reversible = true;
            a.note = Some(
                "Cursor reports file edits only after they happen; the pre-image was rebuilt from \
                 the edit's old and new text and checked by applying the edit again"
                    .into(),
            );
        }
        None => {
            a.note = Some(
                "no pre-image: Cursor reports file edits only after they happen and this edit \
                 could not be reversed exactly from its old and new text"
                    .into(),
            );
        }
    }
    a.end_time_ms = Some(now_ms());
    Ok(Some(store.insert(&a)?))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn e(old: &str, new: &str) -> Edit {
        Edit {
            old_string: old.into(),
            new_string: new.into(),
        }
    }

    #[test]
    fn patch_paths_reads_every_header_once() {
        let p = "*** Begin Patch\r\n*** Update File: a.rs\r\n@@\n-x\n+y\n*** Add File: b/c.rs\n+z\n\
                 *** Delete File: d.rs\n*** Update File: e.rs\n*** Move to: f.rs\n\
                 *** Update File: a.rs\n*** End Patch\n";
        assert_eq!(patch_paths(p), ["a.rs", "b/c.rs", "d.rs", "e.rs", "f.rs"]);
        assert!(patch_paths("no headers here").is_empty());
        // indented markers are read; content lines that only look like markers are not
        assert_eq!(
            patch_paths("  *** Begin Patch\n\t*** Add File: x.rs\n+*** Delete File: no.rs\n"),
            ["x.rs"]
        );
    }

    #[test]
    fn reverse_edits_undoes_several_edits_in_order() {
        // the second edit changes text the first one wrote
        let edits = [e("alpha", "beta"), e("beta two", "gamma")];
        let original = "alpha two\nend\n";
        let current = "gamma\nend\n";
        assert_eq!(reverse_edits(current, &edits).as_deref(), Some(original));
    }

    #[test]
    fn reverse_edits_refuses_anything_ambiguous() {
        assert_eq!(reverse_edits("x x", &[e("y", "x")]), None, "two matches");
        assert_eq!(reverse_edits("abc", &[e("a", "")]), None, "empty new text");
        assert_eq!(reverse_edits("abc", &[]), None, "no edits");
        assert_eq!(reverse_edits("abc", &[e("q", "z")]), None, "not present");
        // undoing works but applying again finds the old text twice
        assert_eq!(reverse_edits("ab", &[e("a", "b")]), None);
    }

    #[test]
    fn reverse_edits_counts_overlapping_matches() {
        // the new text overlaps another copy of its own tail, so it sits at two places
        let original = "fn a() {\n    }\n    X\n";
        let current = original.replacen('X', "}\n    }", 1);
        assert_eq!(reverse_edits(&current, &[e("X", "}\n    }")]), None);
        assert_eq!(occurrences("}}}", "}}"), 2);
        assert_eq!(occurrences("aé aé", "é"), 2);
    }
}
