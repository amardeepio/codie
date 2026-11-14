//! Read-only session inspection commands (`SESS-008`).
//!
//! `codie sessions` never rewrites a session file. Mutating operations stay in
//! the interactive app, where the user can see the resulting transcript.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use codie_session::Session;

use crate::config::{Config, normalize_cwd};

/// Arguments for `codie sessions`.
#[derive(Debug, Args)]
pub(crate) struct SessionsArgs {
    /// Session operation.
    #[command(subcommand)]
    command: SessionsCommand,
}

/// Session operations.
#[derive(Debug, Subcommand)]
enum SessionsCommand {
    /// List sessions, newest first.
    List {
        /// Emit one JSON object per line.
        #[arg(long)]
        json: bool,
        /// Only show sessions recorded for this working directory.
        #[arg(long, value_name = "DIR")]
        cwd: Option<PathBuf>,
    },
    /// Show one session.
    Show {
        /// Session identifier, prefix, or file path.
        id: String,
        /// Print the entry tree.
        #[arg(long)]
        tree: bool,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Export one session as JSONL or HTML.
    Export {
        /// Session identifier, prefix, or file path.
        id: String,
        /// Write to this file instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
    },
}

/// Session export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ExportFormat {
    /// Canonical JSONL.
    Jsonl,
    /// Self-contained HTML transcript.
    Html,
}

/// Runs one session command.
pub(crate) fn run(args: SessionsArgs, config: &Config) -> Result<u8> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cwd = normalize_cwd(&cwd);
    let session_dir = config.session_dir_for(&cwd);

    match args.command {
        SessionsCommand::List { json, cwd: filter } => {
            let cwd = filter.unwrap_or(cwd);
            let summaries = Session::list_for_cwd(&cwd, &session_dir)?;
            if json {
                for summary in summaries {
                    println!(
                        "{}",
                        serde_json::json!({
                            "id": summary.id,
                            "name": summary.name,
                            "cwd": summary.cwd,
                            "path": summary.path,
                            "created_at": summary.created_at,
                            "messages": summary.message_count,
                            "parent_session": summary.parent_session,
                        })
                    );
                }
            } else if summaries.is_empty() {
                println!("No sessions for {}.", cwd.display());
            } else {
                println!("{:<20} {:>6}  {:<18} PATH", "NAME", "MSGS", "ID");
                for summary in summaries {
                    println!(
                        "{:<20} {:>6}  {:<18} {}",
                        summary.name.as_deref().unwrap_or("unnamed"),
                        summary.message_count,
                        summary.id,
                        summary.path.display()
                    );
                }
            }
            Ok(0)
        }
        SessionsCommand::Show { id, tree, json } => {
            let session = Session::resolve(&session_dir, &id)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": session.id(),
                        "name": session.name(),
                        "cwd": session.cwd(),
                        "path": session.path(),
                        "created_at": session.header().timestamp,
                        "parent_session": session.parent_session(),
                        "entries": session.entries().len(),
                        "messages": session.projection().len(),
                        "estimated_tokens": session.estimate_tokens(),
                        "leaf": session.leaf_target(),
                    })
                );
                if tree {
                    for entry in session.tree() {
                        println!(
                            "{}",
                            serde_json::json!({
                                "id": entry.id,
                                "parent_id": entry.parent_id,
                                "depth": entry.depth,
                                "on_branch": entry.on_branch,
                                "kind": entry.kind,
                                "description": entry.description,
                            })
                        );
                    }
                }
            } else {
                println!("id:       {}", session.id());
                println!("name:     {}", session.name().unwrap_or("unnamed"));
                println!("cwd:      {}", session.cwd());
                println!("path:     {}", session.path().display());
                println!("created:  {}", session.header().timestamp);
                if let Some(parent) = session.parent_session() {
                    println!("parent:   {parent}");
                }
                println!("entries:  {}", session.entries().len());
                println!("messages: {}", session.projection().len());
                println!("tokens:   ~{}", session.estimate_tokens());
                if let Some(leaf) = session.leaf_target() {
                    println!("leaf:     {leaf}");
                }
                if tree {
                    println!();
                    for entry in session.tree() {
                        let marker = if entry.on_branch { "●" } else { "○" };
                        println!(
                            "{}{marker} {} {:<16} {}",
                            "  ".repeat(entry.depth.min(8)),
                            entry.id,
                            entry.kind,
                            entry.description
                        );
                    }
                }
            }
            Ok(0)
        }
        SessionsCommand::Export { id, output, format } => {
            let session = Session::resolve(&session_dir, &id)?;
            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(&path)
                        .with_context(|| format!("cannot create {}", path.display()))?;
                    match format {
                        ExportFormat::Jsonl => session.export(&mut file)?,
                        ExportFormat::Html => session.export_html(&mut file)?,
                    }
                    println!("Exported {} to {}.", session.id(), path.display());
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut lock = stdout.lock();
                    match format {
                        ExportFormat::Jsonl => session.export(&mut lock)?,
                        ExportFormat::Html => session.export_html(&mut lock)?,
                    }
                    lock.flush()?;
                }
            }
            Ok(0)
        }
    }
}
