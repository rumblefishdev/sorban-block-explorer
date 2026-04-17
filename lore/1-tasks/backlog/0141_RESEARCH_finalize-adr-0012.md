---
id: '0141'
title: 'Finalize ADR 0012 (proposed → accepted)'
type: RESEARCH
status: backlog
related_adr: ['0011', '0012']
related_tasks: ['0140', '0142']
tags:
  [layer-db, layer-architecture, priority-high, effort-medium, adr-finalization]
milestone: 1
links:
  - lore/2-adrs/0012_zero-upsert-schema-full-fk-graph.md
  - lore/2-adrs/0011_s3-offload-lightweight-db-schema.md
history:
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from task 0140 audit. Umbrella blocker for 0142 (schema migration)
      and every task flagged "pending-adr-0012-rewrite".
---

# Finalize ADR 0012 (proposed → accepted)

## Summary

ADR 0012 lands in `proposed` status. Move it to `accepted` by resolving remaining
open schema questions, confirming partition strategy details, validating parallel
backfill invariants, and signing off activity-projection write amplification.
Blocks all downstream implementation work gated by schema migration (0142).

## Status: Backlog

**Current state:** Not started. Awaits team review of ADR 0012.

## Context

ADR 0012 introduces a significant redesign:

- Zero-upsert + insert-only history tables
- Activity projections (`account_activity`, `token_activity`, `_current` tables,
  `contract_stats_daily`, `search_index`)
- Full FK graph with `ledgers` as dimension (no incoming FKs)
- `operations` repartitioned by `created_at`
- S3 offload with `ledger_metadata` header + `nft_metadata` array
- Identity-first parallel backfill with progressive COALESCE fill
- Deferred post-backfill `CREATE INDEX CONCURRENTLY` + BRIN + partial indexes

Task 0140's audit marks 21 tasks as blocked by migration (0142) and 22 archived
tasks as reference-only. This finalization task ensures the ADR is stable enough
to justify those downstream commitments.

## Open questions to resolve before acceptance

1. **Cursor stability at BIGSERIAL cutover** — ADR 0008 addendum. Decide: migrate
   IDs stably, include entity discriminator in cursor, or invalidate outstanding
   cursors at cutover? Impacts frontend tasks.
2. **Historical state query API** — tech design does not explicitly require
   "balance @ ledger X" / "NFT owner @ ledger Y". ADR 0012 enables them via
   history tables. Decide: in-scope milestone 1, deferred milestone 2, or skip?
3. **Rollup Lambdas cadence** — `contract_stats_daily` (HLL merge for unique
   callers) and `liquidity_pool_current.volume_24h` schedules.
4. **Partition cron frequency for `operations`** — inherits daily cadence from
   0139 hotfix or stays monthly with larger partition sizes?
5. **`search_index` column structure** — absorb decisions from 0133 (pg_trgm vs
   TSVECTOR per entity type).
6. **Post-backfill index build orchestration** — Lambda vs one-off job; how to
   track progress; rollback if a CONCURRENTLY build fails.
7. **Monitoring healthcheck for `ledger_sequence` drift** — no-FK mitigation.
   CloudWatch alarm on `MAX(account_balances.ledger_sequence) > MAX(ledgers.sequence)`
   is proposed; verify query cost.
8. **`0143` historical-state-query API decision** — emit spawn decision.

## Implementation Plan

### Step 1 — Team review

Circulate ADR 0012 within the team. Collect written feedback on open questions
from fmazur, FilipDz, stkrolikiewicz.

### Step 2 — Resolve open questions

Write ADR addenda or inline updates for each of the 8 open questions. Update
`related_adrs` where applicable (e.g., ADR 0008 for cursor stability).

### Step 3 — Flip status to accepted

Update ADR 0012 `status: proposed` → `status: accepted` with history entry.
Populate `related_tasks` if any new tasks emerged during review.

### Step 4 — Unblock 0142

Update 0142 frontmatter: remove `blocked_by: ['0141']`, promote to active.

### Step 5 — Spawn follow-up tasks

For each in-scope open question that needs implementation beyond the migration:

- Rollup Lambda tasks (HLL, volume_24h)
- Post-backfill index build pipeline
- Monitoring healthcheck
- (Optional) `0143` historical-state-query API

## Acceptance Criteria

- [ ] All 8 open questions have a written resolution in the ADR or an addendum
- [ ] Team sign-off from fmazur, FilipDz, stkrolikiewicz
- [ ] `ADR 0012 status: accepted` with history entry
- [ ] `0143` decision recorded (in-scope / deferred / skip)
- [ ] Follow-up tasks spawned for rollup Lambdas, post-backfill index build, monitoring drift
- [ ] `0142` unblocked and ready to start
