---
id: '0144'
title: 'Cleanup: consolidate 0118 task format (dir + .md duplicate)'
type: REFACTOR
status: backlog
related_adr: []
related_tasks: ['0118', '0140']
tags: [layer-docs, priority-low, effort-small, lore-hygiene]
milestone: 1
links: []
history:
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from task 0140 audit. 0118 exists as both a `.md` file and a directory
      with `sources/` — violates lore framework single-format rule.
---

# Cleanup: consolidate 0118 task format

## Summary

Task `0118_BUG_nft-false-positives-fungible-transfers` exists simultaneously in
two formats:

- `lore/1-tasks/active/0118_BUG_nft-false-positives-fungible-transfers.md`
- `lore/1-tasks/active/0118_BUG_nft-false-positives-fungible-transfers/sources/`

Per `lore/1-tasks/CLAUDE.md`, a task is either a file OR a directory, not both.
The directory currently holds only SEP-0041 / SEP-0050 protocol sources — no
`README.md`, no `notes/`. Consolidate to directory format.

## Implementation

1. Create `lore/1-tasks/active/0118_BUG_nft-false-positives-fungible-transfers/README.md`
   with content copied verbatim from
   `lore/1-tasks/active/0118_BUG_nft-false-positives-fungible-transfers.md`.
2. Delete the standalone `.md` file (move to `.trash/` per project policy, don't `rm`):
   `mv lore/1-tasks/active/0118_BUG_nft-false-positives-fungible-transfers.md .trash/`
3. Verify symlink `lore/0-session/current-task.md` still resolves correctly if it
   pointed at the old path.
4. Re-run `lore_generate-index` to refresh the index.

## Acceptance Criteria

- [ ] `README.md` present in `0118_BUG_nft-false-positives-fungible-transfers/`
- [ ] Standalone `.md` removed
- [ ] Task content unchanged (byte-for-byte except file move)
- [ ] Any incoming symlinks or references still resolve
- [ ] Index regenerated

## Notes

Low priority. Does not block any other task. Clean up when convenient.
