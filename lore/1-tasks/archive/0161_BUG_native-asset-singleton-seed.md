---
id: '0161'
title: 'BUG: native asset singleton never seeded in assets table'
type: BUG
status: completed
related_adr: ['0027', '0036', '0037']
related_tasks: ['0120', '0154', '0160']
tags: [priority-medium, effort-small, layer-db, audit-gap]
milestone: 1
links:
  - crates/db/migrations/20260428000000_seed_native_asset_singleton.up.sql
  - crates/db/migrations/20260428000000_seed_native_asset_singleton.down.sql
  - crates/indexer/tests/persist_integration.rs
  - docs/architecture/database-schema/database-schema-overview.md
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from post-0154 audit alongside 0160. `assets` table
      misses the native singleton — no code path emits it, no
      migration seeds it. Singleton is required by schema
      (uidx_assets_native as singleton UNIQUE).
  - date: '2026-04-27'
    status: active
    who: stkrolikiewicz
    note: >
      Promoted. Natural follow-up after 0160 (PR #120 closes "SAC
      identity" gap; this closes the sister "native singleton" gap from
      the same 2026-04-10 audit). Per 0160 lessons learned, will
      land as a forward-only migration (`20260428…_seed_native_asset_singleton`)
      rather than editing 0005 (sqlx checksum break).
  - date: '2026-04-27'
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped on `fix/0161_native-asset-singleton-seed`. One forward-only
      migration (up + down), one persist_integration test
      (`native_asset_singleton_seeded_after_migrations`), one-line addendum
      to `database-schema-overview.md` explaining the seed mechanism. 9/9
      persist_integration tests pass parallel against fresh DB; clippy
      clean. No code changes in xdr-parser or indexer crates. No new ADR
      (schema invariant + index already documented in ADR 0037; this is
      bootstrap data, not an architectural decision).
---

# BUG: native asset singleton never seeded in assets table

## Summary

`assets.asset_type = 0` (native / XLM) is modelled as a singleton via
`uidx_assets_native ON assets ((asset_type)) WHERE asset_type = 0`. The
row is required for any `/assets` listing to include XLM, for
`account_balances_current` foreign-key joins that expect a registry
row, and for the frontend asset detail page to resolve XLM. Nothing
upstream produces this row — `xdr_parser::detect_assets` has no native
branch, migration `0005` declares the table without seeding it, and
`upsert_assets_native` runs only when the parser emits a `Native`
entry (it never does). Pre-existing gap; not caused by 0120 or 0154.

Fix: forward-only DML migration `20260428000000_seed_native_asset_singleton`
that inserts the singleton row directly. Runs once per DB lifetime via
sqlx; integration test pins presence + identity shape after migration.

## Context

Two fix shapes considered (per backlog notes):

1. **Migration seed** — DML that inserts the row once.
2. **Per-ledger upsert** — `INSERT … WHERE NOT EXISTS` in `persist_ledger`.

Picked option 1, chose **new forward-only migration** over editing
`0005_tokens_nfts.sql` directly (the latter would break sqlx checksum
on every already-applied DB — same lesson as 0160 round 1).

## Implementation

One commit:

- `crates/db/migrations/20260428000000_seed_native_asset_singleton.up.sql` —
  `INSERT INTO assets (asset_type, name) VALUES (0, 'Stellar Lumen');`
- `crates/db/migrations/20260428000000_seed_native_asset_singleton.down.sql` —
  `DELETE FROM assets WHERE asset_type = 0;`
- `crates/indexer/tests/persist_integration.rs` — new test
  `native_asset_singleton_seeded_after_migrations` querying the row's
  `(asset_code, issuer_id, contract_id, name)` and asserting the
  singleton shape (NULL+NULL+NULL+`"Stellar Lumen"`).
- `docs/architecture/database-schema-overview.md` — one-line addendum
  in the assets §Design notes explaining the seed mechanism + the
  operator-deletion warning.

## Acceptance Criteria

- [x] Native asset singleton exists in `assets` after migrations run.
- [x] `name = 'Stellar Lumen'` (per user request — singular form chosen
      over the more common `'Stellar Lumens'` plural).
- [x] Idempotent — sqlx tracks `_sqlx_migrations`; migration runs
      exactly once per DB lifetime. Re-running tests via `npm run db:reset`
      drops the DB and re-applies, producing the same singleton.
- [x] Integration test covers presence + identity shape.
- [x] Runtime path NOT chosen (option 1 sufficient; option 2 documented
      as deferred fallback in Design Decisions).

## Implementation Notes

| File                                                                       | Δ                               |
| -------------------------------------------------------------------------- | ------------------------------- |
| `crates/db/migrations/20260428000000_seed_native_asset_singleton.up.sql`   | +13 (new)                       |
| `crates/db/migrations/20260428000000_seed_native_asset_singleton.down.sql` | +6 (new)                        |
| `crates/indexer/tests/persist_integration.rs`                              | +44 (one test + section header) |
| `docs/architecture/database-schema/database-schema-overview.md`            | +5 (Design notes addendum)      |

**Tests**: 9/9 `persist_integration` parallel (added 1, total +1 vs
post-0160). Clippy `--workspace --all-targets -- -D warnings` clean.

**Migrations**: one new file. Zero edits to existing migrations. Sqlx
checksum on all earlier migrations preserved.

## Issues Encountered

None.

## Design Decisions

### From Plan

1. **Forward-only migration over editing `0005`.** Same sqlx-checksum
   reason that drove the 0160 re-open; the playbook is "new migration
   file" for any post-anchor data change.

2. **Migration seed (option 1) over per-ledger upsert (option 2).**
   Singleton is bootstrap data — installed once per DB lifetime. A
   per-ledger upsert would add CPU cost on every ledger to defend a
   case (operator manually deleted the row) that's outside parser
   responsibility. Option 1 is the right shape; option 2 stays
   documented as a fallback if a future operational pattern ever
   demands it.

### Emerged

3. **Plain `INSERT`, no `ON CONFLICT DO NOTHING`.** Original plan
   sketched `ON CONFLICT (asset_type) WHERE asset_type = 0 DO NOTHING`
   for "defensive replay safety". On reflection, sqlx tracks
   `_sqlx_migrations` and runs each migration exactly once per DB —
   the row cannot exist before this migration's only path. `ON CONFLICT`
   would silently suppress an unexpected duplicate (operator misuse
   the only realistic source); plain `INSERT` fails loudly in that
   case, which is the desired observability mode. YAGNI applied.

4. **`name = 'Stellar Lumen'` (singular) over `'Stellar Lumens'`
   (plural).** User-picked. Stellar Foundation marketing convention is
   plural ("Lumens"); singular is technically the unit name (1 lumen,
   N lumens). Both defensible. Going with user's explicit choice.

5. **No new ADR.** The singleton invariant (`uidx_assets_native`) and
   the identity shape (`ck_assets_identity` for `asset_type = 0`) are
   already documented in ADR 0037 §326-338. This task is pure bootstrap
   data — installs the row the schema already requires to exist. Not
   an architectural decision.

6. **`upsert_assets_native` in `write.rs:1040-1068` left untouched.**
   Originally task notes flagged it as evidence of a missing native
   path. After the migration seed lands, `upsert_assets_native` is
   defensive code that costs zero when called with empty input
   (`if rows.is_empty() { return Ok(()); }` early exit). Removing it
   would be a refactor outside 0161 scope; leaving it costs nothing.

## Future Work

None. The native singleton story is complete: schema enforces, migration
provides, test pins. `total_supply` and `holder_count` for native XLM
remain `NULL` — those are the metadata-worker job per ADR 0022 (not
0161 scope).

## Notes

Audit gap. Coordinated with 0160 — the native XLM-SAC contract row (from
0160, `asset_type = 2`, `contract_id = CAS3J7G…OWMA`) is a separate
entity from this native singleton (`asset_type = 0`, no contract). Both
co-exist on a fully-indexed DB: row 1 represents "the native asset
itself", row 2 represents "the Soroban contract that wraps native XLM
for use in smart contracts". Disjoint partial UNIQUE indexes
(`uidx_assets_native` vs `uidx_assets_soroban`) keep them from
colliding.
