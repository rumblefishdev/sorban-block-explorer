---
id: '0081'
title: 'Fix skill directory structure and deploy-board workflow duplicate artifacts'
type: BUG
status: active
related_adr: []
related_tasks: ['0079', '0080']
tags: [priority-high, effort-small, layer-tooling]
links: []
history:
  - date: 2026-03-25
    status: active
    who: stkrolikiewicz
    note: 'Skills not loading due to wrong file structure; deploy-board fails with duplicate artifacts'
---

# Fix skill directory structure and deploy-board workflow duplicate artifacts

## Summary

Two issues from task 0079/0080:

1. `/branch` and `/pr` skills were flat `.md` files — Claude Code requires `skills/<name>/SKILL.md` directory structure
2. `deploy-board.yml` fails with "Multiple artifacts named github-pages" — needs separate build and deploy jobs

## Acceptance Criteria

- [ ] Skills restructured to `.claude/skills/pr/SKILL.md` and `.claude/skills/branch/SKILL.md`
- [ ] Both skills appear in Claude Code `/` autocomplete
- [ ] `deploy-board.yml` uses separate build and deploy jobs
- [ ] Board deploys successfully to GitHub Pages after merge to develop
