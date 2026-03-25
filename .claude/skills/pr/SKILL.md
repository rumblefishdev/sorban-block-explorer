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

### 5. Regenerate board (if tasks changed)

If `git diff {base}...HEAD -- lore/1-tasks/` shows changes, regenerate the board:

```bash
npm run board
```

If `lore/BOARD.md` changed, stage and amend the last commit.

### 6. Verify

Before pushing, run format and verify checks:

```bash
npm run -s format:staged
npm run -s verify:staged
```

If checks fail, fix the issues and amend the commit before proceeding.

### 7. Push and create PR

```bash
git push -u origin {current-branch}
gh pr create --base {base} --title "{title}" --body "{body}"
```

### 8. Confirm

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

### Phase 1: Status-only PR (pre-implementation)

Pick a task, change its status (backlog → active), assign yourself. Create a `chore` branch and PR so the team sees the update in the board after merge.

- Branch: `chore/{id}_{slug}` (via `/branch --status-only`)
- PR title: `chore({id}): assign and activate task`
- Changes: only task frontmatter (status, history) + regenerated BOARD.md

### Phase 2: Implementation PR

After the status PR is merged, create the implementation branch and work on the task.

- Branch: `feat/{id}_{slug}` (via `/branch`)
- PR title: `feat({id}): description of implementation`
- Changes: code + task moved to archive when done + BOARD.md

### Single-phase (small tasks)

For small tasks, both phases can be combined in one PR — change status to active, implement, move to archive.
