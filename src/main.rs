use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use footprints::output::{self, Paint};
use footprints::store::{Filter, Kind, Store};
use footprints::undo::{self, Refused};
use footprints::{diff, export, hook, mcp, run};
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
        Data lives in $FOOTPRINTS_HOME when it is set and in ~/.local/share/footprints otherwise."
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
        /// the agent name stored with each action
        #[arg(long, global = true, default_value = hook::DEFAULT_AGENT)]
        agent: String,
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
    /// record the changes of a `footprints run` that was killed before it could
    Recover,
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
            if std::env::args().nth(1).as_deref() == Some("hook") {
                eprintln!("footprints: hook error: {e}");
                return ExitCode::SUCCESS;
            }
            e.exit();
        }
    };
    // a failing hook must never block the agent's tool call, so hook errors are reported on
    // stderr and the exit status stays 0
    if let Cmd::Hook { phase, agent } = &cli.cmd {
        if let Err(e) = run_hook(phase, agent) {
            eprintln!("footprints: hook error: {e:#}");
        }
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
        Cmd::Install {
            target: InstallTarget::ClaudeCode { print },
        } => {
            if !print {
                bail!(
                    "this version only prints the snippet and never edits agent settings. Run \
                     `footprints install claude-code --print` and merge the output into \
                     ~/.claude/settings.json or .claude/settings.json yourself"
                );
            }
            print!("{}", footprints::CLAUDE_CODE_SETTINGS);
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
        Cmd::Mcp => {
            let store = Store::open_default()?;
            let stdin = std::io::stdin().lock();
            mcp::serve(&store, stdin, std::io::stdout().lock())?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
