You have access to a local issue tracker called pit. Use it to track bugs, tasks, and decisions for the project you are working on. pit stores everything in a local SQLite database — issues persist across conversations.

## Core rules

1. **Track work as you go.** When you discover a bug, find a TODO, or identify a task that can't be completed right now — create an issue immediately rather than trying to remember it.
2. **Check before starting.** Call `list_issues` with status `open` at the start of a session to understand current priorities and avoid duplicate work.
3. **Update status.** When you begin working on an issue, set its status to `in-progress` via `update_issue`. When done, close it with status `closed` and an appropriate `closed_reason`.
4. **Comment as you go.** Use `add_comment` to record progress, decisions, blockers, and context that would be useful in a future conversation.
5. **Search before creating.** Use `search_issues` to check for duplicates before creating a new issue.

## When to create issues

- Bug discovered during development or testing
- Task identified that cannot be completed right now
- Feature request or enhancement idea that surfaces during conversation
- Refactoring opportunity noticed but deferred
- Technical debt identified

Do **not** create issues for trivial tasks you will complete immediately, duplicates of existing issues, or vague ideas with no actionable next step.

## Tool guide

| Goal | Tool | Key parameters |
|---|---|---|
| Track a new bug/task/feature | `create_issue` | title (required), body, labels, status, priority |
| See what needs doing | `list_issues` | status="open", priority, labels, sort="updated" |
| Get full context on an issue | `get_issue` | id |
| Start/finish/update work | `update_issue` | id, status, closed_reason, priority, labels_add/remove/set |
| Record progress or decisions | `add_comment` | id, body |
| Find related past work | `search_issues` | query, status, labels |
| Understand label taxonomy | `list_labels` | — |
| Remove invalid/duplicate issue | `delete_issue` | id |
| Link two issues | `link_issues` | source_id, target_id, link_type (blocks/relates_to/duplicates) |
| Remove a link | `unlink_issues` | source_id, target_id, link_type |

## Labels

Use labels to categorize issues. Labels are created automatically when first used. Keep them lowercase and consistent.

Recommended: `bug`, `feature`, `refactor`, `docs`, `test`, `phase-0`, `phase-p1`, `phase-p2`.

## Issue lifecycle

1. **Create** — status `open`, appropriate labels, priority if known
2. **Start** — update to `in-progress`
3. **Link** — connect related issues with `link_issues` (blocks, relates_to, duplicates)
4. **Comment** — record progress, decisions, blockers
5. **Close** — status `closed` with closed_reason:
   - `completed` — work is done
   - `wontfix` — decided not to address
   - `duplicate` — duplicate of another issue (link to the canonical issue first)

## Priority

Set priority when creating or updating an issue: `p0` (critical), `p1` (high), `p2` (medium), `p3` (low). Filter with `list_issues` or `search_issues`.

## Writing good issues

- **Title**: short, specific, action-oriented. "Fix parser crash on empty input" not "parser bug".
- **Body**: include context. For bugs: reproduction steps. For features: acceptance criteria. Markdown supported.
- **Labels**: at minimum categorize the type (bug/feature/refactor). Add priority if known.

## Writing good comments

- Record **why** decisions were made, not just what was done
- Reference specific files, functions, or line numbers
- Note any follow-up work needed
