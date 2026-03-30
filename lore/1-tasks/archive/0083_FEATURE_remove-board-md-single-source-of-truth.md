---
id: '0083'
title: 'Remove BOARD.md — lore folder as single source of truth'
type: FEATURE
status: completed
priority: high
layer: tooling
assignee: 'stkrolikiewicz'
related_adr: []
related_tasks: ['0082']
tags: ['phase-tooling', 'effort-small', 'priority-high']
milestone: 1
links: []
history:
  - date: '2026-03-26'
    status: backlog
    who: stkrolikiewicz
    note: >
      Created. BOARD.md in lore/ causes merge conflicts on every PR
      because it's auto-generated and committed. We want lore/ task files
      to be the single source of truth and generate board.json only at
      deploy time.
  - date: '2026-03-26'
    status: completed
    who: stkrolikiewicz
    note: >
      Removed BOARD.md from git, stripped generateMarkdown() from
      generate-lore-board.mjs (only board.json now), updated PR/branch
      skills, created new /promote-task skill. 6 files changed.
      PR #15.
---

# Remove BOARD.md — lore folder as single source of truth

## Summary

Remove committed `lore/BOARD.md` from the repo. The lore task files (`lore/1-tasks/`) are the single source of truth. `board.json` is already generated at deploy time for GitHub Pages — `BOARD.md` is redundant and causes merge conflicts on nearly every PR.

## Context

Previously `generate-lore-board.mjs` produced two outputs: `lore/BOARD.md` (committed) and `lore/board.json` (gitignored). Every PR that touched task files regenerated `BOARD.md`, leading to frequent merge conflicts.

## Acceptance Criteria

- [x] `lore/BOARD.md` is not tracked by git (gitignored)
- [x] `generate-lore-board.mjs` generates only `board.json`
- [x] GitHub Pages deploy still works (only uses `board.json`)
- [x] `README.md` no longer links to `BOARD.md`
- [x] PR skill no longer references committing `BOARD.md`
- [x] No merge conflicts from board regeneration in future PRs

## Implementation Notes

- `lore/BOARD.md` — removed from git tracking, added to `.gitignore`
- `tools/scripts/generate-lore-board.mjs` — removed `generateMarkdown()`, `LAYER_EMOJI`, `STATUS_EMOJI`; now only generates `board.json`
- `README.md` — removed markdown board link, kept GH Pages link
- `.claude/skills/pr/SKILL.md` — removed board regeneration step, added task status section mandating `/lore-framework-tasks`, enforced `completed` convention
- `.claude/skills/branch/SKILL.md` — removed `--status-only` flag, simplified workflow
- `.claude/skills/promote-task/SKILL.md` — new skill: activates task and pushes to develop without PR

## Design Decisions

### From Plan

1. **Gitignore BOARD.md instead of deleting the generation code**: Originally planned to keep generating locally. Changed per user feedback — removed markdown generation entirely since only `board.json` is needed.

### Emerged

2. **Created /promote-task skill**: User requested a way to push task status changes directly to develop without PR, replacing the old Phase 1 status-only PR workflow.

3. **Enforced `completed` over `done`**: Added explicit convention to `/pr` skill since inconsistent status values were being used.

4. **Removed --status-only from /branch**: No longer needed since `/promote-task` handles status-only changes directly on develop.
