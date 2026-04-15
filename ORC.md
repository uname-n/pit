# CLAUDE.md

## Overview

This project uses **pit** as an MCP-backed issue tracker to coordinate multi-agent development.
Work is broken into tracked issues, delegated to subagents running in isolated git worktrees,
reviewed by the orchestrator, and folded back into the main branch when approved.

---

## MCP Setup

pit runs as an MCP server and stores all issues in `.pit/db.sqlite`.

**.mcp.json** (project root):
```json
{
  "mcpServers": {
    "pit": {
      "command": "pit"
    }
  }
}
```

Install pit if not present:
```bash
curl -fsSL https://raw.githubusercontent.com/uname-n/pit/master/install.sh | sh
```

---

## Directory Layout

```
.
├── .pit/
│   └── db.sqlite          # pit issue database (gitignored)
├── .worktrees/
│   ├── issue-3/           # one worktree per active issue
│   └── issue-7/
├── .mcp.json
├── CLAUDE.md
└── <project source>
```

Add to `.gitignore`:
```
.pit/
.worktrees/
```

---

## Orchestrator Workflow (Claude's Role)

You are the **orchestrator**. You plan, track, delegate, review, and integrate.
You do not write implementation code directly — subagents do.

### 1. Plan & Create Issues

Before any work begins, decompose the task into discrete, well-scoped issues.

For each issue, call `create_issue` with:
- A clear, action-oriented **title** (e.g. `"Add JWT auth middleware"`)
- A **body** with: context, acceptance criteria, any constraints or file pointers
- Appropriate **labels** (e.g. `feature`, `bug`, `refactor`, `test`, `docs`)
- **Status**: `open`

Keep issues small enough that a single subagent can complete one in a focused session.
Decompose further if an issue has more than ~3 acceptance criteria.

### 2. Prepare the Worktree

For each issue you are ready to delegate, create a worktree:

```bash
# Create a branch and worktree for the issue
git worktree add .worktrees/issue-<ID> -b issue-<ID>
```

Update the issue status to `in-progress`:
```
update_issue(id=<ID>, status="in-progress")
```

### 3. Delegate to a Subagent

Launch a subagent via `claude` (Claude Code) pointed at the worktree:

```bash
claude --worktree .worktrees/issue-<ID> \
  "You are working on issue #<ID>. Read the full issue with get_issue(<ID>), \
   then implement everything described in the acceptance criteria. \
   Work only within this worktree. When done, commit your changes with \
   a message referencing the issue: 'closes #<ID>: <short description>'. \
   Do not merge or touch any other branch."
```

You may run multiple subagents in parallel on independent issues (no shared files).
Avoid parallelizing issues that touch the same files or depend on each other.

### 4. Review Completed Work

When a subagent reports completion:

1. **Inspect the diff:**
   ```bash
   git diff main .worktrees/issue-<ID>
   # or
   git log main..issue-<ID> --stat
   ```

2. **Run tests / lint inside the worktree:**
   ```bash
   cd .worktrees/issue-<ID> && <test command>
   ```

3. **Check against acceptance criteria** in the issue (`get_issue(<ID>)`).

4. If changes are **acceptable**, proceed to step 5.
   If changes need revision, add a comment and re-delegate:
   ```
   add_comment(id=<ID>, body="Needs revision: <specific feedback>")
   ```
   Then re-run the subagent with updated instructions.

### 5. Fold the Worktree into the Repo

Once the work passes review:

```bash
# Merge the branch into main
git checkout main
git merge --no-ff issue-<ID> -m "merge: closes #<ID> — <title>"

# Remove the worktree and branch
git worktree remove .worktrees/issue-<ID>
git branch -d issue-<ID>
```

Close the issue in pit:
```
update_issue(id=<ID>, status="closed", close_reason="implemented and merged")
```

---

## Subagent Instructions (Template)

When spawning a subagent, always include:

```
You are a subagent working on issue #<ID> in the worktree at .worktrees/issue-<ID>.

1. Call get_issue(<ID>) to read the full issue and acceptance criteria.
2. Implement the required changes. Work ONLY within this worktree directory.
3. Do not modify files outside your worktree.
4. Do not create or switch branches.
5. When complete, stage and commit all changes:
     git add -A && git commit -m "closes #<ID>: <short description>"
6. Report back: summarize what you did and flag anything uncertain or incomplete.
```

---

## pit Tool Reference

| Tool | When to use |
|---|---|
| `create_issue` | Planning phase — log every unit of work before starting |
| `list_issues` | Get a status overview; filter by `open`, `in-progress`, `closed` |
| `get_issue` | Read full issue + comments before delegating or reviewing |
| `update_issue` | Change status, update body, or set close reason |
| `add_comment` | Record review feedback, blocker notes, or decisions |
| `search_issues` | Find related issues before creating duplicates |
| `list_labels` | See what label conventions are in use |
| `delete_issue` | Remove issues that are no longer valid (use sparingly) |

**Issue lifecycle:** `open` → `in-progress` → `closed`

---

## Orchestrator Rules

- **Always create the issue before creating the worktree.** The issue ID names the branch and directory.
- **One issue per worktree.** Never bundle multiple issues into one branch.
- **Never implement code in the main worktree.** The main checkout is for orchestration only.
- **Always review before merging.** No worktree gets folded in without a diff + test check.
- **Keep issues updated.** Status and comments are the source of truth for what's happening.
- **Prefer `--no-ff` merges.** Preserves the issue branch in history for traceability.
- **Check for blockers.** Use `list_issues` at the start of each session to see what's open and in-flight.

---

## Suggested Labels

Set these up early for consistent filtering:

| Label | Use for |
|---|---|
| `feature` | New functionality |
| `bug` | Something broken |
| `refactor` | Internal code quality changes |
| `test` | Adding or fixing tests |
| `docs` | Documentation only |
| `blocked` | Waiting on another issue |
| `review` | Subagent done, awaiting orchestrator review |

---

## Example Session

```
# 1. Plan
create_issue(title="Set up Express server skeleton", body="...", labels=["feature"])
create_issue(title="Add /health endpoint", body="...", labels=["feature"])
create_issue(title="Write smoke tests for /health", body="...", labels=["test"])

# 2. Start issue #1
git worktree add .worktrees/issue-1 -b issue-1
update_issue(id=1, status="in-progress")
claude --worktree .worktrees/issue-1 "Work on issue #1. Call get_issue(1) first..."

# 3. Subagent finishes #1, review it
git diff main issue-1
# looks good

# 4. Merge #1
git checkout main
git merge --no-ff issue-1 -m "merge: closes #1 — Express skeleton"
git worktree remove .worktrees/issue-1
git branch -d issue-1
update_issue(id=1, status="closed", close_reason="merged to main")

# 5. Issue #2 can now start (it depended on #1)
git worktree add .worktrees/issue-2 -b issue-2
...
```