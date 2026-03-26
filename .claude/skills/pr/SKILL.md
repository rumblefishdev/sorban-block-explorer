# /pr — Create a pull request from the active lore task

Create a GitHub PR with title and body derived from the active lore task and git diff.

## Steps

### 1. Read active task

Read `lore/0-session/current-task.md` (follows symlink). Parse YAML frontmatter to extract `id`, `title`, and `type`.

If no active task exists, **STOP** and ask the user to set one with `lore_set-task`.

### 2. Determine PR title

Map lore task `type` to Conventional Commits prefix:

| Task Type | Commit Prefix |
| --------- | ------------- |
| FEATURE   | `feat`        |
| RESEARCH  | `research`    |
| BUG       | `fix`         |
| REFACTOR  | `refactor`    |
| DOCS      | `docs`        |

Format: `{prefix}({id}): {short description}`

The short description should be a concise, lowercase summary derived from the task title — NOT a copy-paste of the full title. Keep it under 70 characters total.

Examples:

- `feat(0009): domain types for ledger and transaction`
- `research(0008): event interpreter patterns`
- `fix(0042): identifier display copy button`
- `chore(0080): assign tasks 0001-0008 to stkrolikiewicz` (status-only PR)

### 3. Generate PR body

Format:

```markdown
## Summary

- Bullet point 1 summarizing a key change
- Bullet point 2
- ...
```

Derive the summary bullets from `git diff {base}...HEAD --stat` and commit messages on the branch. Keep it to 3-5 bullets max. Focus on WHAT changed, not HOW.

### 4. Determine base branch

Check if `develop` branch exists (local or remote). If yes, use `develop`. Otherwise use `master`.

### 5. Verify

Before pushing, run format and verify checks:

```bash
npm run -s format:staged
npm run -s verify:staged
```

If checks fail, fix the issues and amend the commit before proceeding.

### 6. Push and create PR

```bash
git push -u origin {current-branch}
gh pr create --base {base} --title "{title}" --body "{body}"
```

### 7. Confirm

Print the PR URL.

## Arguments

- If the user provides `--base <branch>`, use that as base branch.
- If the user provides `--draft`, create as draft PR.
- If the user provides a custom title after `/pr`, use it instead of auto-generating.

## Conventions

- PR title is designed to be the squash merge commit message
- Title follows Conventional Commits format
- Body uses only `## Summary` section with bullet points
- No test plan section unless explicitly requested

## Task status in PRs

Tasks in **any status** can be committed — including `active`. This enables a two-phase workflow:

### Status-only updates (no PR needed)

To activate a task (backlog → active), use `/promote-task` instead of a PR. It pushes directly to develop for board deployment.

### Implementation PR

Create the implementation branch and work on the task.

- Branch: `feat/{id}_{slug}` (via `/branch`)
- PR title: `feat({id}): description of implementation`
- Changes: code + task moved to archive when completed

### Single-phase (small tasks)

For small tasks, both phases can be combined in one PR — change status to active, implement, move to archive.

## Task Status Updates

**IMPORTANT:** All task status changes MUST go through the `/lore-framework-tasks` skill. Never update task frontmatter (status, history, `git mv` between directories) manually.

- Use `/lore-framework-tasks` to mark task as `completed` and move to `archive/`
- The canonical status value is `completed` (NOT `done`) — follow lore-framework conventions
- After the PR is merged, the task status update is already committed as part of the branch
