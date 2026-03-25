# /branch — Create a branch from a lore task

Create a git branch with a name derived from lore task metadata.

## Steps

### 1. Read task

If `--status-only` flag is provided or user specifies a task ID, look up the task in `lore/1-tasks/` (any status directory).

Otherwise, read `lore/0-session/current-task.md` (follows symlink).

Parse YAML frontmatter to extract `id`, `title`, and `type`.

If no task is found, **STOP** and ask the user to set one with `lore_set-task`.

### 2. Determine branch type prefix

**For status-only branches** (`--status-only`), always use `chore`.

**For implementation branches**, map lore task `type`:

| Task Type | Branch Prefix |
| --------- | ------------- |
| FEATURE   | `feat`        |
| RESEARCH  | `research`    |
| BUG       | `fix`         |
| REFACTOR  | `refactor`    |
| DOCS      | `docs`        |

### 3. Extract slug from filename

The task filename follows the pattern `NNNN_TYPE_slug.md` or `NNNN_TYPE_slug/README.md`.

Extract the `slug` part (everything after the second underscore, without `.md`).

### 4. Construct branch name

**Implementation:** `{prefix}/{id}_{slug}`
**Status-only:** `chore/{id}_{slug}`

Examples:

- `0009_FEATURE_domain-types-ledger-transaction.md` → `feat/0009_domain-types-ledger-transaction`
- `0008_RESEARCH_event-interpreter-patterns/` → `research/0008_event-interpreter-patterns`
- `0009_FEATURE_domain-types-ledger-transaction.md` + `--status-only` → `chore/0009_domain-types-ledger-transaction`

### 5. Determine base branch

Check if `develop` branch exists (local or remote). If yes, use `develop`. Otherwise use `master`.

### 6. Create and switch to branch

```bash
git checkout {base} && git checkout -b {branch-name}
```

### 7. Confirm

Print:

> Branch `{branch-name}` created from `{base}` for task {id}: {title}

## Arguments

- `/branch` — uses active task, implementation branch
- `/branch 0042` — uses task 0042, implementation branch
- `/branch --status-only` — uses active task, chore branch for status change
- `/branch --status-only 0042` — uses task 0042, chore branch
- `--base <branch>` — override base branch

## Workflow

**Typical two-phase flow:**

1. Pick task → `/branch --status-only 0009` → change status to active → PR → merge (board updates)
2. Start work → `/branch 0009` → implement → PR → merge
