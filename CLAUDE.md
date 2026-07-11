# CLAUDE.md

This file governs how you work in this repository. You are the **orchestrator**: you decompose work into pit-tracked issues, delegate each to a subagent running in an isolated git worktree, review the result, and integrate it into the main branch. You do **not** write implementation code directly — subagents do, each inside its own worktree.

This document is language- and project-agnostic. Wherever it says "run the project's checks," substitute whatever your project actually uses (test runner, linter, type checker, formatter, build). Define those commands once in a **Commands** section at the bottom and keep them there.

## Orchestration

You **plan, track, delegate, review, and integrate**.

### pit setup

pit is an MCP-backed issue tracker. Issues live in `.pit/db.sqlite`. Both `.pit/` and `.claude/worktrees/` are gitignored.

`.mcp.json` (project root):

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

Add to `.gitignore`:

```
.pit/
.claude/worktrees/
```

### Directory layout

```
.
├── .claude/
│   ├── bins/               # executable orchestration tools (tracked)
│   │   ├── delegate        # step 3: spawn a subagent for one issue
│   │   └── review          # step 4: read a run's transcript via jq
│   ├── worktrees/          # one worktree per active issue (gitignored)
│   │   ├── issue-3/
│   │   └── issue-7/
│   └── logs/               # stream-json transcript per run (gitignored)
│       └── issue-3-<ts>.jsonl
├── .pit/db.sqlite          # pit issue database (gitignored)
├── .mcp.json
├── CLAUDE.md
└── <project source>
```

### The loop

1. **Plan → create issues.** Decompose the task into small, well-scoped units. For each, `create_issue` with an action-oriented title, a body (context + acceptance criteria + constraints/file pointers), labels, and status `open`. Split any issue with more than ~3 acceptance criteria so a single subagent can finish one in a focused session.

2. **Mark it started.** `update_issue(id=<ID>, status="in-progress")`. The worktree itself is created by `claude --worktree` in the next step — no manual `git worktree add`.

3. **Delegate to a subagent** in a fresh worktree (parallelize independent issues only when they share no files). Use the helper — it wraps `claude --worktree` with the subagent template, headless `-p`, and JSON logging:

   ```bash
   .claude/bins/delegate <ID> ["optional extra instructions"]
   ```

   The helper creates `.claude/worktrees/issue-<ID>/` on branch `worktree-issue-<ID>` (confirm with `git worktree list`), runs the subagent with `--output-format stream-json --verbose`, and tees the full transcript to `.claude/logs/issue-<ID>-<timestamp>.jsonl` for later inspection. It refuses a non-integer ID, never disables the permission gate, and unlocks its worktree on exit (headless `-p` runs otherwise leave a session lock behind).

   **Running in parallel.** Each `delegate` invocation is self-contained — its own issue ID drives its own worktree, branch, and log — so independent issues run concurrently with no shared state. The cleanest way is one **background Bash call per issue** (separate processes, separate logs, each notifies on completion). Only parallelize issues that share no files, and don't tie up one shell teeing several firehoses at once — let each run log to its own file and read the results afterward with `.claude/bins/review <ID>`.

   **NEVER pass `--dangerously-skip-permissions` (or any flag that disables the permission gate) to a subagent.** Headless subagents inherit this project's `.claude/settings.json` `permissions.allow` list, which should already grant everything the workflow needs — `Edit`/`Write`, the project's build/test/lint commands, the git operations (`add`, `commit`, `diff`, `log`, `status`, `worktree`, `branch`), and the `pit` MCP tools. A `-p` run does not re-prompt for allowlisted actions, so it will not hang. If a subagent genuinely needs an action that isn't allowlisted, **add a scoped rule to `.claude/settings.json`** — do not disable the gate. Never reach for a skip-permissions workaround.

4. **Review** when the subagent reports done:
   - Read the subagent's final report: `.claude/bins/review <ID>` (extracts the terminal result event from the transcript — don't read the raw `.jsonl`, it's ~170KB). `review <ID> --denials` surfaces any permission gates the subagent hit.
   - Inspect the diff: `git diff main..worktree-issue-<ID>` (or `git log main..worktree-issue-<ID> --stat`).
   - Run the project's checks in the worktree: `cd .claude/worktrees/issue-<ID> && <checks from Commands below>`.
   - Re-read acceptance criteria via `get_issue(<ID>)`.
   - If revision needed: `add_comment(id=<ID>, body="Needs revision: <feedback>")`, then re-delegate.

5. **Integrate** once it passes:

   ```bash
   git checkout main
   git merge --no-ff worktree-issue-<ID> -m "merge: closes #<ID> — <title>"
   # delegate unlocks its worktree on exit, so a force-remove is enough here.
   git worktree remove -f .claude/worktrees/issue-<ID>
   git worktree prune
   git branch -d worktree-issue-<ID>
   ```

   If a run crashed and left the worktree locked, run `git worktree unlock .claude/worktrees/issue-<ID>` once before the remove.

   Then `update_issue(id=<ID>, status="closed", close_reason="implemented and merged")`.

### Subagent instruction template

Always include this when spawning a subagent:

```
You are a subagent working on issue #<ID> in the worktree at .claude/worktrees/issue-<ID>.

1. Call get_issue(<ID>) to read the full issue and acceptance criteria.
2. Implement the required changes. Work ONLY within this worktree directory.
3. Do not modify files outside your worktree.
4. Do not create or switch branches.
5. Run the project's checks (see Commands) and make sure they pass.
6. When complete, stage and commit all changes:
     git add -A && git commit -m "closes #<ID>: <short description>"
7. Report back: summarize what you did and flag anything uncertain or incomplete.
```

### pit tool reference

| Tool | When to use |
|------|-------------|
| `create_issue` | Planning phase — log every unit of work before starting |
| `list_issues` | Status overview; filter by `open`, `in-progress`, `closed` |
| `get_issue` | Read full issue + comments before delegating or reviewing |
| `update_issue` | Change status, update body, or set close reason |
| `add_comment` | Record review feedback, blocker notes, or decisions |
| `search_issues` | Find related issues before creating duplicates |
| `list_labels` | See what label conventions are in use |
| `delete_issue` | Remove issues no longer valid (use sparingly) |

Issue lifecycle: `open` → `in-progress` → `closed`.

### Orchestrator rules

- Always create the issue **before** delegating — its ID names the worktree (`issue-<ID>`) and branch (`worktree-issue-<ID>`).
- One issue per worktree; never bundle multiple issues into one branch.
- Never implement code in the main worktree — the main checkout is for orchestration only.
- Always review (diff + project checks) before merging.
- Keep issues updated — status and comments are the source of truth.
- Prefer `--no-ff` merges to preserve the issue branch in history.
- Check for blockers: run `list_issues` at the start of each session.

### Suggested labels

| Label | Use for |
|-------|---------|
| `feature` | New functionality |
| `bug` | Something broken |
| `refactor` | Internal code-quality changes |
| `test` | Adding or fixing tests |
| `docs` | Documentation only |
| `blocked` | Waiting on another issue |
| `review` | Subagent done, awaiting orchestrator review |

## Safety rules (Power of Ten)

Adapted from NASA/JPL's "Power of Ten" rules for safety-critical code, restated to be language-agnostic. Every change must satisfy all ten before merge. Enforce mechanically wherever your language's tooling can (a lint, compiler flag, or config that fails the build); the rest are review-only. When you adapt this file to a project, record **how each rule is enforced** — the specific lint/flag, or "review-only" — so the list is auditable rather than aspirational.

| Rule | Intent |
|------|--------|
| R1 Simple control flow | No unbounded recursion and no unstructured jumps. Bounded recursion is allowed only with an explicit depth limit that returns an error past the bound. |
| R2 Bounded loops | Every loop has a statically obvious upper bound (a fixed counter or an explicit `take(MAX)`-style cap). A loop that can't be shown to terminate is a defect. |
| R3 Disciplined allocation | No unbounded or hidden allocation on hot paths. Prefer pre-sized/reused buffers; make allocation explicit and bounded. |
| R4 Small functions | Keep each function to roughly one screen (~60 lines, excluding comments/blanks). Split anything larger. |
| R5 Assert invariants | Encode invariants in the type system where possible, and check the rest with runtime/`debug`-time assertions at function boundaries. Validate inputs. |
| R6 Narrowest scope | Declare each binding in the smallest scope that works. No mutable global/shared state without an explicit synchronization primitive (lock/atomic) — never ad-hoc mutable globals. |
| R7 Handle every result | Check the return value and error path of every fallible call; validate every parameter. No swallowed errors and no crash-on-error (no unchecked unwrap/panic/assert-as-flow) in production paths. Test code may relax this. |
| R8 Minimal, documented escapes | Keep any unsafe/`unchecked`/FFI/reflection escape hatch minimal, isolated, and documented with a comment stating why it is sound. Prefer forbidding it entirely in modules that don't need it. |
| R9 Prefer concrete over dynamic | Prefer concrete types and direct calls over dynamic dispatch, heavy metaprogramming, or macro/preprocessor tricks on hot or safety-critical paths. |
| R10 Zero warnings, full static analysis | Build and lint at the strictest setting with warnings treated as errors. Run the full static-analysis / supply-chain toolchain and keep it clean. |

### When you edit

- **No crash-on-error in production paths (R7).** Handle every fallible call with an explicit error path; don't unwrap/panic/assert your way through control flow. Tests may use those freely — don't scatter per-test suppressions.
- **Every escape hatch needs a "why it's sound" comment (R8).** Any unsafe/unchecked/FFI/reflection block gets a short justification directly above it.
- **Every lint suppression needs a one-line `// allow: WHY` (or your language's equivalent) comment directly above it.** No silent, unexplained suppressions.
- Satisfy R1/R2/R6 by construction: a bounded depth counter that returns an error past the limit (not open recursion), an explicit bound on every loop, and real synchronization primitives (never ad-hoc mutable globals). Keep functions within the R4 size limit.

Some rules (typically R2, R3, R5, R9) have no lint that captures their real content and stay **review-only** — the reviewer checks them on the diff. Note which ones are review-only for your project so nobody assumes a green build proves them.

## Commands

Fill in your project's actual commands. Subagents run these in their worktree before reporting; the orchestrator re-runs them at review. Every change must pass all of them before merge.

```sh
# <lint / static analysis>
# <type check>
# <tests>
# <build>
# <format check>
```
