---
id: '0149'
title: 'Indexer write-path against ADR 0027 schema (persist_ledger body)'
type: FEATURE
status: active
related_adr: ['0024', '0026', '0027']
related_tasks: ['0140', '0148', '0126', '0118', '0120', '0135']
tags:
  [
    layer-backend,
    layer-indexer,
    layer-db,
    priority-high,
    effort-large,
    adr-0027,
    performance,
  ]
links:
  - crates/indexer/src/handler/persist.rs
  - crates/indexer/src/handler/process.rs
  - crates/xdr-parser/src/types.rs
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
history:
  - date: '2026-04-20'
    status: backlog
    who: fmazur
    note: >
      Spawned from 0148. Task 0148 stubbed persist_ledger with an empty body
      so the workspace turns green; this task replaces the stub with the full
      ADR 0027 write-path. Performance target: p95 ≤ 150ms/ledger on the
      local-bench profile (matches 0137 baseline). Correctness target: zero
      data loss — idempotent, replay-safe, atomic per ledger.
  - date: '2026-04-20'
    status: backlog
    who: fmazur
    note: >
      Scope clarified — parser is not touched in this task. Signature of
      persist_ledger is extended with placeholder params (`nft_events`,
      `lp_positions`, `inner_tx_hashes`, …) for data the parser will produce
      later; process_ledger passes empty slices for now so the wiring exists
      end-to-end and corresponding rows stay at zero until the parser
      catches up. Parser enhancement for any of these is a separate follow-up.
  - date: '2026-04-20'
    status: active
    who: fmazur
    note: 'Activated task — promoted from backlog to active, set as current task.'
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Write-path wired end-to-end: 14-step pipeline + staging pass, retry
      wrapper on 40001/40P01, UNNEST batching at 5 k rows, StrKey → id
      resolver, watermark upserts, trustline removal with same-ledger re-add
      cancellation. Two migrations needed to make replay idempotent:
      `uq_transactions_hash_created_at` and the four natural-key UNIQUEs on
      operations / soroban_events / soroban_invocations / liquidity_pool_snapshots.
      Integration test asserts 18-table row counts on first write and zero
      delta on replay. `backfill-bench` indexed 100 real ledgers from
      mainnet partition 62016000 with zero errors.
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Performance reality check. Three bench runs on fresh DB:
      pure UNNEST p95=424 ms, UNNEST+async+balance+COPY p95=434 ms,
      shipped UNNEST+async+balance p95=450 ms. Variance between runs
      exceeds the measured lever; none of async commit / balance refactor /
      COPY events / COPY ops moves p95 outside noise on local NVMe.
      Server-side FK + index + partition routing cost dominates. Reverted
      the two COPY paths (added complexity without payoff). Kept async
      commit and balance-history refactor — both are justified on their
      own merits (fsync latency on prod EBS; O(log n) vs O(n) replay).
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Diagnostic-event filter added to staging (event_type="diagnostic" rows
      skipped). Aligned with Stellar docs + ADR 0018/0027 S3 lane: diagnostic
      events are "not hashed into the ledger, and therefore are not part of
      the protocol" and not read by any ADR 0027 endpoint. Impact: event rows
      360 523 → 55 581 (–85 %), soroban_events heap 39 MB → 7.7 MB (–80 %),
      p95 450 ms → 305 ms (–32 %). Still over 150 ms SLO but the gap is now
      bounded by server-side schema cost, not event volume.
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Spawned ADR 0030 (contracts surrogate BIGINT id, mirrors ADR 0026 for
      accounts) + task 0151_FEATURE_contracts-surrogate-id-migration to
      implement it. Captures ~270–320 GB/year storage saving and ~10–20 ms
      estimated p95 contribution. PR ships 15/16 acceptance criteria; the
      perf gap is documented with a concrete next-lever + ADR.
---

# Indexer write-path against ADR 0027 schema (persist_ledger body)

## Summary

Replace the no-op body of `persist_ledger` with the full ADR 0027 write-path.
Bridges parser `Extracted*` output to the new schema in a single atomic DB
transaction per ledger — idempotent, replay-safe, and UNNEST-batched. No
round-trip waste, no silent data drops, no orphans.

**Parser stays untouched.** Where the parser does not yet produce data for a
column/table (fee-bump `inner_tx_hash`, NFT ownership events, LP positions),
the signature of `persist_ledger` gains a new parameter that `process_ledger`
fills with an empty slice or `None`. The wiring exists end-to-end; the rows
simply stay at zero until a separate parser task catches up.

## Context

After 0140 (schema) and 0148 (stub), the DB is ADR 0027 and the indexer
compiles against it. Parsed data (`Extracted*`) is dropped on the floor every
invocation. Gap between parser output and the new schema is non-trivial:
surrogate `accounts.id` replaces StrKey FKs, hashes are `BYTEA(32)`,
`operations.details` JSONB was split into typed columns, event/invocation
payloads moved to S3 (ADR 0018), and several tables are new
(`transaction_hash_index`, `nft_ownership`, `lp_positions`,
`account_balances_current`, `account_balance_history`).

## Signature extensions

`persist_ledger` grows new parameters for data the parser will eventually
produce. Today they are always empty / `None`. Matching updates in
`process_ledger` only (parser itself is not edited).

| New param                                                                | Future source                    | Blocked by       | Row impact until parser ready           |
| ------------------------------------------------------------------------ | -------------------------------- | ---------------- | --------------------------------------- |
| `nft_events: &[ExtractedNftEvent]`                                       | `extract_nft_events` enhancement | 0118 / follow-up | `nft_ownership` stays empty             |
| `lp_positions: &[ExtractedLpPosition]`                                   | parser enhancement               | 0126             | `lp_positions` stays empty              |
| `inner_tx_hashes: &[(String, Option<String>)]` (tx_hash → fee-bump hash) | parser enhancement               | follow-up        | `transactions.inner_tx_hash` stays NULL |

## Write pipeline (per ledger, one DB transaction)

FK dependencies set the order. Identifier maps built up in-pass so later
steps translate StrKeys and hashes in O(1).

| #   | Step                                                                                               | Output                                                                                                                                    |
| --- | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Collect StrKeys referenced anywhere                                                                | `HashSet<&str>` — tx sources, op destinations/issuers, deployers, callers, transfer from/to, NFT owners, LP participants, balance holders |
| 2   | Upsert `accounts` (UNNEST)                                                                         | `HashMap<String, i64>` StrKey → `accounts.id`                                                                                             |
| 3   | Upsert `wasm_interface_metadata`                                                                   | —                                                                                                                                         |
| 4   | Upsert `soroban_contracts`                                                                         | —                                                                                                                                         |
| 5   | Insert `ledgers` (idempotent)                                                                      | —                                                                                                                                         |
| 6   | Insert `transactions` RETURNING id                                                                 | `HashMap<String, i64>` tx_hash → `transactions.id`                                                                                        |
| 7   | Insert `transaction_hash_index`                                                                    | —                                                                                                                                         |
| 8   | Insert `transaction_participants`                                                                  | From union of accounts referenced per tx                                                                                                  |
| 9   | Insert `operations` (typed cols unpacked from `details`)                                           | —                                                                                                                                         |
| 10  | Insert `soroban_events` (typed transfer prefix)                                                    | —                                                                                                                                         |
| 11  | Insert `soroban_invocations` (slim cols)                                                           | —                                                                                                                                         |
| 12  | Upsert `tokens` (4 identity classes)                                                               | —                                                                                                                                         |
| 13  | Upsert `nfts` + append `nft_ownership` (from `nft_events`; empty today)                            | —                                                                                                                                         |
| 14  | Upsert `liquidity_pools` + insert `liquidity_pool_snapshots` + upsert `lp_positions` (empty today) | —                                                                                                                                         |
| 15  | Upsert `account_balances_current` + append `account_balance_history` + handle removed trustlines   | —                                                                                                                                         |

## Parser → schema translation table

| Parser output                                                             | ADR 0027 shape                                                                                                 | Translation                                                                             |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `ExtractedLedger.hash: String` (hex)                                      | `ledgers.hash BYTEA(32)`                                                                                       | `hex::decode` in staging pass                                                           |
| `ExtractedTransaction.hash: String`                                       | `transactions.hash` + `transaction_hash_index.hash` BYTEA(32)                                                  | hex decode once, bind same bytes in both tables                                         |
| `ExtractedTransaction.source_account`                                     | `transactions.source_id BIGINT`                                                                                | StrKey lookup via accounts map                                                          |
| _(no field — new signature param)_                                        | `transactions.inner_tx_hash BYTEA(32)?`                                                                        | from `inner_tx_hashes` param; NULL while empty                                          |
| _(derive)_                                                                | `transactions.has_soroban BOOLEAN`                                                                             | `!events[tx].is_empty() \|\| !invocations[tx].is_empty()`                               |
| `ExtractedOperation.details: Value`                                       | typed cols `destination_id`, `asset_code`, `asset_issuer_id`, `pool_id BYTEA`, `transfer_amount NUMERIC(28,7)` | pattern-match on `op_type`, extract from `details` JSON in staging                      |
| `ExtractedEvent.topics: Value` + `data`                                   | `soroban_events.topic0 TEXT` + `transfer_from_id` + `transfer_to_id` + `transfer_amount NUMERIC(39,0)`         | parse transfer-shape topic array + first topic string; full payload → S3 (out of scope) |
| `ExtractedInvocation.function_args/return_value`                          | _(not in schema)_                                                                                              | dropped here; S3 lane (out of scope)                                                    |
| `ExtractedAccountState.balances: Value`                                   | `account_balances_current` + `account_balance_history` rows                                                    | iterate balances array; split native vs credit per `ck_abc_native`                      |
| `ExtractedAccountState.removed_trustlines`                                | `DELETE FROM account_balances_current`                                                                         | skip entries also present in merged balances (cross-tx remove-then-recreate)            |
| `ExtractedLiquidityPool.{asset_a,asset_b,reserves,total_shares,tvl}` JSON | `liquidity_pools.{asset_a_type,…}` + `liquidity_pool_snapshots.{reserve_a,reserve_b,…}`                        | unpack JSON to typed cols                                                               |
| _(new signature param)_                                                   | `lp_positions` rows                                                                                            | from `lp_positions` param; empty today                                                  |
| _(new signature param)_                                                   | `nft_ownership` rows                                                                                           | from `nft_events` param; empty today                                                    |
| `ExtractedContractDeployment.metadata: Value`                             | `soroban_contracts.metadata JSONB` + `contract_type` + `is_sac`                                                | keep JSONB; surface `contract_type` and `is_sac` as cols                                |

## Performance

**Target:** `p95 ≤ 150ms` per ledger on local-bench profile (matches the
baseline measured in archived task 0137). Regression guard via per-step
timings logged at end of `persist_ledger`.

**Patterns:**

1. **UNNEST batch binds** — one INSERT per table, column-arrays as params,
   exactly one round trip per table per ledger.
2. **RETURNING for surrogate recovery** —
   `INSERT … ON CONFLICT (natural_key) DO UPDATE SET natural_key = EXCLUDED.natural_key RETURNING id, natural_key`
   recovers the id whether the row was freshly inserted or already existed.
   Needed for `accounts` (StrKey → id) and `transactions` (hash → id).
3. **Staging outside the tx** — StrKey collection, dedup, JSON unpacking,
   hex decoding all synchronous and in-memory before `pool.begin()`. Tx
   window is pure I/O.
4. **Chunked UNNEST at 5000 rows/table** — PG binds are limited to 65535
   parameters per statement. With 10 columns that caps at ~6500 rows; chunk
   at 5000 for headroom. Ledgers usually fit in one chunk, but Soroban event
   storms exist. Chunking is per-table in the same tx.
5. **Account de-dup early** — one StrKey can appear in many roles
   (source + destination + issuer + event + invocation); resolve once.
6. **Hex decode once** — hex → `Vec<u8>` in staging, bind bytes afterwards.
7. **No COPY protocol** — 0137 measured UNNEST+RETURNING inside the target.
   Revisit only if the SLO is missed after clean baseline.

## Correctness / zero data loss

1. **Atomic per ledger** — all 15 steps in one `pool.begin()` … `commit()`;
   any error rolls back everything. Lambda retry replays the whole ledger.
2. **Idempotent inserts** — immutable tables (`ledgers`, `transactions`,
   `transaction_hash_index`, `operations`, `transaction_participants`,
   `soroban_events`, `soroban_invocations`, `nft_ownership`,
   `liquidity_pool_snapshots`, `account_balance_history`) use
   `ON CONFLICT (…) DO NOTHING` (or `DO UPDATE … RETURNING id` where we need
   the surrogate back).
3. **Watermark-guarded upserts** for derived state
   (`accounts.last_seen_ledger`, `nfts.current_owner_ledger`,
   `liquidity_pools.created_at_ledger`, `lp_positions.last_updated_ledger`,
   `account_balances_current.last_updated_ledger`):
   `SET col = GREATEST(existing, EXCLUDED)` and only overwrite payload
   columns when the incoming watermark is strictly newer. Older batches can
   never roll state backward.
4. **Retry policy** — wrap `pool.begin() … commit()` with exponential
   backoff (3 attempts, 50/200/800 ms) on PG SQLSTATE `40001` (serialization
   failure) and `40P01` (deadlock detected). Any other error fails the
   Lambda invocation (pipeline retries the S3 event).
5. **Composite FK alignment** — partitioned child tables bind the parent
   transaction's `created_at`, so composite FKs to `transactions(id, created_at)` hold.
6. **`ck_tokens_identity` satisfied** — set fields per `asset_type` class
   (native: all identifying cols NULL; classic: `asset_code` + `issuer_id`;
   sac: both + `contract_id`; soroban: `contract_id` only) at bind time.
7. **Native vs credit balances** — `ck_abc_native` / `ck_abh_native` require
   `(asset_code, issuer_id)` both NULL for native, both NOT NULL for credit.
8. **Trustline removals** — `removed_trustlines` → DELETE per
   `(account_id, asset_code, issuer_id)`, but skip entries re-added in the
   final merged balances of the same ledger.

## Concurrency

Multi-Lambda concurrency is allowed — each invocation persists one ledger.
Collisions on shared rows (accounts, contracts, tokens) are serialized by
row-level `ON CONFLICT` locks; correctness holds by idempotency +
watermarks. **We do not take advisory locks per ledger** — the cost of that
outweighs the win at observed concurrency levels.

If profiling under burst load reveals row-lock contention (watch for
`40001` rate climbing), partitioning writes by `sequence mod N` across
dedicated Lambdas is the next lever — recorded as follow-up, not this
task.

## Observability

Every log line in `persist_ledger` carries `ledger_sequence`. Per-step
timings emitted once at the end as a single structured log:

```
tracing::info!(
    ledger_sequence,
    total_ms = …,
    "persist breakdown: accounts=5ms contracts=3ms transactions=12ms operations=22ms …"
);
```

Errors additionally log `step_name` and, where applicable, `tx_hash` so
CloudWatch searches can correlate failures to specific artifacts. No
per-row debug logging (kills Lambda throughput at scale).

## Acceptance Criteria

- [x] `persist_ledger` body covers all 15 steps; no TODO markers, no `unimplemented!()`
- [x] Signature extended with `nft_events`, `lp_positions`, `inner_tx_hashes`; `process_ledger` passes empty slices/`None`
- [x] StrKey → `accounts.id` resolver: bulk upsert + RETURNING, per-ledger `HashMap`
- [x] Every table write uses UNNEST batching — one round trip per table (or one round trip per 5k-row chunk); per-step timings logged
- [x] Replay-safe: re-running the same ledger yields no duplicate-key errors and no duplicated rows
- [x] Watermark-guarded: feeding an older ledger for an account/nft/pool/balance does not regress state
- [x] Trustline removals land, except when re-added in the same ledger
- [x] `ck_tokens_identity` and `ck_abc_native` / `ck_abh_native` never throw in practice
- [x] `transaction_participants` contains every account referenced by a given tx (source + ops + events + invocations), de-duplicated
- [x] Composite FKs to `transactions(id, created_at)` hold for all partitioned children
- [x] Retry policy: 3-attempt exponential backoff on `40001` / `40P01`; other errors bubble up
- [x] `cargo clippy --all-targets -- -D warnings` green; `SQLX_OFFLINE=true cargo build --workspace` green
- [x] `npm run db:prepare` succeeds; `.sqlx/` offline cache committed (empty — only runtime queries)
- [x] Integration test: insert one synthetic ledger, assert row counts across 15 tables written today + 0 rows in `nft_ownership` / `lp_positions`; replay the same ledger, counts unchanged
- [x] `backfill-bench` runs ≥100 ledgers from a real Stellar partition without errors
- [ ] Performance: `p95 ≤ 150ms/ledger` on local-bench profile (logged + asserted in bench) — **not met; shipped p95 = 305 ms (after diagnostic-event filter). See [Performance](#performance) below. Gap remains open; next largest schema lever captured in ADR 0030 + task 0151.**

## Performance

Bench added to `backfill-bench` with `--assert-p95-ms <N>` flag for CI gating.
Measurements on local-bench profile (docker-compose PostgreSQL 16, Ubuntu
24.04, NVMe SSD, backfill 100 real ledgers from mainnet partition 62016000).
Three runs on freshly-migrated DB, identical input range, showing the
levers are indistinguishable from noise:

| Metric  | (A) Pure UNNEST | (B) + async commit + balance refactor + COPY events + COPY ops | (C) Final shipped: (A) + async commit + balance refactor |          SLO |
| ------- | --------------: | -------------------------------------------------------------: | -------------------------------------------------------: | -----------: |
| min     |          124 ms |                                                         117 ms |                                                   116 ms |            — |
| mean    |          305 ms |                                                         304 ms |                                                   317 ms |            — |
| p50     |          337 ms |                                                         339 ms |                                                   354 ms |            — |
| **p95** |      **424 ms** |                                                     **434 ms** |                                               **450 ms** | **≤ 150 ms** |
| p99     |          445 ms |                                                         458 ms |                                                   497 ms |            — |
| max     |          502 ms |                                                         483 ms |                                                   555 ms |            — |

Variance between runs is ~30 ms at p95, larger than any measured lever.
The shipped config is (C): async commit and the balance-history refactor
are kept because they are justified on their own merits (fsync latency
on slower disks; O(log n) vs O(n) replay cost) even though they are not
perf wins on local NVMe. COPY was reverted because it adds ~300 lines of
machinery without a measurable payoff (see "Root cause" below).

Per-step breakdown on a heavy ledger (tx_count ≈ 500) shows
`soroban_events` dominates (~50 % of persist_ms) with ~1.5 k–3 k rows per
ledger on Soroban-heavy periods.

### What was tried and kept

1. **Async commit** inside the persist tx
   (`SET LOCAL synchronous_commit = off`). Safe here because the S3 event
   source + Lambda retry policy provides end-to-end durability. Measurable
   win is small on local NVMe (~1–3 ms) but bigger on EBS gp3 in
   production.
2. **Balance-history refactor** — partitioned the `account_balance_history`
   append-only insert by identity class (native vs credit) and routed each
   through the matching partial UNIQUE index with `ON CONFLICT DO NOTHING`
   instead of the prior `WHERE NOT EXISTS` anti-join. Same latency, clearer
   code, O(log n) vs O(n) replay cost.

### What was tried and reverted

3. **`COPY FROM STDIN` for `soroban_events` and `operations`** — created
   `TEMPORARY TABLE … ON COMMIT DROP`, streamed TEXT-format COPY, then
   `INSERT … SELECT … ON CONFLICT DO NOTHING`. Measured p95 delta of +10 ms
   (within noise). Reverted to keep the UNNEST path — the reason it didn't
   help is spelled out below.

### Root cause of the gap

The `soroban_events` hot path cost is dominated by **server-side** work, not
wire-protocol parsing:

- 4 FK checks per row (contract_id → soroban_contracts, transfer_from_id /
  transfer_to_id → accounts, composite (transaction_id, created_at) →
  transactions)
- 3 indexes on soroban_events: primary key, partial idx_events_transfer_from,
  partial idx_events_transfer_to, plus the new natural-key UNIQUE
- Partition routing through the DEFAULT partition (local bench creates
  `_default` partitions because db-partition-mgmt only covers 3 of the 8
  partitioned tables; see 0139 / related follow-up)

`COPY` only replaces the INSERT parser in front of those checks — the
server-side cost per row is unchanged. The same reasoning applies to the
`operations` path.

### Realistic optimization candidates (all out of scope for 0149)

Each requires a change the task explicitly keeps out:

- **Monthly range partitions for all 8 partitioned tables** — smaller hot
  indexes, shorter FK lookups. Belongs in partition-Lambda extension
  (task 0139 neighborhood). Estimated win: 20–40 %.
- **Relax FKs on soroban_events.transfer_from_id / transfer_to_id** — they
  are convenience FKs; the authoritative lookup for transfer history is
  through `transaction_participants`. ADR change; ~30–50 ms saved on heavy
  ledgers.
- **COPY BINARY instead of TEXT** — bypasses text parse for NUMERIC(39,0)
  and TIMESTAMPTZ. Needs a PG binary-wire encoder in Rust. Uncertain win.
- **Parallel per-ledger processing across multiple connections** — doesn't
  help per-ledger p95, but raises indexer throughput under burst load.

### Follow-up spawned

See **[ADR 0030](../../2-adrs/0030_contracts-surrogate-bigint-id.md)** +
task **0151_FEATURE_contracts-surrogate-id-migration** (backlog) — captures
the next single largest schema lever (contract_id VARCHAR(56) → BIGINT
surrogate, mirroring ADR 0026's accounts-surrogate pattern). Projected
contribution: ~10–20 ms on p95, plus ~270–320 GB/year DB-size saving.

The broader sub-150 ms perf work (monthly partitions, FK relaxation on
`soroban_events` transfer\_\* columns, COPY BINARY) remains open but is not
currently tracked as a single task — call it out when we actually need
stricter latency than 305 ms.

## Out of Scope

- **Any parser change.** Fields the parser does not produce today stay at
  NULL (scalar) or empty slice (list). Separate follow-ups extend parser:
  - `nft_events` — belongs with task 0118 work or its own task
  - `lp_positions` — task 0126 (blocked)
  - `inner_tx_hash` — new follow-up
- S3 writes for memo, signatures, raw op params, XDR, diagnostic events,
  full event payloads, invocation args/return values (ADR 0011/0018 lanes).
- `operation_tree` persistence (no column in ADR 0027 — belongs in S3).
- API-layer JOIN updates (ADR 0027 Part III endpoints).
- Partition Lambda extension to cover all 8 partitioned tables and
  `operations` re-partition by `created_at`.
- `tokens.holder_count` maintenance (task 0135).
- NFT false-positive filter (task 0118).
- Soroban-native token detection (task 0120).
- Advisory locking per ledger; revisit only if `40001` rate climbs under
  burst load.
- COPY-protocol experiments; revisit only if `p95 ≤ 150ms` is missed after
  clean baseline.

## Notes

- Any signature change to `persist_ledger` is mirrored in `process_ledger`
  and `backfill-bench` in the same PR. Three call sites total.
- Staging pass (StrKey collection, JSON unpacking, hex decoding) is pure
  synchronous; only I/O happens between `pool.begin()` and `commit()`.
- Per-step timings log shape matches the pre-ADR baseline from 0137 so
  regressions are directly comparable.
