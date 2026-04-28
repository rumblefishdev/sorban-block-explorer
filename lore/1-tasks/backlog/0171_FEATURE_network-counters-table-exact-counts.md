---
id: '0171'
title: 'Indexer: maintain network_counters table for exact entity counts'
type: FEATURE
status: backlog
related_adr: ['0021', '0037']
related_tasks: ['0045', '0167']
tags: [layer-indexer, counters, exactness, phase-future]
links:
  - 'docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql'
history:
  - date: 2026-04-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from PR #125 (task 0045) review. Activate only if measured
      drift between `pg_class.reltuples` and ground-truth counts becomes
      user-visible on the home dashboard. Anticipated explicitly by the
      canonical SQL header comment in `01_get_network_stats.sql`:
      "If exact is ever needed, spawn a periodic counter table — do NOT
      add COUNT(*) here."
---

# Indexer: maintain network_counters table for exact entity counts

## Summary

`/network/stats` returns `total_accounts` and `total_contracts` from
`pg_class.reltuples` per the canonical SQL deliverable of task 0167
(`docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`).
That file's header comment explicitly anticipates this task: _"If exact is
ever needed, spawn a periodic counter table — do NOT add COUNT(_) here."\*

Activate only if measured drift between the reltuples estimate and ground
truth becomes user-visible (UI feedback, partner reporting, press
discrepancies vs. stellar.expert / stellarchain.io). Until then, the
~99% accuracy of `reltuples` is sufficient for an overview-card UI value.

## Context

Deferred from PR #125 alignment work. The reasoning chain:

1. `count(*)` on partition-sized tables is a 5–15 s seq scan at mainnet
   scale — unacceptable for a hot dashboard endpoint.
2. `pg_class.reltuples` is microseconds and ~99% accurate post-VACUUM-ANALYZE.
3. If product ever requires exact counts, the counter table approach
   below is the next step — not adding `COUNT(*)` back to the hot path.

Activation gate: record observed drift, the trigger that made it
user-visible, and whether tuning autovacuum / ANALYZE frequency would
have closed the gap before resorting to a counter table.

## Implementation Plan

1. **Migration.** Add `network_counters` table:

   ```sql
   CREATE TABLE network_counters (
       name        VARCHAR(64) PRIMARY KEY,
       value       BIGINT      NOT NULL,
       updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
   );
   ```

2. **Indexer hook.** In `crates/indexer` account/contract upsert paths,
   increment the relevant counter atomically with the entity insert
   (same transaction). First-seen rows only — duplicates do not bump.
3. **API switch.** Replace `pg_class.reltuples` queries in
   `crates/api/src/network/queries.rs` with single-row PK lookups
   against `network_counters`.
4. **Backfill.** One-shot job at deployment populates initial values
   from `count(*)`; from then on, incremental maintenance via the
   indexer hook.
5. **Update canonical SQL.** Edit
   `docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`
   to read from `network_counters` instead of `pg_class.reltuples`;
   remove the "do NOT add COUNT(\*) here" guidance comment as it no
   longer applies.

## Acceptance Criteria

- [ ] Migration applied on staging + production
- [ ] Indexer increments counters atomically with entity insert
      (single transaction, no possibility of counter drift on retry)
- [ ] Backfill job populates initial values once at deployment
- [ ] `/network/stats` reads from counter table (microsecond lookup)
- [ ] Canonical `01_get_network_stats.sql` updated to match
- [ ] Unit test: counter increments on first-seen account/contract,
      does NOT bump on duplicate insert
- [ ] **Docs updated** — `docs/architecture/database-schema/database-schema-overview.md`
      adds `network_counters` table; `docs/architecture/indexing-pipeline/*.md`
      notes the counter maintenance hook per
      [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md)

## Notes

Skip until measured drift is user-visible. Premature activation =
unnecessary indexer complexity for a UI value where ~99% accuracy
suffices. If autovacuum tuning or scheduled `ANALYZE` could close the
gap with no schema changes, prefer that path first.
