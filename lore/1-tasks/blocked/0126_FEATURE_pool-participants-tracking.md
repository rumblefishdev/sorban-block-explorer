---
id: '0126'
title: 'LP: pool participants and share tracking'
type: FEATURE
status: blocked
related_adr: []
related_tasks: ['0052', '0077', '0136']
blocked_by: ['0136']
tags: [priority-low, effort-medium, layer-indexer, layer-db, audit-gap]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design specifies pool participants table on LP detail page but no schema exists.'
---

# LP: pool participants and share tracking

## Summary

The technical design specifies a "Pool participants" section on the LP detail page showing
liquidity providers and their share. No per-provider tracking exists in the current schema.

## Implementation

1. Create `liquidity_pool_participants` table (pool_id, account_id, shares, last_updated).
2. Track pool share changes from `LedgerEntryChanges` — trustline entries for pool shares.
3. Alternatively, derive from `soroban_events` or `soroban_invocations` for deposit/withdraw
   activity.

## Acceptance Criteria

- [ ] Per-provider pool shares trackable
- [ ] API endpoint returns participants for a given pool
- [ ] Shares updated on deposit/withdrawal events
