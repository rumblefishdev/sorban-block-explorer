---
id: '0161'
title: 'BUG: native asset singleton never seeded in assets table'
type: BUG
status: backlog
related_adr: ['0027', '0036']
related_tasks: ['0120', '0154', '0160']
tags: [priority-medium, effort-small, layer-db, layer-indexer, audit-gap]
milestone: 1
links:
  - crates/db/migrations/0005_tokens_nfts.sql
  - crates/xdr-parser/src/state.rs
  - crates/indexer/src/handler/persist/write.rs
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from post-0154 audit alongside 0160. `assets` table
      misses the native singleton — no code path emits it, no
      migration seeds it. Singleton is required by schema
      (uidx_assets_native as singleton UNIQUE).
---

# BUG: native asset singleton never seeded in assets table

## Summary

`assets.asset_type = 0` (native / XLM) is modelled as a singleton via
`uidx_assets_native ON assets ((asset_type)) WHERE asset_type = 0`.
The row is required for any `/assets` listing to include XLM, for
`account_balances_current` foreign-key joins that expect a registry
row, and for the frontend asset detail page to resolve XLM. Nothing
today produces this row:

- `xdr_parser::detect_assets` has no native branch — only SAC and
  Fungible WASM (`state.rs:513-543`).
- Migration `0005_tokens_nfts.sql` creates `assets` schema but does
  not INSERT the native row.
- `upsert_assets_native` (`write.rs:1040-1068`) writes only when
  `asset_rows` contains a `TokenAssetType::Native` entry — which
  never happens because nothing upstream emits one.

Net effect: **`assets` has no native row, ever.** Pre-existing gap;
not caused by 0120 or 0154.

## Context

Two possible fix shapes:

1. **Migration seed** — add
   `INSERT INTO assets (asset_type, name) VALUES (0, 'Stellar
Lumens')` to `0005_tokens_nfts.sql` or a new migration. Runs
   once, never replays. Simple. No indexer change.
2. **Per-ledger upsert** — in `persist_ledger`, on first ledger
   (or every ledger, idempotent via
   `INSERT … WHERE NOT EXISTS`), insert the singleton. Handles
   fresh DBs without a migration rebuild.

Option 1 is cleaner for clean-slate deployments. Option 2 is
defensive if DBs exist that missed the seed. Pick 1 as primary,
keep 2 as mental fallback — we're pre-production so a migration
rewrite is cheap.

Related: 0160 covers the SAC edge case where an XLM-wrapped SAC
needs to reference the same native asset. If 0160 lands option (c)
(synthesise `asset_code = "XLM"`), this task's seed values must
match that convention. Coordinate.

## Implementation

### Primary: migration seed

Edit `crates/db/migrations/0005_tokens_nfts.sql` (pre-production, no
live DB to worry about — same precedent as 0154 editing base
migrations in place):

```sql
-- Seed the native XLM singleton. `name` is the human label;
-- holder_count is populated by task 0135 once trustline tracking
-- starts counting native balances.
INSERT INTO assets (asset_type, name)
VALUES (0, 'Stellar Lumens');
```

### Alternative: runtime idempotent upsert

If the migration path is rejected, add to `write.rs` a
`seed_native_asset` step run once per persist tx:

```sql
INSERT INTO assets (asset_type, name)
SELECT 0, 'Stellar Lumens'
WHERE NOT EXISTS (SELECT 1 FROM assets WHERE asset_type = 0)
```

Runs before `upsert_assets` so per-ledger native updates (if any —
e.g. holder_count from 0135) can `DO UPDATE` cleanly.

### Tests

- **Integration** (`persist_integration.rs`): fresh DB after
  migrations → `assets` contains exactly one row with `asset_type = 0`
  and `name = 'Stellar Lumens'`.
- **Integration**: replay / second ledger → still exactly one native
  row (no duplicate, no UNIQUE violation).

## Acceptance Criteria

- [ ] Native asset singleton exists in `assets` after migrations run.
- [ ] `name = 'Stellar Lumens'` (or agreed display name).
- [ ] Idempotent — re-running migrations / replaying ledgers does not
      produce duplicates or violate `uidx_assets_native`.
- [ ] Integration test covers presence + idempotency.
- [ ] If runtime path chosen (alternative), documented in task notes
      and ordered before `upsert_assets` in `persist/mod.rs`.

## Notes

Audit gap. Effort small (one INSERT + one test). Coordinate with
0160 on XLM-SAC identity convention so the two native references
agree.
