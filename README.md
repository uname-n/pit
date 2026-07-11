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
- **`.claude/settings.json`** — permission allowlist + worktree config so headless subagents run without hanging
- **`.gitignore`** entries for `.pit/`, `.claude/worktrees/`, `.claude/logs/`

`pit init` won't overwrite any of these if they already exist. Open Claude Code in the repo and it starts planning work in pit and delegating to worktrees.

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

A live, read-only board of your issues in the terminal. Colors live in `.pit/settings.json` (next to the database), written with defaults on first launch. Override any `#rrggbb` code under `"kanban"` and relaunch — include only the keys you want to change; the rest fall back to defaults.

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
    }
}
```

## Commands

```bash
pit            # run as MCP server on stdio (default)
pit init       # scaffold the orchestration config into the current repo
pit kanban     # live read-only kanban board
pit --help     # show all commands
```

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
