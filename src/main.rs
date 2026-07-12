mod db;
mod error;
mod init;
mod kanban;
mod mcp;
mod settings;
mod tail;
mod types;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser)]
#[command(
    name = "pit",
    about = "local issue tracker",
    long_about = "pit — local issue tracker\n\nRuns as an MCP server on stdio when invoked with no subcommand.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Kanban style read-only board
    Kanban,
    /// Follow assistant text from run logs. With an issue id, follows that
    /// issue's most recent run; with no argument, opens the live dashboard of
    /// all in-progress runs (switch tabs with ←/→).
    Tail {
        /// Issue id whose newest .claude/logs/issue-<id>-*.jsonl to follow;
        /// omit to watch every in-progress run in the dashboard
        issue: Option<i64>,
    },
    /// Scaffold orchestration config files
    Init,
}

fn main() {
    let cli = Cli::parse();

    // Handled before the DB is opened so `pit init` never creates a .pit/ directory.
    if let Some(Command::Init) = cli.command {
        if let Err(e) = init::run() {
            eprintln!("pit: init error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let db_path = std::env::var("PIT_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./.pit/db.sqlite"));

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            eprintln!("pit: failed to create directory: {e}");
            std::process::exit(1);
        });
    }

    let db = db::Db::open(&db_path).unwrap_or_else(|e| {
        eprintln!("pit: failed to open database: {e}");
        std::process::exit(1);
    });

    // UI colors live in .pit/settings.json (sibling of the db file), created
    // with defaults on first run. It carries a `kanban` and a `tail` section.
    let load_settings = || {
        let settings_path = db_path
            .parent()
            .map(|p| p.join("settings.json"))
            .unwrap_or_else(|| PathBuf::from("./.pit/settings.json"));
        settings::load_or_create(&settings_path)
    };

    match cli.command {
        Some(Command::Kanban) => {
            let settings = load_settings().unwrap_or_else(|e| {
                eprintln!("pit: kanban error: {e}");
                std::process::exit(1);
            });
            if let Err(e) = kanban::run(&db, &settings.kanban) {
                eprintln!("pit: kanban error: {e}");
                std::process::exit(1);
            }
        }
        Some(Command::Tail { issue }) => {
            let settings = load_settings().unwrap_or_else(|e| {
                eprintln!("pit: tail error: {e}");
                std::process::exit(1);
            });
            // Logs live under .claude/logs/ (sibling of .pit/, gitignored). It is
            // independent of PIT_DB, so a dedicated PIT_LOG_DIR override mirrors
            // how PIT_DB relocates the database.
            let log_dir = std::env::var("PIT_LOG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./.claude/logs"));
            if let Err(e) = tail::run(&db, issue, &log_dir, &settings.tail) {
                eprintln!("pit: tail error: {e}");
                std::process::exit(1);
            }
        }
        Some(Command::Init) => {} // handled before the DB is opened
        None => run_mcp_server(db),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn run_mcp_server(db: db::Db) {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(msg) => {
                        if let Some(response) = mcp::handle_message(&db, &msg) {
                            let mut out = serde_json::to_string(&response).unwrap();
                            out.push('\n');
                            if let Err(e) = stdout.write_all(out.as_bytes()).await {
                                eprintln!("pit: write error: {e}");
                                break;
                            }
                            if let Err(e) = stdout.flush().await {
                                eprintln!("pit: flush error: {e}");
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let err = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": null,
                            "error": {
                                "code": -32700,
                                "message": format!("PARSE_ERROR: {e}")
                            }
                        });
                        let mut out = serde_json::to_string(&err).unwrap();
                        out.push('\n');
                        let _ = stdout.write_all(out.as_bytes()).await;
                        let _ = stdout.flush().await;
                    }
                }
            }
            Err(e) => {
                eprintln!("pit: read error: {e}");
                break;
            }
        }
    }
}
