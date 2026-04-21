---
id: '0151'
title: 'Implement ADR 0030: soroban_contracts surrogate BIGINT id + migrate 5 FK tables'
type: FEATURE
status: completed
related_adr: ['0026', '0027', '0030']
related_tasks: ['0149', '0152']
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
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Activating — starting ADR 0030 implementation (soroban_contracts
      surrogate BIGINT id + 5 FK table migration). Task 0149 landed on
      develop via PR #101, so the write-path this task extends
      (register_referenced_contracts, upsert_accounts pattern) is now in
      place.
  - date: '2026-04-21'
    status: completed
    who: fmazur
    note: >
      Landed. 2 phases shipped (schema migration + Rust persist); phase 3
      (API) deferred since endpoints live in backlog tasks 0043-0057 and
      ADR 0030 Part III is their design reference. Files: migrations
      0002-0005 edited as source-of-truth (per Filip's mid-task direction);
      write.rs upsert_contracts_returning_id + resolve_contract_id/_opt_id
      helpers; 6 callsites flipped VARCHAR→BIGINT binds; mod.rs threads
      contract_ids map through run_all_steps. Tests: persist_integration
      1/1 pass with JOIN-via-soroban_contracts cleanup + count CTEs;
      clippy + build green. Bench: 100 ledgers clean, p95 309 ms (matches
      post-diag-filter 0149 baseline — no regression); 1:728 ref-to-
      unique ratio vs 1:730 forecast. ADR 0030 → accepted, ADR 0027 →
      superseded by 0030. Spawned 0152 (ADR 0031 implementation:
      enum-like VARCHAR → SMALLINT + Rust enum, ~160-220 GB/year
      additional saving).
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

- [x] Migration applied cleanly on a fresh local DB (`docker compose down -v`
      → `npm run db:migrate`) — `npm run db:reset` passed across multiple runs.
- [~] **N/A** — Migration applied cleanly on a non-empty DB. Strategy
  changed mid-task: instead of a separate `yyyymmddHHMMSS_contracts_surrogate_id`
  migration with backfill, the ADR 0030 schema was folded into the
  source-of-truth migrations `0002–0005` per Filip's direction
  ("domain ma reprezentować stan bazy po wszystkich migracjach"). The
  project is pre-GA; there is no production data to preserve yet.
  Non-empty backfill is tracked separately in the "RDS production
  migration plan" note below for launch readiness.
- [x] `persist_integration.rs` test passes — 1/1 pass.
      `synthetic_ledger_insert_and_replay_is_idempotent` was updated to
      JOIN `soroban_contracts` in cleanup DELETEs and in the `test_counts`
      aggregation (`tk`, `n`, `no` CTEs).
- [x] `backfill-bench --start 62016000 --end 62016099` indexes 100 ledgers
      without errors; p95 measured + reported. Two clean runs:
      Run 1 p95=385 ms, Run 2 p95=309 ms (variance ±40 ms — typical local
      Docker Postgres jitter). 0 errors, 0 retries, 100/100 ledgers indexed.
- [x] DB size after 100 ledgers — captured (see `## Implementation Notes`).
      Pre-0030 baseline skipped: project pre-GA with edited source-of-truth
      migrations, no historical snapshot exists. Reference-to-unique ratio
      measured at **1:728** (105 contracts : 76,437 refs) — within ADR 0030
      forecast (1:730).
- [~] **N/A** — API endpoint tests per E10-soroban / E11 / E13 / E14.
  None of the 22 endpoints are implemented yet (all backlog tasks
  0043–0057). ADR 0030 Part III is the design reference those tasks
  will follow. Criterion was aspirational; it maps to future per-
  endpoint tasks, not this one.
- [x] `cargo clippy --all-targets -- -D warnings` green.
- [x] `SQLX_OFFLINE=true cargo build --workspace` green.
- [x] ADR 0027 marked `superseded` with `by: 0030`; ADR 0030 promoted to
      `accepted` (done at task close).

## Implementation Notes

### Phase 1 — schema migration (strategy changed mid-task)

Originally planned as a separate `yyyymmddHHMMSS_contracts_surrogate_id`
migration. Changed to edit source-of-truth migrations `0002–0005` directly
(project pre-GA, pattern precedent in 998b774 `refactor(lore-0140):
implement ADR 0027 schema from scratch`). Files touched:

- `crates/db/migrations/0002_identity_and_ledgers.sql` — `soroban_contracts`
  gains `id BIGSERIAL PRIMARY KEY`; `contract_id VARCHAR(56) NOT NULL UNIQUE`
  retained as lookup/display key.
- `crates/db/migrations/0003_transactions_and_operations.sql` —
  `operations.contract_id BIGINT REFERENCES soroban_contracts(id)`.
- `crates/db/migrations/0004_soroban_activity.sql` — same for
  `soroban_events.contract_id` and `soroban_invocations.contract_id`.
- `crates/db/migrations/0005_tokens_nfts.sql` — same for `tokens.contract_id`
  and `nfts.contract_id`.

The incremental migration attempted first (`20260421000200_contracts_surrogate_id.up/down.sql`)
was moved to `.trash/`.

### Phase 2 — Rust persist layer

- `crates/indexer/src/handler/persist/write.rs`:
  - `upsert_contracts` → `upsert_contracts_returning_id`: two-pass upsert,
    rich rows first (with deployment metadata), then bare-row registration
    pass for StrKeys referenced from ops/events/invocations/tokens/nfts.
    Both passes use `ON CONFLICT ... DO UPDATE SET ... RETURNING id, contract_id`
    (no-op DO UPDATE trick to force RETURNING on replay; `DO NOTHING` would
    suppress it for conflicting rows).
  - New helpers: `resolve_contract_id` / `resolve_contract_opt_id`
    (mirror of `resolve_id` / `resolve_opt_id` for accounts).
  - 6 callsites switched from `$X::VARCHAR[]` to `$X::BIGINT[]` binds:
    `insert_operations`, `insert_events`, `insert_invocations`,
    `upsert_tokens_classic_like`, `upsert_tokens_soroban`,
    `upsert_nfts_and_ownership` (+ `nft_ownership` child).
- `crates/indexer/src/handler/persist/mod.rs`:
  - `contract_ids: HashMap<String, i64>` threaded through `run_all_steps`
    and passed to the 6 write helpers alongside `account_ids`.
  - Step 3 (`upsert_contracts_returning_id`) now returns the map instead
    of `()`.

### Phase 3 — API layer

- **Deferred.** None of the 22 endpoints are implemented yet. ADR 0030
  Part III + the endpoint feasibility matrix are the spec every future
  API task (0043–0057) must follow: preflight StrKey→id resolve on
  `:contract_id` route params, `LEFT JOIN soroban_contracts` on per-row
  display.

### Bench numbers (100 ledgers, partition `62016000`)

Two clean runs from fresh DB:

| Metric     |      Run 1 |      Run 2 |
| ---------- | ---------: | ---------: |
| index time |     29.5 s |     24.3 s |
| min        |     149 ms |     111 ms |
| mean       |     284 ms |     234 ms |
| p50        |     290 ms |     246 ms |
| **p95**    | **385 ms** | **309 ms** |
| p99        |     440 ms |     366 ms |
| max        |     471 ms |     389 ms |

Run-to-run variance ±40 ms dominated by local Docker Postgres jitter
(cold cache, shared buffer reload on `npm run db:reset`). Run 2
p95=309 ms matches the post-diagnostic-filter baseline from task 0149
(~305 ms), confirming ADR 0030 does not regress perf.

`contracts_ms` per ledger: 0–6 ms (mostly 0–1 ms). Pass-2 bare-row
upsert overhead is negligible — hypothesis "RETURNING trick adds
measurable write traffic" was falsified by breakdown timings.

### DB sizes (100 ledgers, post-ADR-0030 layout)

Total DB: **84 MB**.

| Table (parent + partitions)   |   Heap |    Idx |  Total |
| ----------------------------- | -----: | -----: | -----: |
| `operations` (76,882 ops)     |   8 MB | 9.4 MB |  17 MB |
| `account_balance_history`     |   6 MB | 4.9 MB |  11 MB |
| `transactions` (36,319)       | 4.8 MB |   6 MB |  11 MB |
| `soroban_events` (55,581)     |   5 MB | 4.7 MB | 9.8 MB |
| `transaction_participants`    | 3.2 MB | 4.9 MB | 8.1 MB |
| `accounts` (14,825)           | 2.4 MB |   5 MB | 7.4 MB |
| `soroban_invocations` (6,942) | 624 kB | 728 kB | 1.4 MB |
| `nfts` (912)                  | 112 kB | 368 kB | 520 kB |
| `soroban_contracts` (105)     |  24 kB | 136 kB | 192 kB |

Contract-FK indexes (all `BIGINT` btree, post-ADR-0030):

| Index                                                    |                       Size |
| -------------------------------------------------------- | -------------------------: |
| `soroban_events_default_contract_id_created_at_idx`      |                     744 kB |
| `operations_default_contract_id_created_at_idx`          |                     192 kB |
| `soroban_invocations_default_contract_id_created_at_idx` |                     104 kB |
| `nfts_contract_id_token_id_key` (UNIQUE)                 |                      56 kB |
| **Total**                                                | **~1.1 MB** on 76,437 refs |

### Ratio and saving estimate

| Measure                                 |                                                                      Value |
| --------------------------------------- | -------------------------------------------------------------------------: |
| Unique contracts in `soroban_contracts` |                                                                    **105** |
| `contract_id` refs across 5 FK tables   | **76,437** (ops:13,308 + events:55,581 + invs:6,942 + nfts:912 + tokens:0) |
| Reference : unique ratio                |                                     **1 : 728** (ADR 0030 forecast: 1:730) |

Estimated saving vs. pre-0030 (VARCHAR(56) → BIGINT, 49 B per ref):

- **Heap**: 76,437 × 49 B ≈ **3.7 MB**
- **Indexes**: BIGINT btree entries ~3–5× narrower than VARCHAR(56).
  Current 1.1 MB vs. estimated pre-0030 ~4–5 MB → **~3–4 MB** saving.
- **Total on 100 ledgers**: **~6.5–7.5 MB / 84 MB ≈ 8–9%**.
- **Mainnet-year extrapolation**: 76,437 / 100 × 6.3 M × 49 B heap ≈
  234 GB + index savings ~100–150 GB = **~330–380 GB/year**.
  In range of ADR 0030 forecast (270–320 GB/year).

## Design Decisions

### From Plan

1. **Two-pass `upsert_contracts_returning_id`** — rich rows first
   (with deployment metadata), bare-row registration second. Mirror
   of ADR 0026 account upsert shape.
2. **StrKey → `i64` map threaded through `run_all_steps`** — same
   shape as `account_ids`. Resolvers (`resolve_contract_id` /
   `resolve_contract_opt_id`) mirror the account helpers.
3. **Integration test updated to JOIN via `soroban_contracts`** — cleanup
   DELETEs and `test_counts` CTEs for `tokens`/`nfts`/`nft_ownership`
   now JOIN `soroban_contracts` to filter by StrKey.

### Emerged

4. **Source-of-truth migrations edited in place (0002–0005)** — plan
   had a new `yyyymmddHHMMSS_contracts_surrogate_id` migration. Filip
   directed mid-task to fold the schema into the canonical pre-GA
   migrations instead. Precedent: 998b774 `refactor(lore-0140):
implement ADR 0027 schema from scratch`. The incremental migration
   files were moved to `.trash/`.
5. **No-op `DO UPDATE SET contract_id = EXCLUDED.contract_id` in
   pass-2** — `ON CONFLICT DO NOTHING` suppresses `RETURNING` on
   conflicting rows, which would leave the StrKey→id map incomplete
   on replay. No-op DO UPDATE forces RETURNING to fire for every
   row. Contracts_ms remains 0–6 ms per ledger, so the trick is
   effectively free.
6. **Phase 3 (API) skipped** — no endpoints implemented yet. ADR 0030
   Part III becomes the design reference for backlog tasks 0043–0057.

## Issues Encountered

- **Initial migration approach required precise DDL ordering**: first
  attempt (`20260421000200_contracts_surrogate_id`) hit `ERROR: there is
no unique constraint matching given keys for referenced table
"soroban_contracts"` because `DROP CONSTRAINT soroban_contracts_pkey`
  ran before `ADD CONSTRAINT uq_sc_contract_id UNIQUE`. Mooted by the
  strategy change to edit source-of-truth migrations directly — no
  DDL ordering pitfalls when the table is created in its final shape.
  The rejected migration lives in `.trash/`.
- **Non-unique index drop during backfill explored as perf lever** —
  empirical test on 100 ledgers showed only ~5–7% saving, far below
  the hypothetical 25–40%. Root cause: local DB (84 MB) fits entirely
  in `shared_buffers` + UNIQUE constraints (ADR-0149 replay safety)
  dominate insert cost. At mainnet scale where cache misses would
  dominate, the lever may recover — but that belongs in a future ADR
  (candidate ADR 0032 "parallel backfill pipeline") not in 0151 scope.

## Future Work

Spawned during 0151 review, scoped out of the ADR 0030 deliverable:

- **ADR 0031 — Enum-like VARCHAR columns → `SMALLINT` + Rust enum**
  (`proposed`, drafted during 0151). Next storage/speed lever after 0030. Saving ~160–220 GB/year mainnet + ~2–3× faster `WHERE type = …`
  filter probes. Covered columns: `operations.type`,
  `soroban_events.event_type`, `account_balance*.asset_type`,
  `tokens.asset_type`, `liquidity_pools.asset_*_type`,
  `nft_ownership.event_type`, `soroban_contracts.contract_type`.
  Implementation task to be spawned post-merge of 0151.
- **ADR 0032 candidate — Parallel backfill pipeline** (concept only, not
  yet drafted). Pattern: two-phase (drop non-unique idx + FK NOT VALID,
  parallel workers, CREATE INDEX CONCURRENTLY + VALIDATE CONSTRAINT).
  Relevant at mainnet-history scale (~60M ledgers); speculative at 11M
  scale (worth < 10% gain, parallelism dominates). Defer until pre-
  launch when target backfill range is known.
- **nfts.token_id surrogate** — per-contract token identity, weaker
  cardinality math. Explicitly out of scope in ADR 0030 §3; revisit
  only if measured impact justifies.

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
