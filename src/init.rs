use std::fs;
use std::path::Path;

const CLAUDE_MD: &str = include_str!("../template/CLAUDE.md");
const MCP_JSON: &str = include_str!("../template/.mcp.json");
const SETTINGS_JSON: &str = include_str!("../template/.claude/settings.json");
const DELEGATE: &str = include_str!("../template/.claude/bins/delegate");
const REVIEW: &str = include_str!("../template/.claude/bins/review");

/// gitignore entries pit's orchestration workflow relies on. Note `.claude/bins/`
/// is intentionally NOT ignored — the delegate/review tools are tracked (see CLAUDE.md).
const GITIGNORE_ENTRIES: [&str; 3] = [".pit", ".claude/worktrees", ".claude/logs"];

/// Fixed table of files scaffolded by `pit init`: (target path, embedded
/// contents, executable). The `.claude/bins/` tools must be executable — see
/// `make_executable`, since `fs::write` creates files 0o644.
const FILES: [(&str, &str, bool); 5] = [
    ("CLAUDE.md", CLAUDE_MD, false),
    (".mcp.json", MCP_JSON, false),
    (".claude/settings.json", SETTINGS_JSON, false),
    (".claude/bins/delegate", DELEGATE, true),
    (".claude/bins/review", REVIEW, true),
];

/// Scaffold the orchestration config files into the current directory.
///
/// Refuses to run if any target file already exists (fresh-repo guard),
/// writing nothing in that case.
pub fn run() -> Result<(), String> {
    // R5: validate the precondition (no target exists) before any mutation.
    let existing: Vec<&str> = FILES
        .iter()
        .filter(|(path, _, _)| Path::new(path).exists())
        .map(|(path, _, _)| *path)
        .collect();
    if !existing.is_empty() {
        return Err(format!(
            "refusing to overwrite existing files: {}",
            existing.join(", ")
        ));
    }

    // R2: bounded loop over the fixed file table.
    for (path, contents, executable) in FILES {
        create_parent(path)?;
        fs::write(path, contents).map_err(|e| format!("failed to write {path}: {e}"))?;
        if executable {
            make_executable(path)?;
        }
    }

    println!("pit: project initialized");
    ensure_gitignore()
}

/// Set the executable bit (0o755) on `path`. `fs::write` creates files 0o644, so
/// the `.claude/bins/` tools need this to be runnable straight after `pit init`.
#[cfg(unix)]
fn make_executable(path: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|e| format!("failed to stat {path}: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).map_err(|e| format!("failed to chmod {path}: {e}"))
}

/// No-op on non-unix platforms, which have no executable permission bit.
#[cfg(not(unix))]
fn make_executable(_path: &str) -> Result<(), String> {
    Ok(())
}

/// Create the parent directory of `path` if it has one.
fn create_parent(path: &str) -> Result<(), String> {
    let parent = match Path::new(path).parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Ok(()),
    };
    fs::create_dir_all(parent).map_err(|e| format!("failed to create {}: {e}", parent.display()))
}

/// Append any missing `GITIGNORE_ENTRIES` to `.gitignore`, creating it if absent.
fn ensure_gitignore() -> Result<(), String> {
    let path = ".gitignore";
    let existing = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("failed to read {path}: {e}")),
    };
    if let Some(block) = merge_gitignore(&existing, &GITIGNORE_ENTRIES) {
        let mut out = existing;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&block);
        fs::write(path, out).map_err(|e| format!("failed to write {path}: {e}"))?;
    }
    Ok(())
}

/// Return the newline-terminated block of `entries` missing (line-exact) from
/// `existing`, or `None` if every entry is already present.
fn merge_gitignore(existing: &str, entries: &[&str]) -> Option<String> {
    let present: std::collections::HashSet<&str> = existing.lines().collect();
    let mut block = String::new();
    for entry in entries {
        if !present.contains(entry) {
            block.push_str(entry);
            block.push('\n');
        }
    }
    (!block.is_empty()).then_some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_gitignore_all_present_returns_none() {
        let existing = ".pit/\n.claude/worktrees/\n.claude/logs/\n";
        assert_eq!(merge_gitignore(existing, &GITIGNORE_ENTRIES), None);
    }

    #[test]
    fn merge_gitignore_some_missing_appends_just_those_lines() {
        let existing = ".pit/\n";
        assert_eq!(
            merge_gitignore(existing, &GITIGNORE_ENTRIES),
            Some(".claude/worktrees/\n.claude/logs/\n".to_string())
        );
    }

    #[test]
    fn merge_gitignore_empty_appends_all() {
        assert_eq!(
            merge_gitignore("", &GITIGNORE_ENTRIES),
            Some(".pit/\n.claude/worktrees/\n.claude/logs/\n".to_string())
        );
    }

    #[test]
    fn merge_gitignore_matches_are_line_exact() {
        // A superstring line must not count as a match.
        let existing = "foo/.pit/\n";
        assert_eq!(
            merge_gitignore(existing, &GITIGNORE_ENTRIES),
            Some(".pit/\n.claude/worktrees/\n.claude/logs/\n".to_string())
        );
    }
}
