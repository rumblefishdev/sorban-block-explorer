---
id: '0142'
title: 'Schema migration: implement ADR 0012 (zero-upsert + activity projections + S3 offload)'
type: FEATURE
status: backlog
related_adr: ['0011', '0012']
related_tasks: ['0140', '0141']
blocked_by: ['0141']
tags:
  [
    layer-db,
    layer-indexer,
    layer-backend,
    priority-high,
    effort-large,
    migration,
    adr-0012,
  ]
milestone: 1
links:
  - lore/2-adrs/0012_zero-upsert-schema-full-fk-graph.md
history:
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from task 0140 audit as the umbrella implementation task for ADR 0012.
      Blocks 21 downstream tasks. Blocked by 0141 (ADR finalization).
---

# Schema migration: implement ADR 0012

## Summary

Implement the full schema and ingestion-pattern redesign specified in ADR 0012.
Replaces the pre-ADR-0012 DB layout (JSONB-heavy, upsert-driven, `transaction_id`
partitioning) with zero-upsert history tables, activity projections, `created_at`
partitioning, a full FK graph, and S3 offload of heavy parsed JSON. This single
migration unblocks 21 downstream tasks.

## Status: Backlog

**Current state:** Not started. Blocked by 0141 (ADR finalization).

## Context

Task 0140 audited all lore tasks against ADR 0012 and identified 21 schema-dependent
tasks whose implementation cannot proceed until this migration lands. All flagged
tasks share the tag `pending-adr-0012-rewrite` and `blocked_by: ['0142']`.

## Scope

### 1. New schema (DDL)

Create all tables per ADR 0012 §"Core tables — schema" and §"Activity projection tables":

Core (16):
`ledgers`, `transactions`, `operations` (partitioned by `created_at`),
`accounts`, `account_balances`, `account_home_domain_changes`,
`soroban_contracts`, `soroban_events` (partitioned), `soroban_invocations` (partitioned),
`tokens`, `token_supply_snapshots`, `nfts`, `nft_ownership`,
`liquidity_pools`, `liquidity_pool_snapshots` (partitioned), `wasm_interface_metadata`.

Activity projections (7):
`account_activity` (partitioned), `token_activity` (partitioned),
`nft_current_ownership`, `token_current_supply`, `liquidity_pool_current`,
`contract_stats_daily`, `search_index`.

Extensions: `pg_trgm`, `hll`.

FKs: full graph per ADR 0012 §"Foreign key graph summary", `ON DELETE RESTRICT`.
`ledgers` has no incoming FKs (dimension).

### 2. Index strategy

- Backfill-time: only PK/UNIQUE + partial UNIQUE (implicit). Extensions installed.
  Fillfactor 90 + autovacuum tuning on progressive-COALESCE tables
  (`soroban_contracts`, `nfts`, `tokens`).
- Post-backfill: build every secondary index CONCURRENTLY (or build-then-ATTACH
  per sealed partition). Complete list per ADR 0012 §"Post-backfill index set".

### 3. Indexer rewrite

- Identity-first persist order per ADR 0012 §"Parallel backfill strategy".
- COALESCE progressive fill on `soroban_contracts`, `nfts`, `tokens` — never
  overwrite known values.
- S3 persist phase: write `parsed_ledger_{seq}.json` with full file structure
  (`ledger_metadata`, `transactions[]`, `wasm_uploads[]`, `contract_metadata[]`,
  `token_metadata[]`, `nft_metadata[]`).
- Activity projection writes at persist time (`account_activity`, `token_activity`).
- `_current` projection upserts with watermark guard.
- `search_index` upsert at identity-row creation.

### 4. API rewrite

- Replace DB-source queries per endpoint per ADR 0012 §"Per-endpoint verification".
- Detail endpoints: at most 1 S3 fetch of `parsed_ledger_{seq}.json`.
- `GET /ledgers/:sequence` served from S3 `ledger_metadata` header.
- `_current` projection tables replace `DISTINCT ON` / `LATERAL JOIN` patterns for
  "current state" queries.
- `search_index` replaces per-entity UNION ALL in `/search`.

### 5. Rollup Lambdas

- `contract_stats_daily` rollup (HLL merge for unique callers).
- `liquidity_pool_current.volume_24h` rollup.
- Both scheduled via EventBridge. Cadence decided in 0141.

### 6. Monitoring

- CloudWatch alarm on `ledger_sequence` drift (no-FK mitigation).
- Alarms for post-backfill index build progress.

### 7. Data migration runbook

Document the old-schema → new-schema conversion path:

- Drain or pause the indexer.
- Run new DDL in a dedicated migration.
- Backfill from archive (Stellar pubnet S3) under new schema + identity-first order.
- Post-backfill CONCURRENTLY index build.
- API cutover.

## Implementation Plan

1. Write SQL migration file(s) per ADR 0012 — one migration adds all new schema,
   drops/replaces old as needed. Follow migration framework from task 0021.
2. Rewrite `crates/db/src/*` with new query/insert paths.
3. Rewrite `crates/indexer/src/handler/persist.rs` for identity-first + S3 persist
   - activity projections + `_current` upserts.
4. Add `crates/backfill-bench/` integration for new persist path.
5. Rewrite `crates/api/src/` per-endpoint source mappings.
6. New rollup crates/Lambdas (`crates/contract-stats-rollup`, `crates/lp-volume-rollup`).
7. Add CDK for new Lambdas + alarms in `infra/src/lib/stacks/`.
8. Write runbook in `lore/3-wiki/` describing migration execution.
9. Stage the migration on `Explorer-staging-*`, validate, then roll to production.

## Acceptance Criteria

- [ ] New DDL migration idempotent and reversible
- [ ] All FKs enforced `ON DELETE RESTRICT`; identity-first ingestion passes tests
- [ ] Indexer writes parsed JSON to S3 + lightweight rows + activity projections in one
      logical transaction per ledger
- [ ] `_current` projections always reflect latest history-table row
- [ ] Post-backfill index build pipeline completes CONCURRENTLY without blocking writes
- [ ] Rollup Lambdas deployed and producing metrics
- [ ] Per-endpoint regression tests pass against new schema
- [ ] 21 downstream tasks unblocked (tag `pending-adr-0012-rewrite` cleared after each
      task re-aligns its body)
- [ ] Monitoring alarms for `ledger_sequence` drift and index-build progress active
- [ ] Runbook committed under `lore/3-wiki/`

## Notes

- 21 downstream tasks are gated on this: `0045`–`0053`, `0116`, `0121`, `0122`,
  `0124`, `0125`, `0126`, `0130`, `0132`, `0133`, `0135`, `0136`, `0138`.
- This task is itself blocked by 0141 (ADR finalization). When 0141 closes, remove
  `blocked_by: ['0141']` and promote to active.
- Open question resolutions from 0141 may require scope adjustments here — keep
  acceptance criteria open until 0141 ships.
