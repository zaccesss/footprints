//! footprints records what AI agents do on a machine and undoes any one step.
//!
//! the ledger is a SQLite database of actions plus a content-addressed blob store. Actions
//! arrive from three capture tiers. Agent hooks know the tool and the intent. The
//! `footprints run` wrapper sees file changes made by any process it starts. The opt-in
//! `footprints daemon` sees changes made by any process at all under the roots it watches.

pub mod blob;
pub mod daemon;
pub mod diff;
pub mod export;
pub mod hook;
pub mod mcp;
pub mod output;
pub mod platform;
pub mod run;
pub mod signals;
pub mod snapshot;
pub mod store;
pub mod undo;

/// the Claude Code hook configuration, shared by `install claude-code --print` and the example
/// file so the two can never drift apart.
pub const CLAUDE_CODE_SETTINGS: &str = include_str!("../hooks/claude-code/settings.example.json");

/// the Cursor `hooks.json` printed by `install cursor --print`.
pub const CURSOR_HOOKS: &str = include_str!("../hooks/cursor/hooks.example.json");

/// the Codex `hooks.json` printed by `install codex --print`.
pub const CODEX_HOOKS: &str = include_str!("../hooks/codex/hooks.example.json");

/// the Gemini CLI `settings.json` block printed by `install gemini --print`.
pub const GEMINI_SETTINGS: &str = include_str!("../hooks/gemini-cli/settings.example.json");
