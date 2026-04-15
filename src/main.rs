mod db;
mod error;
mod kanban;
mod mcp;
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
    /// live read-only kanban board (TUI)
    Kanban,
    /// tmux dashboard: claude (top) + pit kanban (bottom)
    Dashboard,
}

fn main() {
    let cli = Cli::parse();

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

    match cli.command {
        Some(Command::Kanban) => {
            if let Err(e) = kanban::run(&db) {
                eprintln!("pit: kanban error: {e}");
                std::process::exit(1);
            }
        }
        Some(Command::Dashboard) => {
            if let Err(e) = run_dashboard() {
                eprintln!("pit: dashboard error: {e}");
                std::process::exit(1);
            }
        }
        None => run_mcp_server(db),
    }
}

fn run_dashboard() -> Result<(), String> {
    use std::process::{Command as ProcCommand, Stdio};

    let session = "pit-dashboard";
    let top_percent: u32 = 30;

    let tmux_exists = ProcCommand::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("tmux not found: {e}"))?;
    if !tmux_exists.success() {
        return Err("tmux is required for `pit dashboard`".into());
    }

    let has_session = ProcCommand::new("tmux")
        .args(["has-session", "-t", session])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("failed to run tmux: {e}"))?;

    if !has_session.success() {
        let pit_bin = std::env::current_exe()
            .map_err(|e| format!("failed to locate pit binary: {e}"))?;
        let pit_bin = pit_bin.to_string_lossy().into_owned();

        let new_session = ProcCommand::new("tmux")
            .args(["new-session", "-d", "-s", session, "claude"])
            .status()
            .map_err(|e| format!("failed to create tmux session: {e}"))?;
        if !new_session.success() {
            return Err("tmux new-session failed".into());
        }

        let _ = ProcCommand::new("tmux")
            .args(["set-option", "-t", session, "mouse", "on"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = ProcCommand::new("tmux")
            .args(["set-option", "-t", session, "status", "off"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let kanban_cmd = format!("{} kanban", shell_escape(&pit_bin));
        let split = ProcCommand::new("tmux")
            .args([
                "split-window",
                "-v",
                "-p",
                &(100 - top_percent).to_string(),
                "-t",
                session,
                &kanban_cmd,
            ])
            .status()
            .map_err(|e| format!("failed to split tmux window: {e}"))?;
        if !split.success() {
            let _ = ProcCommand::new("tmux")
                .args(["kill-session", "-t", session])
                .status();
            return Err("tmux split-window failed".into());
        }

        let _ = ProcCommand::new("tmux")
            .args(["select-pane", "-t", &format!("{session}.0")])
            .status();
    }

    let hook_cmd = format!(
        "run-shell 'tmux resize-pane -t {session}:0.0 -y $(( #{{window_height}} * {top_percent} / 100 ))'"
    );
    let _ = ProcCommand::new("tmux")
        .args(["set-hook", "-t", session, "client-attached", &hook_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = ProcCommand::new("tmux")
        .args(["set-hook", "-t", session, "client-resized", &hook_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let target = if std::env::var_os("TMUX").is_some() {
        "switch-client"
    } else {
        "attach-session"
    };
    ProcCommand::new("tmux")
        .args([target, "-t", session])
        .status()
        .map_err(|e| format!("failed to attach tmux: {e}"))?;

    Ok(())
}

fn shell_escape(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ':' | '='))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
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
