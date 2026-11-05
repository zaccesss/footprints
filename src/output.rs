//! human-readable output. Every piece of meaning is carried by text, so colour is only ever
//! an extra and the output reads the same with `NO_COLOR` set or through a screen reader.

use crate::store::{Action, Kind, Status};
use std::io::IsTerminal;

/// colour is used only on a terminal, never when `NO_COLOR` is set to anything (see
/// https://no-color.org) and never for `TERM=dumb`.
pub fn use_colour(disabled_by_flag: bool) -> bool {
    if disabled_by_flag {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
}

pub struct Paint {
    pub on: bool,
}

impl Paint {
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn added(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub fn removed(&self, s: &str) -> String {
        self.wrap("31", s)
    }
    pub fn info(&self, s: &str) -> String {
        self.wrap("36", s)
    }
    pub fn warn(&self, s: &str) -> String {
        self.wrap("33", s)
    }
}

pub fn colour_diff(text: &str, p: &Paint) -> String {
    if !p.on {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let painted = if body.starts_with("+++") || body.starts_with("---") {
            p.bold(body)
        } else if body.starts_with('+') {
            p.added(body)
        } else if body.starts_with('-') {
            p.removed(body)
        } else if body.starts_with("@@") {
            p.info(body)
        } else {
            body.to_string()
        };
        out.push_str(&painted);
        out.push_str(nl);
    }
    out
}

pub fn format_time(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

/// a short state word that says whether the action can be undone.
pub fn state(a: &Action) -> &'static str {
    match a.status {
        Status::Undone => "undone",
        Status::Pending => "pending",
        Status::Recorded if a.reversible => "reversible",
        Status::Recorded => "not reversible",
    }
}

fn clip(s: &str, max: usize) -> String {
    let one_line = s.replace(['\n', '\r', '\t'], " ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let mut t: String = one_line.chars().take(max.saturating_sub(3)).collect();
        t.push_str("...");
        t
    }
}

fn display_target(a: &Action) -> String {
    let mut t = if a.kind.is_file() {
        tilde(&a.target)
    } else {
        clip(&a.target, 72)
    };
    if let Some(p) = a.parent_id {
        t.push_str(&format!(" (via #{p})"));
    }
    if let Some(u) = a.undo_of {
        t.push_str(&format!(" (undo of #{u})"));
    }
    t
}

/// paths under the current directory are shown relative to it, which is how they were most
/// likely typed; anything else keeps its full path with the home directory shortened to `~`.
fn tilde(path: &str) -> String {
    let p = std::path::Path::new(path);
    if let Ok(cwd) = std::env::current_dir() {
        let canon = crate::platform::canonical(&cwd).unwrap_or(cwd);
        if let Ok(rest) = p.strip_prefix(&canon)
            && !rest.as_os_str().is_empty()
        {
            return rest.to_string_lossy().into_owned();
        }
    }
    if let Some(h) = crate::platform::home_dir()
        && let Ok(rest) = p.strip_prefix(&h)
        && !rest.as_os_str().is_empty()
    {
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.to_string_lossy());
    }
    path.to_string()
}

/// the timeline as an aligned table. Columns are padded with spaces only, so the table also
/// reads correctly in a braille display or a screen reader's line mode.
pub fn log_table(actions: &[Action], p: &Paint) -> String {
    if actions.is_empty() {
        return "No actions recorded yet.\n".into();
    }
    let headers = ["ID", "TIME", "AGENT", "SESSION", "KIND", "STATE", "TARGET"];
    let rows: Vec<[String; 7]> = actions
        .iter()
        .map(|a| {
            [
                a.id.to_string(),
                format_time(a.time_ms),
                clip(&a.agent, 16),
                clip(&a.session, 16),
                a.kind.to_string(),
                state(a).to_string(),
                display_target(a),
            ]
        })
        .collect();
    let mut widths = headers.map(str::len);
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let mut out = String::new();
    let header: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| pad(h, widths[i], i == 0))
        .collect();
    out.push_str(&p.bold(header.join("  ").trim_end()));
    out.push('\n');
    for (r, a) in rows.iter().zip(actions) {
        let cells: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let padded = pad(c, widths[i], i == 0);
                if i == 5 {
                    match a.status {
                        Status::Recorded if !a.reversible => p.warn(&padded),
                        _ => padded,
                    }
                } else {
                    padded
                }
            })
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

fn pad(s: &str, w: usize, right: bool) -> String {
    if right {
        format!("{s:>w$}")
    } else {
        format!("{s:<w$}")
    }
}

/// every field of one action as `label: value` lines.
pub fn show(a: &Action, children: &[Action]) -> String {
    let mut lines: Vec<(String, String)> = vec![
        ("id".into(), a.id.to_string()),
        ("time".into(), format_time(a.time_ms)),
    ];
    if let Some(e) = a.end_time_ms {
        lines.push(("finished".into(), format_time(e)));
    }
    lines.push(("agent".into(), a.agent.clone()));
    lines.push(("session".into(), a.session.clone()));
    lines.push(("kind".into(), a.kind.to_string()));
    lines.push(("state".into(), state(a).to_string()));
    let label = match a.kind {
        Kind::Command => "command",
        Kind::Network => "request",
        _ => "path",
    };
    lines.push((label.into(), a.target.clone()));
    if let Some(i) = &a.intent {
        lines.push(("intent".into(), i.clone()));
    }
    if let Some(t) = &a.tool {
        lines.push(("tool".into(), t.clone()));
    }
    if a.kind.is_file() {
        lines.push(("pre-image".into(), image(&a.pre_hash, a.pre_size)));
        lines.push(("post-image".into(), image(&a.post_hash, a.post_size)));
    }
    if let Some(c) = &a.cwd {
        lines.push(("directory".into(), c.clone()));
    }
    if let Some(p) = a.parent_id {
        lines.push(("part of".into(), format!("#{p}")));
    }
    if let Some(u) = a.undo_of {
        lines.push(("undo of".into(), format!("#{u}")));
    }
    if let Some(u) = a.undone_by {
        lines.push(("undone by".into(), format!("#{u}")));
    }
    if let Some(n) = &a.note {
        lines.push(("note".into(), n.clone()));
    }
    if !children.is_empty() {
        let ids: Vec<String> = children
            .iter()
            .map(|c| format!("#{} {} {}", c.id, c.kind, tilde(&c.target)))
            .collect();
        lines.push(("file changes".into(), ids.join("\n")));
    }
    let w = lines.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 1;
    lines
        .into_iter()
        .map(|(l, v)| {
            let v = v.replace('\n', &format!("\n{:w$} ", ""));
            format!("{:<w$} {v}\n", format!("{l}:"))
        })
        .collect()
}

fn image(hash: &Option<String>, size: Option<i64>) -> String {
    match (hash, size) {
        (Some(h), Some(s)) => format!("blake3 {h} ({s} bytes)"),
        (None, Some(s)) => format!("not stored ({s} bytes; the note says why)"),
        _ => "none (file did not exist)".into(),
    }
}
