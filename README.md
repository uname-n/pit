<p align="center">
  <img src="docs/logo.png" width="256" />
</p>

<h1 align="center">pit</h1>

<p align="center">
  a local issue tracker for orchestrating Claude — one binary, one SQLite file.
</p>

---

**pit** is an [MCP](https://modelcontextprotocol.io) issue tracker that lives entirely in a single SQLite file. No accounts, no server to run, no network. It's a plain MCP server that works with any compatible client, but it's built around one workflow in particular: turning [Claude Code](https://docs.anthropic.com/en/docs/claude-code) into an **orchestrator** that plans work as issues, hands each to a subagent in its own git worktree, reviews the diff, and merges.

That workflow is the reason pit exists. If you just want a lightweight tracker your agent can read and write, it's that too — skip to [As a plain MCP server](#as-a-plain-mcp-server).

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/uname-n/pit/master/install.sh | sh
```

## Quick start

Run `pit init` at the root of your repository:

```bash
pit init
```

That scaffolds everything Claude Code needs to run the orchestration workflow:

- **`CLAUDE.md`** — the orchestrator playbook (plan → delegate → review → integrate)
- **`.mcp.json`** — registers pit as an MCP server
- **`.claude/settings.json`** — the permission allowlist and worktree config that let headless subagents run without hanging
- **`.gitignore`** entries for `.pit/`, `.claude/worktrees/`, and `.claude/logs/`

`pit init` refuses to run if any of those config files already exists, so it never clobbers your own setup. Open Claude Code in the repo and it will start planning work in pit and delegating to worktrees.

## The workflow it's built for

Claude stops writing code in your main checkout. Instead, one session becomes the **orchestrator** — it plans, delegates, reviews, and merges, and it never edits a file in the main tree. A fleet of throwaway subagents does the typing, each in its own git worktree. State lives in pit, so it survives a context window and is readable by a *different* process than the one that wrote it.

Every unit of work runs the same loop:

1. **Plan** — decompose the task into small, scoped issues. The issue ID minted here names everything downstream.
2. **Start** — flip the issue to `in-progress`.
3. **Delegate** — spawn a headless subagent in a fresh worktree (`.claude/worktrees/issue-<ID>/` on branch `worktree-issue-<ID>`). It reads its one issue, implements it inside the worktree, and commits.
4. **Review** — diff the branch, run the project's checks, re-read the acceptance criteria. Falls short? Comment on the issue and re-delegate.
5. **Integrate** — merge the branch, drop the worktree, close the issue. The next issue branches off the new `HEAD`, so work compounds instead of colliding.

Two settings make it hold together, both in `.claude/settings.json`:

- **`worktree.symlinkDirectories: [".pit"]`** — a fresh worktree is a clean checkout, so the gitignored `.pit/` database wouldn't come along. The symlink points every worktree at the same issue DB. One shared brain.
- **`worktree.baseRef: "head"`** — branch each worktree off the current `HEAD`, not a fixed base, so every issue starts from the latest integrated state.

The permission `allow` list is scoped tight (`Bash(git commit *)`, never a bare `Bash`) and covers everything the loop touches, so a headless `-p` subagent never stops to ask. Don't reach for `--dangerously-skip-permissions` — if a subagent needs a new action, add a scoped rule.

## As a plain MCP server

pit is a standard MCP server on stdio. If you don't want the orchestration scaffolding, register it directly:

```bash
claude mcp add pit -- pit
```

Or add it to your project's `.mcp.json` by hand:

```json
{
  "mcpServers": {
    "pit": {
      "command": "pit"
    }
  }
}
```

pit creates `.pit/db.sqlite` in your working directory. Set the `PIT_DB` environment variable to use a custom path.

## Kanban board

`pit kanban` opens a live, read-only board of your issues in the terminal:

```bash
pit kanban
```

It reads its colors from `.pit/settings.json` (a sibling of the database), created with sensible defaults on first launch. Edit the `#rrggbb` hex codes under `"kanban"` and relaunch to recolor — you only need to include the keys you want to change; omitted colors fall back to defaults. An invalid hex value or malformed JSON makes `pit kanban` exit with an error naming the offending field rather than silently ignoring it.

```json
{
    "kanban": {
        "open": "#b2b2b2",
        "in_progress": "#ff5f5f",
        "closed": "#00cdcd",
        "dim": "#6c6c6c",
        "muted": "#b2b2b2",
        "label": "#cd00cd",
        "link_blocks": "#ff5f5f",
        "link_duplicates": "#cd00cd",
        "link_related": "#00cdcd"
    }
}
```

`.pit/` is gitignored, so this file is user-local by design.

## Commands

```bash
pit            # run as MCP server on stdio (default)
pit init       # scaffold the orchestration config into the current repo
pit kanban     # live read-only kanban board (TUI)
pit --help     # show all commands
```

## Features

- **Single binary, single file** — no setup, no accounts, no network dependency
- **Full-text search** — powered by SQLite FTS5
- **Labels** — auto-created on first use, with issue counts
- **Comments** — threaded context on any issue
- **Issue links** — `blocks`, `duplicates`, and `related` relationships between issues
- **Issue lifecycle** — `open` → `in-progress` → `closed`
- **Kanban TUI** — live read-only board via `pit kanban`
- **One-command setup** — `pit init` scaffolds a repo for the orchestration workflow

## Tools

| Tool | Description |
|---|---|
| `pit_create_issue` | Create a new issue with optional body, labels, status, and priority |
| `pit_list_issues` | List issues with filtering by status/labels, sorting, and pagination |
| `pit_get_issue` | Get a single issue with all its comments |
| `pit_update_issue` | Update title, body, status, priority, labels, or close reason |
| `pit_add_comment` | Add a comment to an issue |
| `pit_search_issues` | Full-text search across titles, bodies, and comments |
| `pit_list_labels` | List all labels with issue counts |
| `pit_link_issues` | Link two issues (`blocks`, `duplicates`, `related`) |
| `pit_unlink_issues` | Remove a link between two issues |
| `pit_delete_issue` | Delete an issue and its comments |
