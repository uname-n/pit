<p align="center">
  <img src="docs/logo.png" width="256" />
</p>

<h1 align="center">pit</h1>

<p align="center">
  local issue tracker MCP server — one binary, one SQLite file.
</p>

---

**pit** is a lightweight issue tracker that runs as an [MCP](https://modelcontextprotocol.io) server. It stores everything in a single SQLite database, so there's nothing to configure and no external services to manage.

Built for use with [Claude Code](https://docs.anthropic.com/en/docs/claude-code) and other MCP-compatible clients.

## Features

- **Single binary, single file** — no setup, no accounts, no network dependency
- **Full-text search** — powered by SQLite FTS5
- **Labels** — auto-created on first use
- **Comments** — threaded context on any issue
- **Issue lifecycle** — `open` → `in-progress` → `closed`
- **Kanban TUI** — live read-only board via `pit kanban`
- **One-command setup** — `pit init` scaffolds a repo for the orchestration workflow

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/uname-n/pit/master/install.sh | sh
```

## Setup

The fastest way to get going is to run `pit init` in your repository:

```bash
pit init
```

This scaffolds `CLAUDE.md`, `.mcp.json`, and `.claude/settings.json`, and adds `.pit/` and `.claude/worktrees/` to your `.gitignore` — everything Claude Code needs to run the [orchestration workflow](#orchestration). It refuses to run if any of those three files already exists, so it never clobbers your own config.

If you just want the MCP server without the orchestration scaffolding, register it directly:

```bash
claude mcp add pit -- pit
```

Or add pit to your project's `.mcp.json` by hand:

```json
{
  "mcpServers": {
    "pit": {
      "command": "pit"
    }
  }
}
```

pit will create a `.pit/db.sqlite` file in your working directory. To use a custom path, set the `PIT_DB` environment variable.

## Usage

```bash
pit            # run as MCP server on stdio
pit init       # scaffold orchestration config files into the current repo
pit kanban     # live read-only kanban board (TUI)
pit --help     # show all commands
```

## Orchestration

[`CLAUDE.md`](CLAUDE.md) and [`.claude/settings.json`](.claude/settings.json) turn Claude Code into an orchestrator that plans work in pit, delegates each issue to a subagent in its own git worktree, reviews the diff, and merges it back to main. The main checkout stays clean — Claude only plans, reviews, and integrates; subagents write the code.

Run `pit init` to drop these files into your repo in one step, or copy them in by hand.

## Tools

| Tool | Description |
|---|---|
| `pit_create_issue` | Create a new issue with optional body, labels, and status |
| `pit_list_issues` | List issues with filtering by status/labels, sorting, and pagination |
| `pit_get_issue` | Get a single issue with all its comments |
| `pit_update_issue` | Update title, body, status, labels, or close reason |
| `pit_add_comment` | Add a comment to an issue |
| `pit_search_issues` | Full-text search across titles, bodies, and comments |
| `pit_list_labels` | List all labels with issue counts |
| `pit_delete_issue` | Delete an issue and its comments |