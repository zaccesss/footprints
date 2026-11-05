use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Parser, Subcommand};
use footprints::output::{self, Paint};
use footprints::store::{Filter, Kind, Store};
use footprints::undo::{self, Refused};
use footprints::{daemon, diff, export, hook, mcp, prune, run};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

/// exit status for an undo that was refused on purpose, kept apart from 1 so scripts can tell
/// "nothing was changed because of a conflict" from a crash.
const EXIT_REFUSED: u8 = 3;

#[derive(Parser)]
#[command(
    name = "footprints",
    version,
    about = "See and undo what AI agents do on your machine",
    long_about = "See and undo what AI agents do on your machine.\n\n\
        Footprints keeps a local ledger of file writes, shell commands and network calls made by \
        AI agents, with a stored copy of every file before and after each change. Any one file \
        change can be undone. An undo refuses if the file changed since.\n\n\
        Data lives in $FOOTPRINTS_HOME when it is set, otherwise in ~/.local/share/footprints \
        on macOS and Linux and in %LOCALAPPDATA%\\footprints on Windows."
)]
struct Cli {
    /// never print colour, even on a terminal (NO_COLOR is also respected)
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// show the timeline of recorded actions, oldest first
    Log {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        /// file_write, file_create, file_delete, command or network
        #[arg(long)]
        kind: Option<String>,
        /// show the most recent N actions
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        /// show every action, ignoring --limit
        #[arg(long)]
        all: bool,
        /// print JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// show every recorded field of one action
    Show {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// show a unified diff from an action's pre-image to its post-image
    Diff { id: i64 },
    /// restore a file's pre-image or delete a file the agent created
    Undo {
        #[arg(required_unless_present = "session", conflicts_with = "session")]
        id: Option<i64>,
        /// undo every file action in a session, newest first
        #[arg(long)]
        session: Option<String>,
        /// undo even when the file changed since it was recorded
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    /// export the ledger in another format
    Export {
        /// OpenTelemetry GenAI-style spans as OTLP JSON
        #[arg(long)]
        otel: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        /// write to this file instead of standard output
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },
    /// agent hook adapter: reads a hook payload as JSON on standard input
    Hook {
        #[command(subcommand)]
        phase: HookPhase,
        /// which agent sent the payload
        #[arg(long, global = true, value_enum, default_value = "claude-code")]
        from: hook::Dialect,
        /// the agent name stored with each action (defaults to the --from agent)
        #[arg(long, global = true)]
        agent: Option<String>,
    },
    /// print the configuration that connects an agent to footprints
    Install {
        #[command(subcommand)]
        target: InstallTarget,
    },
    /// run any command and record every file it creates, changes or deletes
    Run {
        /// agent name for the ledger (defaults to the program name)
        #[arg(long)]
        agent: Option<String>,
        /// session name (defaults to a new one per run)
        #[arg(long)]
        session: Option<String>,
        /// directory to watch and run the command in (defaults to the current directory)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// what the run is meant to do, stored with every action
        #[arg(long)]
        intent: Option<String>,
        /// the command to run, after `--`
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// serve the ledger to agents as an MCP server over stdio
    Mcp,
    /// record changes made by any process under the given roots, until stopped
    Daemon {
        /// a directory to watch; repeat for several
        #[arg(long = "root", required = true)]
        roots: Vec<PathBuf>,
        /// copy every file under the roots first, so the first change to each is reversible
        #[arg(long)]
        snapshot: bool,
        /// stop after this many seconds instead of waiting for Ctrl+C
        #[arg(long)]
        seconds: Option<u64>,
        /// Linux only: also read fanotify to name the writing process (needs CAP_SYS_ADMIN)
        #[arg(long)]
        fanotify: bool,
        /// Windows only: also read the volume's USN change journal (needs administrator rights)
        #[arg(long)]
        usn: bool,
    },
    /// record the changes of a `footprints run` that was killed before it could
    Recover,
    /// remove old actions and the stored file copies only they use
    ///
    /// Without --yes nothing is changed: the actions, sessions and bytes that would go are
    /// printed. With --yes the pruned actions and every file copy they use are written as one
    /// archive folder, footprints-pruned-<date>-<n>, which is moved to the Trash (the Recycle
    /// Bin on Windows). `footprints import` puts an archive back. Selectors combine: an action
    /// is pruned only when it matches every one given. A command and the file changes under it
    /// are pruned together. Sessions still being recorded are never pruned. Prune refuses
    /// while any other footprints process has the ledger open. Prune is not offered over MCP,
    /// so an agent can never erase its own trail.
    #[command(
        override_usage = "footprints prune <SELECTOR>... [--yes] [--permanent] [--json]",
        group(
        ArgGroup::new("selector")
            .required(true)
            .multiple(true)
            .args(["older_than", "before", "session", "agent", "all", "max_size"])
        )
    )]
    Prune {
        /// actions that finished longer ago than this, such as 30d, 12h or 2w
        #[arg(long, value_name = "DURATION", value_parser = prune::parse_duration)]
        older_than: Option<i64>,
        /// actions that finished before this local date (YYYY-MM-DD) or RFC 3339 time
        #[arg(long, value_name = "DATE", value_parser = prune::parse_date)]
        before: Option<i64>,
        /// actions in this session
        #[arg(long)]
        session: Option<String>,
        /// actions recorded for this agent
        #[arg(long)]
        agent: Option<String>,
        /// every action
        #[arg(long)]
        all: bool,
        /// prune the oldest sessions first until the data folder fits in this size, such as
        /// 500MB or 2GiB
        #[arg(long, value_name = "SIZE", value_parser = prune::parse_size)]
        max_size: Option<u64>,
        /// prune for real; without it nothing is changed
        #[arg(long)]
        yes: bool,
        /// delete outright instead of moving an archive to the Trash
        #[arg(long)]
        permanent: bool,
        /// print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// restore the actions in an archive made by `footprints prune`
    ///
    /// Every action keeps its original id. The archive is read and left as it is. Import
    /// refuses when an id in the archive is already in the ledger, which means it was
    /// restored before.
    Import {
        /// the archive folder, taken out of the Trash first
        archive: PathBuf,
        /// print JSON instead of text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum HookPhase {
    /// PreToolUse: store the pre-image
    Pre,
    /// PostToolUse: store the post-image
    Post,
}

#[derive(Subcommand)]
enum InstallTarget {
    /// Claude Code hooks for settings.json
    ClaudeCode {
        /// print the snippet (the only mode in this version; nothing is written)
        #[arg(long)]
        print: bool,
    },
    /// Cursor hooks for hooks.json
    Cursor {
        /// print the snippet (the only mode in this version; nothing is written)
        #[arg(long)]
        print: bool,
    },
    /// OpenAI Codex CLI hooks for hooks.json
    Codex {
        /// print the snippet (the only mode in this version; nothing is written)
        #[arg(long)]
        print: bool,
    },
    /// Gemini CLI hooks for settings.json
    Gemini {
        /// print the snippet (the only mode in this version; nothing is written)
        #[arg(long)]
        print: bool,
    },
}

impl InstallTarget {
    fn parts(&self) -> (bool, &'static str, &'static str, &'static str) {
        match self {
            InstallTarget::ClaudeCode { print } => (
                *print,
                "claude-code",
                "~/.claude/settings.json or .claude/settings.json",
                footprints::CLAUDE_CODE_SETTINGS,
            ),
            InstallTarget::Cursor { print } => (
                *print,
                "cursor",
                "~/.cursor/hooks.json or .cursor/hooks.json",
                footprints::CURSOR_HOOKS,
            ),
            InstallTarget::Codex { print } => (
                *print,
                "codex",
                "~/.codex/hooks.json or .codex/hooks.json",
                footprints::CODEX_HOOKS,
            ),
            InstallTarget::Gemini { print } => (
                *print,
                "gemini",
                "~/.gemini/settings.json or .gemini/settings.json",
                footprints::GEMINI_SETTINGS,
            ),
        }
    }
}

fn main() -> ExitCode {
    // every file footprints creates holds or describes copies of user files, so none of them
    // should ever be readable by other accounts
    #[cfg(unix)]
    // SAFETY: umask only changes this process's file creation mask
    unsafe {
        libc::umask(0o077);
    }
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // exit status 2 from a PreToolUse hook blocks the tool call, so a bad hook
            // argument in an agent's settings is reported without stopping the agent
            let args: Vec<String> = std::env::args().collect();
            if args.iter().skip(1).any(|a| a == "hook") {
                eprintln!("footprints: hook error: {e}");
                // the agent is still writing the payload; reading it to the end first means
                // the agent never sees a closed pipe and the reply below always arrives
                let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
                // the safe reply for whichever agent this looks like, so a typo in its
                // settings never blocks a tool call
                let pre = args.iter().any(|a| a == "pre");
                let dialect = if args.iter().any(|a| a.contains("cursor")) {
                    hook::Dialect::Cursor
                } else if args.iter().any(|a| a.contains("gemini")) {
                    hook::Dialect::Gemini
                } else {
                    hook::Dialect::ClaudeCode
                };
                print!("{}", dialect.reply(pre));
                return ExitCode::SUCCESS;
            }
            e.exit();
        }
    };
    // a failing hook must never block the agent's tool call, so hook errors are reported on
    // stderr and the exit status stays 0
    if let Cmd::Hook { phase, from, agent } = &cli.cmd {
        let agent = agent.as_deref().unwrap_or(from.agent());
        // a panic is caught too, so the reply below is always printed
        match std::panic::catch_unwind(|| run_hook(phase, agent)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("footprints: hook error: {e:#}"),
            Err(_) => eprintln!("footprints: hook error: internal panic"),
        }
        // printed even after an error, since Cursor blocks a before hook without a reply
        print!("{}", from.reply(matches!(phase, HookPhase::Pre)));
        return ExitCode::SUCCESS;
    }
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            if let Some(r) = e.downcast_ref::<Refused>() {
                eprintln!("footprints: refused: {r}");
                ExitCode::from(EXIT_REFUSED)
            } else {
                eprintln!("footprints: error: {e:#}");
                ExitCode::FAILURE
            }
        }
    }
}

fn run_hook(phase: &HookPhase, agent: &str) -> Result<()> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let input = hook::HookInput::parse(&buf)?;
    let store = Store::open_default()?;
    match phase {
        HookPhase::Pre => hook::pre(&store, &input, agent)?,
        HookPhase::Post => hook::post(&store, &input, agent)?,
    };
    Ok(())
}

/// a busy ledger is a deliberate refusal, so it exits 3 like a refused undo.
fn busy_is_refusal(e: anyhow::Error) -> anyhow::Error {
    if e.is::<footprints::store::Busy>() {
        Refused(e.to_string()).into()
    } else {
        e
    }
}

fn print_json(v: &impl serde::Serialize) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, v)?;
    writeln!(out)?;
    Ok(())
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    let paint = Paint {
        on: output::use_colour(cli.no_color),
    };
    match cli.cmd {
        Cmd::Log {
            session,
            agent,
            kind,
            limit,
            all,
            json,
        } => {
            let store = Store::open_default()?;
            let kind = kind.map(|k| k.parse::<Kind>()).transpose()?;
            let actions = store.list(&Filter {
                session,
                agent,
                kind,
                limit: if all { None } else { Some(limit) },
            })?;
            if json {
                print_json(&actions)?;
            } else {
                print!("{}", output::log_table(&actions, &paint));
            }
        }
        Cmd::Show { id, json } => {
            let store = Store::open_default()?;
            let a = store.require(id)?;
            if json {
                print_json(&a)?;
            } else {
                let children = store.children(id)?;
                print!("{}", output::show(&a, &children));
            }
        }
        Cmd::Diff { id } => {
            let store = Store::open_default()?;
            let a = store.require(id)?;
            print!("{}", output::colour_diff(&diff::diff(&store, &a)?, &paint));
        }
        Cmd::Undo {
            id,
            session,
            force,
            json,
        } => {
            let store = Store::open_default()?;
            let opts = undo::UndoOpts::cli(force);
            match (id, session) {
                (Some(id), _) => {
                    let u = undo::undo_one(&store, id, &opts)?;
                    if json {
                        print_json(&u)?;
                    } else {
                        println!("{u}");
                    }
                }
                (None, Some(s)) => {
                    let r = undo::undo_session(&store, &s, &opts)?;
                    if json {
                        print_json(&r)?;
                    } else {
                        for u in &r.done {
                            println!("{u}");
                        }
                        for (_, why) in &r.skipped {
                            println!("skipped: {why}");
                        }
                        println!(
                            "{} action(s) undone, {} skipped. To reverse this undo: footprints undo --session {}",
                            r.done.len(),
                            r.skipped.len(),
                            r.session
                        );
                    }
                }
                (None, None) => unreachable!("clap requires an id or --session"),
            }
        }
        Cmd::Export {
            otel,
            session,
            agent,
            output,
        } => {
            if !otel {
                bail!("choose an export format; --otel is the only one in this version");
            }
            let store = Store::open_default()?;
            let actions = store.list(&Filter {
                session,
                agent,
                ..Default::default()
            })?;
            let doc = serde_json::to_string_pretty(&export::otel(&actions))?;
            match output {
                Some(p) => std::fs::write(&p, doc + "\n")
                    .with_context(|| format!("writing {}", p.display()))?,
                None => println!("{doc}"),
            }
        }
        Cmd::Hook { .. } => unreachable!("handled in main"),
        Cmd::Install { target } => {
            let (print, name, place, snippet) = target.parts();
            if !print {
                bail!(
                    "this version only prints the snippet and never edits agent settings. Run \
                     `footprints install {name} --print` and merge the output into {place} \
                     yourself"
                );
            }
            print!("{snippet}");
        }
        Cmd::Run {
            agent,
            session,
            dir,
            intent,
            command,
        } => {
            let store = Store::open_default()?;
            let outcome = run::run(
                &store,
                &run::RunOptions {
                    agent,
                    session,
                    dir,
                    intent,
                    command,
                },
            )?;
            eprintln!(
                "footprints: recorded command #{} in session {} with {} file change(s). See \
                 `footprints log --session {}`",
                outcome.command_id,
                outcome.session,
                outcome.changes.len(),
                outcome.session
            );
            return Ok(ExitCode::from(outcome.exit_code.clamp(0, 255) as u8));
        }
        Cmd::Recover => {
            let store = Store::open_default()?;
            let lines = run::recover(&store)?;
            if lines.is_empty() {
                println!("nothing to recover");
            }
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Daemon {
            roots,
            snapshot,
            seconds,
            fanotify,
            usn,
        } => {
            let store = Store::open_default()?;
            let r = daemon::run(
                &store,
                &daemon::DaemonOptions {
                    roots,
                    snapshot,
                    seconds,
                    fanotify,
                    usn,
                },
            )?;
            eprintln!(
                "footprints: daemon stopped. {} change(s) recorded in session {}, {} already \
                 recorded by another tier. See `footprints log --session {}`",
                r.recorded, r.session, r.joined, r.session
            );
        }
        Cmd::Prune {
            older_than,
            before,
            session,
            agent,
            all,
            max_size,
            yes,
            permanent,
            json,
        } => {
            let sel = prune::Selectors {
                older_than_ms: older_than,
                before_ms: before,
                session,
                agent,
                all,
                max_size,
            };
            let home = footprints::store::default_home()?;
            let outcome = if yes {
                let store = Store::open_exclusive(&home).map_err(busy_is_refusal)?;
                let plan = prune::plan(&store, &sel)?;
                prune::apply(&store, &plan, &prune::Disposal::from_env(permanent))?
            } else {
                // a dry run still works while the ledger is in use. It says that --yes would be
                // refused
                let (store, busy) = match Store::open_exclusive(&home) {
                    Ok(s) => (s, false),
                    Err(e) if e.is::<footprints::store::Busy>() => (Store::open(&home)?, true),
                    Err(e) => return Err(e),
                };
                let plan = prune::plan(&store, &sel)?;
                prune::Outcome {
                    plan,
                    pruned: false,
                    archive_kept_in: "none",
                    archive_name: None,
                    archive_path: None,
                    size_after: None,
                    busy,
                }
            };
            if json {
                print_json(&outcome)?;
            } else {
                print!("{}", prune::render(&outcome));
            }
        }
        Cmd::Import { archive, json } => {
            let store = Store::open_default()?;
            let r = prune::import(&store, &archive)?;
            if json {
                print_json(&r)?;
            } else {
                println!(
                    "restored {} action(s) in {} session(s) and {} stored file copy(ies) from {}",
                    r.actions,
                    r.sessions,
                    r.blobs_added,
                    archive.display()
                );
                if r.blobs_missing > 0 {
                    println!(
                        "{} file copy(ies) named by these actions are in neither the archive nor                          the ledger, so the changes that need them cannot be undone",
                        r.blobs_missing
                    );
                }
            }
        }
        Cmd::Mcp => {
            let store = Store::open_default()?;
            let stdin = std::io::stdin().lock();
            mcp::serve(&store, stdin, std::io::stdout().lock())?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
