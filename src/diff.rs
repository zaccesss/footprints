//! unified diffs from a recorded pre-image to its post-image.

use crate::store::{Action, Store};
use anyhow::Result;
use similar::TextDiff;

enum Side {
    Absent,
    NotStored,
    Bytes(Vec<u8>),
}

fn side(store: &Store, hash: &Option<String>, size: Option<i64>) -> Result<Side> {
    Ok(match (hash, size) {
        (Some(h), _) => Side::Bytes(store.blobs.get(h)?),
        (None, Some(_)) => Side::NotStored,
        (None, None) => Side::Absent,
    })
}

fn is_binary(b: &[u8]) -> bool {
    b.contains(&0) || std::str::from_utf8(b).is_err()
}

/// returns the diff as plain text. Colour is added by the caller, so the same text works in a
/// pipe, a file and the MCP server.
pub fn diff(store: &Store, a: &Action) -> Result<String> {
    if !a.kind.is_file() {
        return Ok(format!(
            "#{} is a {}, not a file change, so there is nothing to diff. Target: {}\n",
            a.id, a.kind, a.target
        ));
    }
    let old = side(store, &a.pre_hash, a.pre_size)?;
    let new = side(store, &a.post_hash, a.post_size)?;
    // git-style names use forward slashes, so a Windows path reads `a/C:/work/f.txt`
    let path = if cfg!(windows) {
        a.target.replace('\\', "/")
    } else {
        a.target.clone()
    };
    let path = path.trim_start_matches('/');
    let old_name = match old {
        Side::Absent => "/dev/null".to_string(),
        _ => format!("a/{path}"),
    };
    let new_name = match new {
        Side::Absent => "/dev/null".to_string(),
        _ => format!("b/{path}"),
    };
    let (old_b, new_b) = match (old, new) {
        (Side::NotStored, _) | (_, Side::NotStored) => {
            return Ok(format!(
                "#{}: {} changed, but one side was not stored, so no diff is available. {}\n",
                a.id,
                a.target,
                a.note.as_deref().unwrap_or("")
            ));
        }
        (o, n) => (bytes(o), bytes(n)),
    };
    if is_binary(&old_b) || is_binary(&new_b) {
        return Ok(format!(
            "Binary files {old_name} and {new_name} differ ({} bytes to {} bytes)\n",
            old_b.len(),
            new_b.len()
        ));
    }
    let old_s = String::from_utf8(old_b).expect("checked utf-8");
    let new_s = String::from_utf8(new_b).expect("checked utf-8");
    if old_s == new_s {
        return Ok(format!(
            "#{}: contents are identical (only metadata changed)\n",
            a.id
        ));
    }
    let td = TextDiff::from_lines(&old_s, &new_s);
    let mut out = td
        .unified_diff()
        .context_radius(3)
        .missing_newline_hint(true)
        .header(&old_name, &new_name)
        .to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn bytes(s: Side) -> Vec<u8> {
    match s {
        Side::Bytes(b) => b,
        _ => Vec::new(),
    }
}
