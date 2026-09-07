---
id: '0529'
title: 'SUPERSEDED by 0531 — drop the vestigial nfts.minted_at_ledger column'
type: REFACTOR
status: completed
related_adr: ['0044']
related_tasks: ['0528', '0531', '0310']
tags: ['nft', 'clickhouse', 'dead-columns', 'ops', 'superseded', 'priority-low']
links: []
history:
  - date: '2026-09-01'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0528 future work. Once 0528 serves `minted_at_ledger` from
      `nft_ownership`, the stored column is unread and can go. Held back from
      0528 deliberately: this half needs a prod `ALTER TABLE`, and the driver's
      two-way schema validation makes the deploy ordering the hard part, not
      the code.
  - date: '2026-09-01'
    status: completed
    who: karolkow
    note: >
      SUPERSEDED — folded into 0531, nothing implemented under this id. 0531
      established that five other Tier-1 columns carry the same defect, so
      dropping this one column on its own would buy one production `ALTER`
      window for a fraction of the dead weight, and the remaining columns would
      need a second window later. The `ALTER` is the expensive, risky part (0310
      cost ~9 minutes of ingest stall), so it is paid once, for every dead
      column at once, in 0531 step 5. The deploy-ordering analysis written here
      moved there verbatim.
---

# SUPERSEDED by 0531 — drop the vestigial nfts.minted_at_ledger column

> **This task is not to be worked.** Its scope is 0531 step 4–5. It is kept
> only so the id resolves and the reasoning is traceable. See
> `0531_BUG_tier1-min-columns-corrupt-on-live-ingest-not-just-backfill.md`.

## Summary

After 0528, `nfts.minted_at_ledger` (and its twin in `nfts_pending`) is written
by the indexer and read by nobody. Remove it from the row struct, the staging
merge, the schema and `repair-tier1`, then drop it in prod.

## Context

The column is not merely redundant — it is actively wrong ~30 times a day,
because `nfts` is `ReplacingMergeTree(current_owner_ledger)` and any post-mint
event replaces the whole row with one carrying `NULL`. 0528 removes the last
reader; this task removes the writer and the storage.

It joins `name`, `media_url` and `collection_name` in the same table, which the
schema comment already labels vestigial with the DROP deferred to a cleanup
task. Consider batching all four into one `ALTER` rather than paying two prod
windows — the same consolidation 0310 applied to the `assets` columns.

## Implementation

- Strip the field from `NftRow` / `NftPendingRow` (`db-clickhouse/src/persist/rows.rs`)
  and from the staging merge (`persist/stage.rs`) — including the in-batch
  `min()` fold, which becomes dead with it.
- Drop the `nfts` / `nfts_pending` half of `repair_tier1::rebuild_nfts`. The
  `nft_ownership` fact table it reads from stays untouched.
- Remove the column from `db-clickhouse/schema/init.sql`.
- Prod `ALTER TABLE nfts DROP COLUMN minted_at_ledger` (+ `nfts_pending`),
  operator-run.

## Deploy ordering — the actual risk

This is where 0310 cost ~9 minutes of ingest stall, and the same trap applies
here. The `clickhouse` 0.15 driver validates the row struct against
`DESCRIBE TABLE` **in both directions**:

- slimmed struct + column still present (and no `DEFAULT`) → client-side
  `SchemaMismatch`, inserts fail
- struct still carrying the field + column already dropped → also a mismatch

So neither "deploy first" nor "ALTER first" is safe on its own, and warm Lambda
containers additionally cache the `DESCRIBE` result — 0310 needed a config-touch
recycle before the fleet picked up the new shape.

**Untested idea worth trying before scheduling a window:** give the column an
explicit `DEFAULT` first
(`ALTER TABLE nfts MODIFY COLUMN minted_at_ledger Nullable(Int64) DEFAULT NULL`).
0310's failure was specifically _"table columns without DEFAULT not covered by
the struct"_, so a defaulted column may let the slimmed struct pass while the
column still exists — collapsing the window to zero. **Verify against the driver
before relying on it**; if it does not hold, fall back to 0310's sequence
(deploy → ALTER immediately after → recycle containers) and accept the short
stall.

## Acceptance Criteria

- [ ] Field gone from the row structs, the staging merge, `repair-tier1` and
      `init.sql`
- [ ] Ingest verified healthy after the prod `ALTER` — no `SchemaMismatch`, no
      ledger gap
- [ ] `GET /v1/nfts` and `GET /v1/nfts/:id` still serve the mint ledger (0528's
      derivation is the only source now)
- [ ] **Docs updated** — schema docs under `docs/architecture/database-schema/**`
      drop the column
- [ ] **API types regenerated** — `N/A` expected (wire shape unchanged by 0528);
      confirm with an actual empty diff, do not assume

## Notes

- Prod `ALTER` and the deploy are the operator's to run, not the agent's.
- Blocked on 0528 landing and being verified in prod. Dropping the column while
  anything still reads it turns a stale value into a hard failure.
