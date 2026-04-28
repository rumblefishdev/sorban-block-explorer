# Backfill Execution Plan (Proposal)

How we propose to populate staging RDS with the full Soroban era (Feb 2024 →
present) ahead of switching live ingestion on. Local-first, dump-restore,
sequential cutover.

> Status: **proposal**. Sub-tasks
> ([0130](../1-tasks/backlog/0130_FEATURE_historical-partition-gaps.md),
> [0132](../1-tasks/backlog/0132_FEATURE_missing-db-indexes.md),
> [0041](../1-tasks/backlog/0041_FEATURE_galexie-config-testnet-validation.md))
> own concrete deliverables. This page ties them together and records
> methodology not captured elsewhere.

## Goal

A staging Postgres pre-populated with parsed history for every Soroban-era
ledger (`SOROBAN_START = (2024, 2)` per [`db-partition-mgmt`](../../crates/db-partition-mgmt/src/lib.rs#L22)),
with monthly partitions correctly populated (no `_default` overflow), so that
when Galexie + the indexer Lambda turn on they can cleanly continue from the
last historical ledger without double-ingest or missing rows.

## Why local-first

| Constraint      | Why local-first wins                                                                 |
| --------------- | ------------------------------------------------------------------------------------ |
| Iteration speed | Parser bugs caught before they touch staging RDS — re-run is free locally            |
| Cost            | Staging RDS hours are billed; local docker is not                                    |
| Determinism     | Dump captures one frozen state, not a moving target                                  |
| Reversibility   | If a bug is found post-restore, redump locally and re-restore one bundle (per-month) |

## Prerequisites (must land before T0)

These tasks own the gates; this proposal references rather than duplicates
them.

| Gate                                                            | Task                                                                                                                                                                               | Why / when it blocks                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Monthly partitions cover the full Soroban era (local + staging) | [0130](../1-tasks/active/0130_FEATURE_historical-partition-gaps.md) — operational; reuses the Lambda code path via a CLI binary (`cargo run -p db-partition-mgmt --bin cli`)       | Blocks T0. Without children, every backfill row falls into `_default` — kills partition pruning everywhere. The bench's `_default` shortcut (`backfill-bench/src/main.rs`) is fine for smoke runs but does not give partition pruning for real workloads.                                                  |
| Galexie testnet/mainnet operational validation                  | [0041](../1-tasks/backlog/0041_FEATURE_galexie-config-testnet-validation.md) — CDK config already shipped (`infra/src/lib/stacks/ingestion-stack.ts`); remaining work is empirical | Blocks T7 (Galexie cutover), not T0.                                                                                                                                                                                                                                                                       |
| Indexes from 0167's INDEX-GAP audit                             | [0132](../1-tasks/backlog/0132_FEATURE_missing-db-indexes.md) — three concrete indexes (`idx_tx_keyset`, `idx_nfts_collection_trgm`, `idx_pools_created_at_ledger`)                | Does not block T0. `CREATE INDEX CONCURRENTLY` post-restore on staging.                                                                                                                                                                                                                                    |
| ~~Envelope/meta ordering validation~~                           | ~~[0134](../1-tasks/archive/0134_BUG_envelope-meta-ordering-validation.md)~~ — superseded                                                                                          | Closed. The hash-based pairing rewrite of `extract_envelopes` in `crates/xdr-parser/src/envelope.rs` (driven by 0167's audit on mainnet ledger 62016099) satisfies every concrete AC; the only remaining design choice — fail-fast vs. skip-with-log — was deliberately settled in favor of skip-with-log. |

## Phases

```
T0  backfill-runner --start <SOROBAN_START> --end <today>          (24–72h)
T1  pause runner
T2  Bundle M dump per closed month (parallel, --jobs 4)
T3  Bundle 0 dump (FINAL — captures end-state sequences + registries)
T4  Bundle Z (_default) sanity dump → assert empty
T5  pg_restore staging   (Bundle 0 → Bundle M chronologically)
T6  ANALYZE staging.*
T7  Deploy Galexie with --start = max(ledgers.sequence) + 1
T8  Live ingest on; monotonic merge handles any catch-up overlap
```

## Dump methodology — per-partition bundles

Bundle = one month × all 7 partitioned tables. Per-month grouping is
**FK-self-contained**: every child of `transactions` references the parent
via composite FK `(transaction_id, created_at)`, and `created_at` is the
partition key, so cross-month foreign keys are structurally impossible.

### Bundle layout

| Bundle                   | Contents                                                                                                                                                                                                      | Cardinality                              | When dumped     |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- | --------------- |
| **0 (registries)**       | `ledgers`, `accounts`, `assets`, `soroban_contracts`, `wasm_interface_metadata`, `nfts`, `liquidity_pools`, `lp_positions`, `account_balances_current`, `transaction_hash_index`                              | 1 file                                   | T3 (final pass) |
| **M (per month)**        | 7 children: `transactions`, `operations_appearances`, `transaction_participants`, `soroban_events_appearances`, `soroban_invocations_appearances`, `nft_ownership`, `liquidity_pool_snapshots` for that month | 27 files (Soroban-era 2024-02 → 2026-04) | T2              |
| **Z (\_default sanity)** | `*_default` for all 7 partitioned parents                                                                                                                                                                     | 1 file                                   | T4              |

### Why Bundle 0 is dumped LAST, not first

`accounts`, `assets`, `soroban_contracts` accumulate across the entire
backfill. A pre-flight Bundle 0 dump would freeze sequences at T0, leaving
staging with stale `SETVAL` values; new live INSERTs after T7 would risk
collisions in the gap window.

Re-dumping at T3 captures the final sequence state plus the full registry
content. Restored at T5, this gives staging the correct `SETVAL` watermark
before Galexie writes its first new row.

### Sample commands

```bash
# T2 — per-month bundle
M=2024-02
pg_dump --format=custom --no-owner --no-acl --data-only \
  -t transactions_y${M//-/m_y}                  \  # placeholder; resolve via shell
  -t operations_appearances_y2024m02            \
  -t transaction_participants_y2024m02          \
  -t soroban_events_appearances_y2024m02        \
  -t soroban_invocations_appearances_y2024m02   \
  -t nft_ownership_y2024m02                     \
  -t liquidity_pool_snapshots_y2024m02          \
  -f bundle-${M}.dump
```

```bash
# T3 — Bundle 0 (full schema + data of unpartitioned tables)
pg_dump --format=custom --no-owner --no-acl \
  -t ledgers -t accounts -t assets -t soroban_contracts \
  -t wasm_interface_metadata -t nfts -t liquidity_pools \
  -t lp_positions -t account_balances_current \
  -t transaction_hash_index \
  -f bundle-00-registries.dump
```

```bash
# T4 — sanity: _default partitions must be empty
pg_dump --format=custom --data-only -t '*_default' -f bundle-zz-default.dump
# Expect: tiny file, decompresses to zero rows.
```

## Cutover at T7

```sql
SELECT MAX(sequence) FROM ledgers;  -- N
```

Galexie / indexer Lambda starts at `N + 1`. Past that, the persist layer's
monotonic merges (`GREATEST(last_seen_ledger, EXCLUDED...)`,
`CASE WHEN incoming ≥ stored THEN ...` on
[`account_balances_current`](../../crates/indexer/src/handler/persist/write.rs#L1786))
make any incidental overlap idempotent — staging row counts and balances
won't be corrupted by a re-played ledger.

## Concurrent backfill + live — supported but not used by this plan

The persist layer is uniformly monotonic
([`accounts`](../../crates/indexer/src/handler/persist/write.rs#L73),
[`assets`](../../crates/indexer/src/handler/persist/write.rs#L1154),
[`liquidity_pools`](../../crates/indexer/src/handler/persist/write.rs#L1562),
[`lp_positions`](../../crates/indexer/src/handler/persist/write.rs#L1666),
[`nfts`](../../crates/indexer/src/handler/persist/write.rs#L1412),
[`account_balances_current`](../../crates/indexer/src/handler/persist/write.rs#L1786)).
Backfill-runner and Galexie can write the same DB concurrently without
corruption: older ledgers never overwrite newer state.

This plan still runs them sequentially because dump-restore is cleaner with
a frozen source. If timing forces overlap (e.g. backfill takes longer than
expected and we don't want to lose new ledgers), the supported pattern is:

1. Galexie writes ledger archives to S3 only — indexer Lambda OFF.
2. After cutover, indexer Lambda turns on and replays the accumulated S3
   queue from `N + 1`. Idempotent, no special handling required.

## Race / consistency risks

| Risk                                                                                                   | Mitigation                                                                                                                                                      |
| ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pg_dump` REPEATABLE-READ snapshot misses post-T_start writes to the dumped tables                     | Only dump months strictly older than the active backfill window (T2 runs after T1 pause, so no concurrent writes)                                               |
| Bundle 0 sequence drift                                                                                | Re-dump Bundle 0 at T3 (final pass)                                                                                                                             |
| `transaction_hash_index` divergence between local-parser-version and live-parser-version after cutover | Freeze parser binary version: dump and live must use the same `xdr-parser` build                                                                                |
| `_default` partition unexpectedly populated                                                            | T4 sanity dump → assert empty before proceeding to T5                                                                                                           |
| Restore fails mid-bundle                                                                               | `pg_restore --jobs` is idempotent per object; `pg_restore --list` + `--use-list` to resume; or just retry the failed bundle (per-month bundles are independent) |
| Lambda partition-mgmt races dump                                                                       | Defer Lambda deploy until **after** restore completes; first invocation then sees Bundle 0 state and creates only future months                                 |

## Open questions

1. **Dump host / network path** — local laptop → staging RDS over VPN, or
   stage to S3 first and run `pg_restore` from a bastion EC2 in the staging
   VPC? Latter avoids long-running tunnels for ~50–100 GB compressed.
2. **Total dump size** — empirical; estimate confirmed only after T0
   completes. Plan assumes ≤ 200 GB compressed; revisit if larger.
3. **VACUUM strategy** — full `VACUUM ANALYZE` at T6 or autovacuum
   sufficient? Full analyze recommended for first plan-snapshot; autovacuum
   thresholds may not trigger in time for first user queries.
4. **Cutover ledger window** — define an N-ledger gap between dump cutoff
   and Galexie start (e.g. ledgers `[N-100, N]` re-played) to absorb any
   borderline-edge inconsistencies. Idempotent merges make this free.

## See also

- [Partition Pruning Runbook](partition-pruning-runbook.md) — reverse
  operation (dropping partitions) when storage pressure rises
- [Stellar Pubnet Ledger Archive](stellar-pubnet-ledger-archive.md) — S3
  layout consumed by both `backfill-runner` and Galexie
- [`docs/architecture/indexing-pipeline/indexing-pipeline-overview.md`](../../docs/architecture/indexing-pipeline/indexing-pipeline-overview.md)
  — read-time view of the pipeline this plan populates
