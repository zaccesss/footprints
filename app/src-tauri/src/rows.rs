//! the data behind the window, kept apart from Tauri so it can be tested on its own.

use anyhow::Result;
use chrono::{Local, TimeZone};
use footprints::store::{Action, Filter, Status, Store};
use footprints::undo::{self, UndoOpts};
use serde::Serialize;

/// how many recent actions are read to find today's; a busy day beyond this shows the newest.
const READ_LIMIT: usize = 5000;

#[derive(Debug, Serialize)]
pub struct Row {
    pub id: i64,
    /// local time, 24 hour clock
    pub time: String,
    pub agent: String,
    pub kind: String,
    pub target: String,
    /// "can be undone", "undone", "not reversible" or "waiting for the tool"
    pub state: String,
    pub can_undo: bool,
    pub note: Option<String>,
    /// set on the record an undo leaves behind, so the window can offer to take it back
    pub undo_of: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Today {
    pub rows: Vec<Row>,
    /// a plain sentence for the top of the window, such as "Agents changed 14 things today."
    pub summary: String,
    pub ledger: String,
}

fn state(a: &Action) -> (&'static str, bool) {
    match a.status {
        Status::Undone => ("undone", false),
        Status::Pending => ("waiting for the tool", false),
        Status::Recorded if a.kind.is_file() && a.reversible => ("can be undone", true),
        Status::Recorded => ("not reversible", false),
    }
}

fn row(a: &Action) -> Row {
    let (state, can_undo) = state(a);
    let time = Local
        .timestamp_millis_opt(a.time_ms)
        .single()
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default();
    Row {
        id: a.id,
        time,
        agent: a.agent.clone(),
        kind: a.kind.as_str().replace('_', " "),
        target: a.target.clone(),
        state: state.into(),
        can_undo,
        note: a.note.clone(),
        undo_of: a.undo_of,
    }
}

/// local midnight in milliseconds since the epoch.
fn start_of_today() -> i64 {
    let now = Local::now();
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|m| Local.from_local_datetime(&m).earliest())
        .map(|m| m.timestamp_millis())
        .unwrap_or(0)
}

pub fn today_from(store: &Store, since_ms: i64) -> Result<Today> {
    let actions = store.list(&Filter {
        limit: Some(READ_LIMIT),
        ..Default::default()
    })?;
    // newest first, which is what a person checking "what just happened" wants to see. An
    // undo's own record is listed too, so a click on undo can itself be taken back here
    let rows: Vec<Row> = actions
        .iter()
        .rev()
        .filter(|a| a.time_ms >= since_ms)
        .map(row)
        .collect();
    let changes = rows
        .iter()
        .filter(|r| r.kind.starts_with("file") && r.undo_of.is_none())
        .count();
    let summary = match changes {
        0 if rows.is_empty() => "Nothing recorded today.".to_string(),
        0 => format!("{} action(s) today, none of them file changes.", rows.len()),
        1 => "Agents changed 1 file today.".to_string(),
        n => format!("Agents changed {n} files today."),
    };
    Ok(Today {
        rows,
        summary,
        ledger: store.home().display().to_string(),
    })
}

pub fn today() -> Result<Today> {
    let store = Store::open_default()?;
    today_from(&store, start_of_today())
}

pub fn undo_in(store: &Store, id: i64) -> Result<String> {
    // the same conflict check as the command line, never an overwrite. The conflict message
    // does not suggest --force, since the window has no way to use it
    let opts = UndoOpts {
        offer_force: false,
        ..UndoOpts::cli(false)
    };
    let u = undo::undo_one(store, id, &opts)?;
    Ok(u.to_string())
}

pub fn undo(id: i64) -> Result<String> {
    let store = Store::open_default()?;
    undo_in(&store, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use footprints::store::Kind;
    use std::fs;

    fn record(store: &Store, path: &std::path::Path, before: &[u8], after: &[u8]) -> i64 {
        let mut a = Action::new("tester", "s", Kind::FileWrite, &path.to_string_lossy());
        a.pre_hash = Some(store.blobs.put(before).unwrap());
        a.pre_size = Some(before.len() as i64);
        a.post_hash = Some(store.blobs.put(after).unwrap());
        a.post_size = Some(after.len() as i64);
        a.reversible = true;
        store.insert(&a).unwrap()
    }

    #[test]
    fn today_lists_changes_newest_first_and_undo_uses_the_conflict_check() {
        let d = tempfile::tempdir().unwrap();
        // the ledger stores resolved paths. macOS keeps temp folders behind a symlink
        let root = footprints::platform::canonical(d.path()).unwrap();
        let store = Store::open(&root.join("home")).unwrap();
        let f = root.join("f.txt");
        fs::write(&f, "after\n").unwrap();
        let first = record(&store, &f, b"before\n", b"after\n");
        let g = root.join("g.txt");
        fs::write(&g, "someone else\n").unwrap();
        let second = record(&store, &g, b"g0\n", b"g1\n");

        let t = today_from(&store, 0).unwrap();
        assert_eq!(t.rows[0].id, second);
        assert_eq!(t.rows[1].id, first);
        assert!(t.rows.iter().all(|r| r.can_undo));
        assert_eq!(t.summary, "Agents changed 2 files today.");

        let done = undo_in(&store, first).unwrap();
        assert!(done.contains("restored"), "{done}");
        assert_eq!(fs::read_to_string(&f).unwrap(), "before\n");
        // g changed since it was recorded, so the undo is refused and g is left alone
        let err = undo_in(&store, second).unwrap_err().to_string();
        assert!(err.contains("conflict"), "{err}");
        assert_eq!(fs::read_to_string(&g).unwrap(), "someone else\n");

        let t = today_from(&store, 0).unwrap();
        assert_eq!(t.rows.len(), 3, "the undo's own record is listed");
        assert_eq!(t.summary, "Agents changed 2 files today.");
        let undo_row = &t.rows[0];
        assert_eq!(undo_row.undo_of, Some(first));
        assert!(
            undo_row.can_undo,
            "an undo from the window can be taken back"
        );
        assert_eq!(t.rows[2].state, "undone");

        // taking the undo back puts the agent's change back
        undo_in(&store, undo_row.id).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "after\n");
        let t = today_from(&store, 0).unwrap();
        let original = t.rows.iter().find(|r| r.id == first).unwrap();
        assert!(original.can_undo, "the original change can be undone again");
    }

    #[test]
    fn an_empty_ledger_says_so() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(&d.path().join("home")).unwrap();
        assert_eq!(
            today_from(&store, 0).unwrap().summary,
            "Nothing recorded today."
        );
    }
}
