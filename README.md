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

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/uname-n/pit/master/install.sh | sh
```

## Setup

```bash
claude mcp add pit -- pit
```

Or add pit to your project's `.mcp.json`:

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