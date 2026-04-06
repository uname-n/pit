mod db;
mod error;
mod mcp;
mod types;

use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main(flavor = "current_thread")]
async fn main() {
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
