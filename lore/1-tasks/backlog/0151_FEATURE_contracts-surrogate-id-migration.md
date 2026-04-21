---
id: '0151'
title: 'Implement ADR 0030: soroban_contracts surrogate BIGINT id + migrate 5 FK tables'
type: FEATURE
status: backlog
related_adr: ['0026', '0027', '0030']
related_tasks: ['0149']
tags:
  [
    layer-backend,
    layer-indexer,
    layer-db,
    layer-api,
    priority-medium,
    effort-large,
    adr-0030,
    schema-migration,
    storage,
  ]
links:
  - lore/2-adrs/0030_contracts-surrogate-bigint-id.md
  - crates/db/migrations
  - crates/indexer/src/handler/persist/write.rs
  - crates/api
history:
  - date: '2026-04-21'
    status: backlog
    who: fmazur
    note: >
      Spawned from 0149 follow-up work. ADR 0030 designed during 0149 bench
      analysis: 105 unique contracts vs ~77k references in a 100-ledger sample
      (1:730 ratio) — same shape as accounts surrogate (ADR 0026). Projected
      saving: ~270–320 GB/year of mainnet DB size, plus ~10–20 ms shaved off
      persist_ledger p95 (partial contribution to the old 150 ms SLO target).
      Replaces the earlier 0150_PERF task — diagnostic-event filter in 0149
      already captured the majority of the perf win; contract surrogate is the
      clean next lever and fits an ADR-shaped design.
---

# Implement ADR 0030: `soroban_contracts` surrogate BIGINT id + migrate 5 FK tables

## Summary

Apply the ADR 0030 design: add `soroban_contracts.id BIGSERIAL PRIMARY KEY`,
keep `contract_id VARCHAR(56) UNIQUE` for lookup + display, and migrate the
five dependent tables (`operations`, `soroban_events`, `soroban_invocations`,
`tokens`, `nfts`) from `contract_id VARCHAR(56)` FK to `contract_id BIGINT` FK
pointing at the surrogate.

## Context

ADR 0030 is the symmetric treatment of ADR 0026 (accounts surrogate). ADR
0027 Part V §7 explicitly flagged this as a future ADR candidate; task 0149
measured the size/perf impact and the numbers justify the work. The resolver
pattern, cache sizing, Pattern A / Pattern B queries, and migration shape
are all documented in ADR 0030 — this task is the implementation.

Preconditions (all met):

- ADR 0026 landed accounts surrogate — resolver pattern exists in
  `crates/indexer/src/handler/persist/write.rs::upsert_accounts` and the API
  already JOINs `accounts` for StrKey display.
- ADR 0027 write-path landed via task 0149.
- Diagnostic-event filter landed in 0149 — contract reference volume is now
  stable (~550 contract-events per ledger × 5 tables touched).

## Implementation

### Phase 1 — schema migration

New migration (`yyyymmddHHMMSS_contracts_surrogate_id.up.sql` + `.down.sql`):

1. Add `id BIGSERIAL` to `soroban_contracts`, populate via sequence default,
   swap primary key (`contract_id` → `id`), add `UNIQUE (contract_id)`.
2. For each dependent table (`operations`, `soroban_events`,
   `soroban_invocations`, `tokens`, `nfts`):
   - Add `contract_sid BIGINT` column.
   - Backfill via `UPDATE … FROM soroban_contracts WHERE contract_id =
soroban_contracts.contract_id`.
   - Drop existing FK + column, rename `contract_sid` → `contract_id`.
   - Add new FK + index.
3. For partitioned tables (`operations`, `soroban_events`,
   `soroban_invocations`), repeat per attached partition — the parent DDL
   cascades the column swap but the per-partition indexes/FKs need
   explicit recreation.
4. Pair `down.sql` restores `VARCHAR(56)` by reverse-joining.

### Phase 2 — Rust persist layer

- Add `upsert_contracts_returning_id(&mut tx, &[ContractRow]) ->
HashMap<String, i64>` in `crates/indexer/src/handler/persist/write.rs`
  (mirror of existing `upsert_accounts`).
- Rework `register_referenced_contracts` to build the StrKey → id map for
  downstream steps instead of just registering.
- Update `op_rows`, `event_rows`, `inv_rows`, `token_rows`, `nft_rows`
  construction to resolve `contract_id` StrKey → surrogate id via the map.
- Integration test: extend `persist_integration.rs` assertion to verify
  round-trip (insert ledger → fetch via JOIN → assert `contract_id` text
  matches input).

### Phase 3 — API layer

- Every endpoint that takes `:contract_id` as route param (E11, E12, E13,
  E14, E15 filter, E10 Soroban branch, E22 search) adds a preflight:
  `SELECT id FROM soroban_contracts WHERE contract_id = $1;`
- Every endpoint that displays contract_id in response adds a
  `LEFT JOIN soroban_contracts sc ON sc.id = <table>.contract_id` (E3 ops
  / events / invocations, E8, E9, E15, E16).
- Update OpenAPI annotations — response shapes unchanged, SQL under the
  hood changed.

## Acceptance Criteria

- [ ] Migration applied cleanly on a fresh local DB (`docker compose down -v`
      → `npm run db:migrate`)
- [ ] Migration applied cleanly on a non-empty DB (existing
      soroban_contracts rows preserved; all 5 dependent tables backfilled
      without data loss)
- [ ] `persist_integration.rs` test passes against the new schema —
      contract_id round-trip (JSON → BIGINT FK → JSON).
- [ ] `backfill-bench --start 62016000 --end 62016099` indexes 100 ledgers
      without errors; p95 measured + reported.
- [ ] DB size after 100 ledgers compared to pre-0030 baseline — capture
      per-table heap + index size delta.
- [ ] Each API endpoint that previously filtered by contract_id still
      returns correct rows (unit test per E10-soroban / E11 / E13 / E14).
- [ ] `cargo clippy --all-targets -- -D warnings` green.
- [ ] `SQLX_OFFLINE=true cargo build --workspace` green.
- [ ] ADR 0027 marked `superseded` with `by: 0030` in history after
      landing; ADR 0030 promoted to `accepted`.

## Out of Scope

- Symmetric surrogate for `nfts.token_id` (per-contract token identity,
  different cardinality math — separate future ADR if ever warranted).
- Any perf lever not listed in ADR 0030 (monthly partitions, FK drops, COPY
  BINARY, etc. — those belong in separate follow-up tasks).
- Changes to `accounts` / `liquidity_pools` / other identity tables.

## Notes

- **Backfill ordering matters**: migration must touch `soroban_contracts`
  first (add surrogate), then each FK table in any order. FK constraint
  swap happens per-table; other tables keep VARCHAR FK until their own step.
- **RDS production migration plan** (separate task when nearing launch):
  online backfill via batched UPDATE with `statement_timeout`, expected
  ~20–60 minutes for mainnet-scale data.
- **Cache warmup**: on indexer cold start after migration, the StrKey → id
  cache is empty. First ledger pays ~100 roundtrips to resolve the active
  contract set; subsequent ledgers ~0 roundtrips. No action needed — same
  characteristic as accounts cache (ADR 0026).
