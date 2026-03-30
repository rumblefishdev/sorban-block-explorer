---
id: '0081'
title: 'Fix skill directory structure and deploy-board workflow duplicate artifacts'
type: BUG
status: completed
related_adr: []
related_tasks: ['0079', '0080']
tags: [priority-high, effort-small, layer-tooling]
milestone: 1
links: []
history:
  - date: 2026-03-25
    status: active
    who: stkrolikiewicz
    note: 'Skills not loading due to wrong file structure; deploy-board fails with duplicate artifacts'
  - date: 2026-03-25
    status: completed
    who: stkrolikiewicz
    note: 'Both fixes applied. Skills load in autocomplete, deploy workflow split into build+deploy jobs.'
---

# Fix skill directory structure and deploy-board workflow duplicate artifacts

## Summary

Two issues from task 0079/0080:

1. `/branch` and `/pr` skills were flat `.md` files — Claude Code requires `skills/<name>/SKILL.md` directory structure
2. `deploy-board.yml` fails with "Multiple artifacts named github-pages" — needs separate build and deploy jobs

## Acceptance Criteria

- [x] Skills restructured to `.claude/skills/pr/SKILL.md` and `.claude/skills/branch/SKILL.md`
- [x] Both skills appear in Claude Code `/` autocomplete
- [x] `deploy-board.yml` uses separate build and deploy jobs (done in 0080)
- [x] Board deploys successfully to GitHub Pages after merge to develop (confirmed in 0080)

## Implementation Notes

- Claude Code skills must be directories with `SKILL.md`, not flat `.md` files in `skills/`
- First line `# /name — description` auto-generates the skill description, no YAML frontmatter needed
- `Skill(pr)` and `Skill(branch)` added to `.claude/settings.json` allow list

## Design Decisions

### Emerged

1. **No YAML frontmatter needed**: Claude Code reads the first `# /name — description` heading as skill metadata. Simpler than adding frontmatter.
