---
id: '0079'
title: 'Create /branch and /pr Claude Code skills for lore-aware git workflow'
type: FEATURE
status: completed
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-tooling]
links: []
history:
  - date: 2026-03-25
    status: active
    who: stkrolikiewicz
    note: 'Task created and started'
  - date: 2026-03-25
    status: completed
    who: stkrolikiewicz
    note: 'Both skills created and tested'
---

# Create /branch and /pr Claude Code skills for lore-aware git workflow

## Summary

Create two Claude Code skills that enforce consistent naming conventions for branches and PRs, deriving names from lore task metadata.

## Acceptance Criteria

- [x] `/branch` skill creates a branch named `type/id_slug` from active lore task
- [x] `/pr` skill creates a PR with Conventional Commits title `type(id): description` and summary body
- [x] Both skills read task metadata from lore frontmatter
- [x] PR title suitable as squash merge commit message
- [x] Branch type maps from lore task type (FEATURE→feat, RESEARCH→research, BUG→fix, REFACTOR→refactor, DOCS→docs)
