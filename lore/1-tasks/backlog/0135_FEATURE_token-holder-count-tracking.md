---
id: '0135'
title: 'Indexer: ongoing token holder_count tracking'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0027', '0049', '0119']
tags: [priority-medium, effort-medium, layer-indexer, layer-db, audit-gap]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
  - docs/architecture/technical-design-general-overview.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design requires holder_count on token list/detail but no mechanism populates it. Always NULL.'
---

# Indexer: ongoing token holder_count tracking

## Summary

The technical design specifies `holder_count` in both the token list table and token detail
page. The column exists in the schema (`tokens.holder_count INTEGER`) but is never populated
— it is always NULL. The indexer's `detect_tokens()` does not compute holder counts, and
there is no ongoing mechanism to update them as trustline/balance changes occur.

## Context

Holder count cannot be extracted from a single `LedgerCloseMeta` XDR — it requires knowing
the total number of accounts holding a non-zero balance of a given token. This is either:

1. A full DB aggregation (count distinct accounts with trustline to this asset)
2. An incremental counter updated on every trustline create/remove event

Option 2 is more efficient at scale but requires task 0119 (trustline balance extraction)
to be implemented first, since trustline entries are the source of holder state changes.

## Implementation

**Option A — Incremental counter (recommended):**

1. During trustline extraction (task 0119), detect when a trustline is created (new holder)
   or removed (lost holder) for a token.
2. Increment/decrement `tokens.holder_count` atomically:
   `UPDATE tokens SET holder_count = COALESCE(holder_count, 0) + 1 WHERE ...`
3. After historical backfill, run a one-time correction query to set accurate counts.

**Option B — Periodic aggregation:**

1. Scheduled job (EventBridge + Lambda) that runs:
   `UPDATE tokens SET holder_count = (SELECT COUNT(DISTINCT account_id) FROM ... WHERE ...)`
2. Simpler but expensive at scale and always slightly stale.

**Option C — Materialized view:**

1. Create a materialized view counting holders per token.
2. Refresh on schedule or trigger.

## Acceptance Criteria

- [ ] `tokens.holder_count` populated for classic assets (trustline-based)
- [ ] `tokens.holder_count` updated incrementally on trustline create/remove
- [ ] Holder count visible in `GET /tokens` list and `GET /tokens/:id` detail
- [ ] One-time backfill correction after historical ingestion
- [ ] Test: token with 3 holders shows holder_count = 3
