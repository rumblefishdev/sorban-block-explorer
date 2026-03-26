# /promote-task — Activate a lore task and push to develop

Promote a task from `backlog` to `active`, assign the current user, and push directly to `develop` — no PR needed. This triggers the GH Pages board deploy.

## When to use

- Before starting implementation, to signal the team via the board
- Status-only update — no code changes, just lore task metadata

## Steps

### 1. Identify task

- `/promote-task` — uses current task from `lore/0-session/current-task.md`
- `/promote-task 0042` — uses task by ID (finds it in `lore/1-tasks/`)

Parse YAML frontmatter to extract `id`, `title`, `type`, `status`.

If task is already `active`, **STOP** — nothing to do.

### 2. Update task status via /lore-framework-tasks

**IMPORTANT:** Use `/lore-framework-tasks` to update the task status. Never edit frontmatter manually.

The update must:

- Change `status: backlog` → `status: active`
- Set `assignee` to current user (from `lore/0-session/current-user.md`)
- `git mv` the task file from `backlog/` to `active/`
- Add history entry with date, status `active`, who, and note

### 3. Ensure on develop

```bash
git stash  # if needed
git checkout develop
git pull origin develop
```

### 4. Commit via /lore-framework-git

Use `/lore-framework-git` conventions:

```
chore(lore-NNNN): activate task
```

Stage **only** the lore task file changes (the moved file).

### 5. Push to develop

```bash
git push origin develop
```

This triggers the `deploy-board.yml` workflow which regenerates `board.json` and deploys to GH Pages.

### 6. Confirm

Print:

> Task {id} activated and pushed to develop. Board will update shortly.

### 7. Return to previous branch (if applicable)

If you were on a different branch before, switch back:

```bash
git stash pop  # if stashed
git checkout {previous-branch}
```

## Arguments

- `/promote-task` — promote current task
- `/promote-task 0042` — promote task 0042

## Scope guard

This skill **only** touches lore task files. If `git status` shows non-lore changes, stash them first and restore after.

## Status convention

The canonical statuses in lore-framework are: `backlog`, `active`, `blocked`, `completed`.

**Never use `done`** — always use `completed`.
