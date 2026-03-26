---
id: '0082'
title: 'Lore board: task detail modal on card click'
type: FEATURE
status: completed
related_adr: []
related_tasks: []
tags: [priority-medium, effort-small, layer-tooling]
links: []
history:
  - date: 2026-03-26
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-26
    status: active
    who: fmazur
    note: 'Promoted to active for implementation'
  - date: 2026-03-26
    status: completed
    who: fmazur
    note: >
      Implemented task detail modal in board.html. Extended
      generate-lore-board.mjs with description extraction.
      3 files changed. All acceptance criteria met.
---

# Lore board: task detail modal on card click

## Summary

Add a modal to `lore/board.html` that opens when clicking a task card (kanban or table row). The modal displays the full task details from `board.json` in a well-organized layout matching the board's dark theme.

## Status: Completed

**Current state:** Fully implemented and tested locally.

## Context

The board currently shows task cards with minimal info (ID, title, layer, priority, type). Clicking a card does nothing. Users need to leave the board and find the task file to see full details like description, history, blockers, and related tasks. A modal would let users inspect task details without leaving the board view.

## Acceptance Criteria

- [x] Clicking a task card in kanban view opens a detail modal
- [x] Clicking a task row in table view opens a detail modal
- [x] Modal shows: ID, title, layer, priority, type, status, assignee
- [x] Modal shows: description/summary text
- [x] Modal shows: history timeline (date, who, status, note)
- [x] Modal shows: blockers and related tasks (if present)
- [x] Modal matches the board's dark theme
- [x] Modal closes on backdrop click, X button click, or Escape key
- [x] Body scroll is prevented while modal is open
- [x] Empty/missing fields are handled gracefully (no empty sections shown)

## Implementation Notes

**Files changed:**

- `lore/board.html` — Added modal HTML structure, CSS styles (~120 lines), and JS logic (`openModal`, `closeModal`, `escapeHtml`). Click handlers on cards and table rows.
- `tools/scripts/generate-lore-board.mjs` — Added `extractDescription()` function that pulls text from `## Summary` section. Added `description` and `related_adr` fields to JSON output.
- `lore/BOARD.md` — Regenerated with updated task data.

**Modal sections:** Header (ID + title), tags row (layer/priority/type/status/assignee badges), description, related tasks, related ADRs, history timeline (reverse chronological).

## Design Decisions

### From Plan

1. **Reuse existing tag/badge styles**: Modal tags use the same CSS classes as card tags for visual consistency.
2. **Dark theme matching**: Modal uses existing CSS variables (`--bg`, `--surface`, `--border`).

### Emerged

3. **Description extraction via `## Summary` heading**: The generator parses the first paragraph under `## Summary` in task markdown. Fallback: first paragraph after frontmatter if no Summary heading found.
4. **Related ADRs as separate section**: Added `related_adr` to JSON output alongside `related_tasks`, shown as separate section in modal.
5. **History in reverse chronological order**: Most recent event first — more useful for quickly seeing current state.
6. **escapeHtml utility**: Added to prevent XSS from task content rendered in modal innerHTML.
