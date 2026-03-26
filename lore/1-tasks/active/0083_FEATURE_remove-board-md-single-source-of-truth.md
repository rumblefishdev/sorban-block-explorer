---
id: '0083'
title: 'Remove BOARD.md — lore folder as single source of truth'
type: FEATURE
status: active
priority: high
layer: tooling
assignee: 'stkrolikiewicz'
related_adr: []
related_tasks: ['0082']
tags: ['phase-tooling', 'effort-small', 'priority-high']
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
---

# Remove BOARD.md — lore folder as single source of truth

## Summary

Remove committed `lore/BOARD.md` from the repo. The lore task files (`lore/1-tasks/`) are the single source of truth. `board.json` is already generated at deploy time for GitHub Pages — `BOARD.md` is redundant and causes merge conflicts on nearly every PR.

## Context

Currently `generate-lore-board.mjs` produces two outputs:

1. `lore/BOARD.md` — markdown board, committed to repo
2. `lore/board.json` — JSON data for HTML board, gitignored

Every PR that touches task files regenerates `BOARD.md`, leading to frequent merge conflicts that add no value. The HTML board on GitHub Pages (fed by `board.json`) is the real visualization layer.

## Implementation

1. **Delete `lore/BOARD.md`** from the repo (move to `.trash/`)
2. **Add `lore/BOARD.md` to `.gitignore`** so it's never committed again
3. **Update `generate-lore-board.mjs`** — keep generating `BOARD.md` locally (useful for local preview) but it should be clear it's a local-only artifact
4. **Update `README.md`** — remove the `[Board (Markdown)](lore/BOARD.md)` link, keep only the GitHub Pages link
5. **Update `.claude/skills/pr/SKILL.md`** — remove all references to staging/committing `BOARD.md` in PR workflows
6. **Update deploy workflow** — ensure `deploy-board.yml` only needs `board.json` (already the case, just verify)

## Acceptance Criteria

- [ ] `lore/BOARD.md` is not tracked by git (gitignored)
- [ ] `generate-lore-board.mjs` still generates both files locally
- [ ] GitHub Pages deploy still works (only uses `board.json`)
- [ ] `README.md` no longer links to `BOARD.md`
- [ ] PR skill no longer references committing `BOARD.md`
- [ ] No merge conflicts from board regeneration in future PRs
