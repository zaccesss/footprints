//! footprints records what AI agents do on a machine and undoes any one step.
//!
//! the ledger is a SQLite database of actions plus a content-addressed blob store. Actions
//! arrive from two capture tiers. Agent hooks know the tool and the intent. The
//! `footprints run` wrapper sees file changes made by any process it starts.

pub mod blob;
pub mod diff;
pub mod export;
pub mod hook;
pub mod mcp;
pub mod output;
pub mod run;
pub mod snapshot;
pub mod store;
pub mod undo;

/// the Claude Code hook configuration, shared by `install claude-code --print` and the example
/// file so the two can never drift apart.
pub const CLAUDE_CODE_SETTINGS: &str = include_str!("../hooks/claude-code/settings.example.json");
