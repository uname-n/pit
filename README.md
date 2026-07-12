<p align="center">
  <img src="docs/logo.png" width="256" />
</p>

<h1 align="center">pit</h1>

<p align="center">
  a local issue tracker for orchestrating Claude — one binary, one SQLite file.
</p>

---

**pit** is an [MCP](https://modelcontextprotocol.io) issue tracker in a single SQLite file. No accounts, no server, no network. It works with any MCP client, and it's built for one workflow in particular: turning [claude](https://docs.anthropic.com/en/docs/claude-code) into an **orchestrator** that plans work as issues, hands each to a subagent in its own git worktree, reviews the diff, and merges.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/uname-n/pit/master/install.sh | sh
```

## Use it with Claude Code

Run `pit init` at the root of your repo:

```bash
pit init
```

This scaffolds everything Claude Code needs to orchestrate:

- **`CLAUDE.md`** — the orchestrator playbook (plan → delegate → review → integrate)
- **`.mcp.json`** — registers pit as an MCP server
- **`.claude/settings.json`** — permission allowlist + worktree config
- **`.claude/bins/delegate`** and **`.claude/bins/review`** — orchestrator helper scripts
- **`.gitignore`** entries for `.pit/`, `.claude/worktrees/`, `.claude/logs/`

`pit init` won't overwrite any of these if they already exist.

## Use it as a plain MCP server

Skip the orchestration scaffolding and register pit directly in your `.mcp.json`:

```json
{
    "mcpServers": {
        "pit": {
            "command": "pit"
        }
    }
}
```

pit creates `.pit/db.sqlite` in your working directory. Set `PIT_DB` to use a custom path.

## Kanban board

```bash
pit kanban
```

A live, read-only board of your issues in the terminal. Colors live in `.pit/settings.json` (next to the database), written with defaults on first launch.

```json
{
    "kanban": {
        "open": "#e0cfc2",
        "in_progress": "#ffc34c",
        "closed": "#867268",
        "dim": "#6c6c6c",
        "muted": "#b2b2b2",
        "label": "#b3728f",
        "link_blocks": "#ff5f5f",
        "link_duplicates": "#b3728f",
        "link_related": "#00cdcd"
    },
    "tail": {
        "header": "#b2b2b2",
        "message": "#e0cfc2",
        "tool": "#6c6c6c",
        "status": "#6c6c6c",
        "result": "#b3728f"
    }
}
```

The `tail` section themes `pit tail` (see [Following a run](#following-a-run)).

## Commands

```bash
pit            # run as MCP server on stdio (default)
pit init       # scaffold the orchestration config into the current repo
pit kanban     # live read-only kanban board
pit tail <id>  # follow an issue's most recent run log (streams the subagent's replies)
pit --help     # show all commands
```

### Following a run

While a delegated subagent is working, `pit tail <id>` follows its most recent
`.claude/logs/issue-<id>-*.jsonl` transcript in a full-screen view, pinned to the bottom as
new events stream in:

```bash
pit tail 2
```

```
pit · #2 · Core data model: ...
› I'll start by reading the issue to understand the requirements.
› Now let me explore the codebase structure.
◦ Read: src/db.rs
◦ Bash: cargo build
q · quit
```

The subagent's prose word-wraps under a `›` bullet, its tool calls appear as truncated `◦`
one-liners (thinking is skipped), and the final report closes the stream. Scroll with
`↑`/`↓`, `PgUp`/`PgDn`, `g`/`G`; scrolling back to the bottom re-pins to the live tail. Press
`q` to quit. Colors are configurable in the `tail` section of `.pit/settings.json` (`header`,
`message`, `tool`, `status`, and `result` for the final report), separate from the board's
`kanban` section. Set `PIT_LOG_DIR` to read logs from a custom directory.

## Tools

| Tool | Description |
|---|---|
| `pit_create_issue` | Create an issue with optional body, labels, status, priority |
| `pit_list_issues` | List issues, filtered by status/labels, sorted and paginated |
| `pit_get_issue` | Get one issue with all its comments |
| `pit_update_issue` | Update title, body, status, priority, labels, or close reason |
| `pit_add_comment` | Add a comment to an issue |
| `pit_search_issues` | Full-text search across titles, bodies, comments |
| `pit_list_labels` | List labels with issue counts |
| `pit_link_issues` | Link two issues (`blocks`, `duplicates`, `related`) |
| `pit_unlink_issues` | Remove a link between two issues |
| `pit_delete_issue` | Delete an issue and its comments |
