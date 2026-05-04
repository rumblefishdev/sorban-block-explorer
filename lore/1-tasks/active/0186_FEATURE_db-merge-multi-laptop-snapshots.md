---
id: '0186'
title: 'DB merge script for multi-laptop backfill snapshots'
type: FEATURE
status: active
related_adr: ['0040']
related_tasks: ['0010']
tags: [backfill, db, merge, postgres, tooling]
links: []
history:
  - date: 2026-05-04
    status: active
    who: fmazur
    note: 'Task created — implementation grounded in ADR 0040'
  - date: 2026-05-04
    status: active
    who: fmazur
    note: 'Rewrite — locked structural design (atomicity, diff, batching, rebuild timing); fixed test harness to 4-DB design; added idempotency + scale ACs'
  - date: 2026-05-04
    status: active
    who: fmazur
    note: 'Fix infeasible Step 0 snapshot mechanism — switch to postgres_fdw + 5th ephemeral container (postgres-snapshot-source)'
---

# DB merge script for multi-laptop backfill snapshots

## Summary

Build a script that merges one snapshot (Postgres dump) of a backfilled
laptop database into a local Docker target database, applying the
remap/dedup/watermark logic mandated by [ADR 0040](../../2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md).
The script is invoked once per snapshot, **chronologically oldest-first**,
against the same target. Estimate: 1–2 weeks of focused work
(infra + script + diff harness + test corpus).

## Status: Active

**Current state:** ADR 0040 accepted; schema audit complete. Implementation
not started. Step 0 design decisions need to be ratified before coding.

## Context

- N laptops run `backfill-runner` on disjoint ledger ranges into local
  Dockerised Postgres (port 5432). After each laptop finishes, its DB is
  `pg_dump --format=custom`-ed.
- ADR 0040 lists the merge hazards: surrogate-id collision on 4 sequences
  (`accounts`, `soroban_contracts`, `nfts`, `transactions`); LWW current-state
  tables (`lp_positions`, `account_balances_current`, `nfts.current_owner_*`);
  GENERATED `soroban_contracts.search_vector`; `pg_trgm` extension + 5
  IMMUTABLE label functions; partition-FK quirks.
- Scale we're targeting: per laptop ~2M ledgers ⇒ ~50M rows in `transactions`,
  ~150M in `operations_appearances`. Two laptops merged ⇒ ~300M-row target
  table. This rules out single-transaction merges and naive temp tables.

---

## Implementation Plan

### Step 0: Lock structural design decisions

These are the choices that _shape every subsequent step_. Decide before
writing any code; record decisions inline in the script README.

| Decision               | Recommended                                                                                                                                                                                                | Rationale                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Atomicity**          | Per-table batching with `SAVEPOINT`s every 100k rows; pre-merge `pg_dump` of the target as the rollback path                                                                                               | Single tx over 150M rows blows up WAL/locks. Savepoints give recoverable failure without keeping the whole merge open. The pre-merge dump is the only true rollback for cross-table inconsistency.                                                                                                                                                                                                                               |
| **Diff strategy**      | Normalized natural-key projection per table → ordered → `md5_agg` per table → compare hashes                                                                                                               | Direct row-by-row diff with surrogate ids is meaningless (auto-allocated). Per-table hash gives a single bool answer + cheap cardinality check.                                                                                                                                                                                                                                                                                  |
| **Batching threshold** | 100k rows per `INSERT … SELECT`, ledger-sequence-windowed for partitioned tables                                                                                                                           | Keeps temp memory bounded; aligns with `backfill-runner` batch sizes.                                                                                                                                                                                                                                                                                                                                                            |
| **Rebuild timing**     | Post-final-snapshot only — explicit `merge finalize` subcommand                                                                                                                                            | Rebuilding `nfts.current_owner_*` after every snapshot is wasteful (re-scans full ownership log every time). User runs `merge ingest` N times then `merge finalize` once.                                                                                                                                                                                                                                                        |
| **Snapshot ingestion** | `pg_restore` the snapshot into a separate Postgres container (`postgres-snapshot-source`); expose its `public` to the merge target via `postgres_fdw` + `IMPORT FOREIGN SCHEMA public … INTO merge_source` | `pg_restore` cannot retarget a schema name (`--schema` filters what to restore, not where), and renaming `public` on the target is impossible while the target's own `public` is in use. FDW is Postgres's standard cross-DB access pattern; the merge SQL then `SELECT FROM merge_source.<table>` exactly as if it were local. Container isolation also keeps source's seed rows / extensions from polluting target's `public`. |
| **Language**           | Rust (new `crates/db-merge`)                                                                                                                                                                               | Parity with `db-migrate`/`backfill-runner`; sqlx already in workspace; CLI via clap consistent with `backfill-runner`.                                                                                                                                                                                                                                                                                                           |
| **Pre-merge backup**   | `pg_dump --format=custom` of target before every `merge ingest` invocation; user removes after success                                                                                                     | Only safe rollback for cross-table corruption. Path printed to stderr at start.                                                                                                                                                                                                                                                                                                                                                  |

### Step 1: Test infrastructure — 5 Docker databases

Add to `docker-compose.yml`:

| Service                    | Port | Role                                                                                                                 |
| -------------------------- | ---- | -------------------------------------------------------------------------------------------------------------------- |
| `postgres` (existing)      | 5432 | Live target during real backfill — **don't touch in tests**                                                          |
| `postgres-truth`           | 5433 | Sequential ground-truth backfill of full range                                                                       |
| `postgres-laptop-a`        | 5434 | Simulated laptop A, lower ledger range                                                                               |
| `postgres-laptop-b`        | 5435 | Simulated laptop B, upper ledger range                                                                               |
| `postgres-merge`           | 5436 | Merge target — receives snapshots A+B chronologically                                                                |
| `postgres-snapshot-source` | 5437 | Ephemeral; `pg_restore` target for the current snapshot. Reset (drop volume + recreate) before every `merge ingest`. |

All five test DBs share identical config (image `postgres:16-alpine`,
healthcheck, same credentials). The script accepts `--target-url` for the
merge destination and `--snapshot-source-url` for the FDW source.

**Reset procedures**:

- Between test runs (clean merge target):
  `docker compose stop postgres-merge && docker volume rm <prefix>_pgdata-merge && docker compose up -d postgres-merge` then run migrations.
- Between snapshots within one test run (clean snapshot source):
  same pattern on `postgres-snapshot-source`. The merge script can do this
  automatically as the first step of `merge ingest`.

Truncating tables is _not_ sufficient — leaves sequence state and partition
children behind.

### Step 2: Snapshot ingestion (`merge ingest <snapshot> --target-url <url> --snapshot-source-url <url>`)

1. Reset `postgres-snapshot-source` (drop volume, recreate, wait for healthy);
   `pg_restore` the snapshot into its `public` schema. Source's `pg_trgm`
   extension and the 5 IMMUTABLE label functions are restored alongside the
   data and live in _that_ container's `public` — they don't touch the
   target's schema.
2. On the merge target, prepare the FDW bridge (idempotent — script may
   re-run after partial failure):
   ```sql
   CREATE EXTENSION IF NOT EXISTS postgres_fdw;
   CREATE SERVER IF NOT EXISTS merge_source_server FOREIGN DATA WRAPPER postgres_fdw
       OPTIONS (host 'postgres-snapshot-source', port '5432', dbname 'soroban_block_explorer');
   CREATE USER MAPPING IF NOT EXISTS FOR CURRENT_USER SERVER merge_source_server
       OPTIONS (user 'postgres', password 'postgres');
   DROP SCHEMA IF EXISTS merge_source CASCADE;
   CREATE SCHEMA merge_source;
   IMPORT FOREIGN SCHEMA public FROM SERVER merge_source_server INTO merge_source;
   ```
   Now `merge_source.<table>` exposes every source table as a foreign table;
   the merge SQL `SELECT FROM merge_source.X` reads via the FDW (local Docker
   network, low overhead).
3. Pre-flight precondition checks via the FDW (abort on any mismatch with
   actionable error; tear down the FDW bridge first so the target stays
   clean):
   - target's `_sqlx_migrations` matches `merge_source._sqlx_migrations`
     row-for-row including `checksum` (catches schema drift from mid-merge
     migration runs);
   - `merge_source.ledgers` `MIN/MAX(sequence)` doesn't overlap with target's
     existing range; source range is **strictly later** than target's `MAX`
     (chronological-only contract);
   - both sides have `*_default` partition only and matching CHECK set
     (`ck_assets_identity`, `ck_sia_caller_xor`, partial UNIQUEs) — verified
     by querying `pg_constraint` on each via FDW vs local.
4. Take pre-merge `pg_dump` of target (Step 0 decision); print path to
   stderr. Only after this point is the merge committed.

### Step 3: Topological merge (the 15 SQL steps from ADR 0040)

Run in batches of 100k rows; use `SAVEPOINT` per batch so a single failed
batch retries without losing progress.

1. `wasm_interface_metadata` — `ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata`.
2. `ledgers` — `ON CONFLICT (sequence) DO NOTHING`.
3. `accounts` — remap pass: dedup by `account_id`; clauses per ADR 0040 (LEAST/GREATEST/sentinel-aware sequence_number/latest non-NULL home_domain); capture `RETURNING (id, account_id)` into `merge_remap.accounts(source_id, target_id)`.
4. `soroban_contracts` — remap pass: dedup by `contract_id`; COALESCE per nullable; `is_sac = OR`. **Omit `search_vector` from INSERT** (GENERATED ALWAYS; recomputed by Postgres). Capture remap.
5. `assets` — dedup-only via partial UNIQUEs; `GREATEST(asset_type)` with the SAC-prefer guard from `write.rs:1311–1314`. No remap needed (no FK referrers).
6. `liquidity_pools` — `ON CONFLICT (pool_id) DO UPDATE SET created_at_ledger = LEAST(...)`.
7. `nfts` — remap pass: dedup by `(contract_id, token_id)`. Do **not** copy source's `current_owner_*` — leave NULL/stale until Step 13 (`merge finalize`). Capture remap.
8. `transactions` — remap pass: dedup by `(hash, created_at)` via `uq_transactions_hash_created_at`; `DO UPDATE SET hash = EXCLUDED.hash` no-op for `RETURNING`. Capture `merge_remap.transactions(source_id, source_created_at, target_id, target_created_at)` — note `created_at` is part of the remap because partition routing depends on it.
9. `transaction_hash_index` — `ON CONFLICT (hash) DO NOTHING`.
10. Five appearance tables (`operations_appearances`, `transaction_participants`, `soroban_events_appearances`, `soroban_invocations_appearances`, `nft_ownership`) — FK rewrite via `JOIN merge_remap.<parent>` in the SELECT. **Build B-tree index on `merge_remap.<parent>(source_id)` before the JOINs** — without it, 150M-row JOINs do nested loops and never finish. ON CONFLICT … DO NOTHING on each table's natural-key UNIQUE/PK.
11. `liquidity_pool_snapshots` — `ON CONFLICT uq_lp_snapshots_pool_ledger DO NOTHING` (dedup-only).
12. `lp_positions` and `account_balances_current` (native + credit paths) — watermark-guarded UPSERT exactly mirroring `write.rs:1749–1754` and `write.rs:1866–1948`.
13. **(Deferred to `merge finalize`)** Rebuild `nfts.current_owner_*` from `nft_ownership` via `SELECT DISTINCT ON (nft_id) … ORDER BY nft_id, ledger_sequence DESC, event_order DESC`. Do NOT run after each `merge ingest`; only on the final invocation.
14. **(Deferred to `merge finalize`)** `setval(<seq>, MAX(id))` on all 7 sequences.
15. **(In each `merge ingest`)** Tear down the FDW bridge on the target:
    `DROP SCHEMA merge_source CASCADE; DROP USER MAPPING FOR CURRENT_USER SERVER merge_source_server; DROP SERVER merge_source_server;`.
    Optionally `docker compose stop postgres-snapshot-source` to release the
    volume; the next `merge ingest` will reset it anyway.

### Step 4: Diff harness (`merge diff --left <url> --right <url>`)

Build a separate utility that produces a **per-table normalized hash** for
two DBs. Approach:

- For each table, project rows to a SELECT that:
  - replaces every surrogate FK with the natural key (e.g.
    `transactions.source_id` → `(SELECT account_id FROM accounts WHERE id = source_id)`),
  - excludes auto-allocated surrogate `id` columns from the projection,
  - excludes `search_vector` (recomputed),
  - sorts deterministically by natural key.
- Compute `md5(string_agg(row::text, '|' ORDER BY natural_key))` per table.
- Output a 25-row table: `table | row_count_left | row_count_right | hash_left | hash_right | match?`.

Two DBs with identical _logical_ contents but different surrogate id
allocations will produce identical hashes. This is the **only credible
correctness check** for the merge.

### Step 5: Test corpus

Pick a small ledger range that exercises every code path (recommend
~10k ledgers around a known interesting block — Soroban activity, SAC,
NFT mints, LP activity). Then run, in order:

| Test                                     | Setup                                                                                                                               | Expected                                                                                                                                |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| **T1: First-snapshot edge case**         | Empty `postgres-merge` ← snapshot of `postgres-laptop-a`                                                                            | All remap tables degenerate (every natural key is new); diff vs `postgres-laptop-a` returns 25× match.                                  |
| **T2: Single-snapshot reproducibility**  | `postgres-truth` ← sequential backfill of laptop-a's range; `postgres-merge` ← snapshot of `postgres-laptop-a`                      | diff(`postgres-truth`, `postgres-merge`) returns 25× match. **This is the test that single-snapshot merge equals sequential backfill.** |
| **T3: Two-snapshot chronological merge** | `postgres-truth` ← sequential backfill of full range; `postgres-merge` ← snapshot of laptop-a, then laptop-b, then `merge finalize` | diff(`postgres-truth`, `postgres-merge`) returns 25× match. **The actual end-to-end correctness test.**                                 |
| **T4: Idempotency**                      | After T3, re-run `merge ingest <snapshot-a>` and `merge ingest <snapshot-b>` (replay)                                               | Zero new rows in any table; zero changes to watermark columns; diff still 25× match.                                                    |
| **T5: Wrong order rejected**             | After ingesting laptop-b, attempt to ingest laptop-a (older range)                                                                  | Pre-flight precondition aborts with "source range precedes target — chronological-only contract violated".                              |
| **T6: Scale smoke test**                 | Full-range pair (whatever the team has handy ≥10M ledgers per snapshot)                                                             | Completes; record wall-clock time, peak temp space, peak RSS. AC threshold below.                                                       |

---

## Acceptance Criteria

- [ ] `docker-compose.yml` has `postgres-truth`, `postgres-laptop-a`,
      `postgres-laptop-b`, `postgres-merge`, `postgres-snapshot-source`
      services on ports 5433–5437; both reset procedures (merge target,
      snapshot source) documented in script README.
- [ ] `merge ingest` automates the FDW setup (`CREATE EXTENSION
  postgres_fdw`, server, user mapping, `IMPORT FOREIGN SCHEMA public …
  INTO merge_source`) and tears it down on success.
- [ ] `crates/db-merge` exists with three subcommands: `ingest`, `finalize`,
      `diff`; CLI flags follow `backfill-runner` conventions.
- [ ] All 18 ADR-0040 table-by-table merge semantics implemented (collapsed
      into 15 substeps under task §"Step 3: Topological merge"); FK rewrites
      are JOIN-in-SELECT with B-tree indexes on remap tables; no post-insert
      UPDATE on partitioned tables.
- [ ] Per-table batching at 100k rows; `SAVEPOINT` per batch; failure of one
      batch retries without rolling back the whole table.
- [ ] Pre-merge precondition checks abort on: migration mismatch (incl.
      checksum), ledger-range overlap, source-precedes-target, partition
      drift, CHECK drift.
- [ ] `search_vector` excluded from `soroban_contracts` INSERT column list;
      Postgres recomputes on each insert (verified by post-insert
      `to_tsvector` parity check on a sample row).
- [ ] Pre-merge `pg_dump` taken; path printed to stderr; user owns cleanup.
- [ ] `merge finalize` runs Step 13 (`nfts.current_owner_*` rebuild) and
      Step 14 (`setval` all 7 sequences); idempotent.
- [ ] `merge diff` produces 25-row table with row counts + md5 per table on
      a normalized natural-key projection.
- [ ] **T1** (first-snapshot) passes: 25× match.
- [ ] **T2** (single-snapshot reproducibility) passes: 25× match.
- [ ] **T3** (two-snapshot chronological) passes: 25× match.
- [ ] **T4** (idempotency) passes: re-running ingest is a strict no-op
      (zero new rows, zero modified columns; diff still 25× match).
- [ ] **T5** (wrong order) passes: pre-flight rejects with actionable error.
- [ ] **T6** (scale smoke) passes thresholds: ≤4h wall-clock per ~10M-ledger
      snapshot on a workstation; peak temp space ≤30% of source dump size.
      (Adjust thresholds after first run; record actuals in task notes.)
- [ ] **Docs updated** — N/A: offline operational tool, not part of indexer/
      API/infra shape under `docs/architecture/**`. If `crates/db-merge`
      becomes a permanent piece of the pipeline (e.g. ongoing parallel
      backfill workflow), revisit
      `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md`.

---

## Notes

**Genuine open questions** (deliberately deferred — affect implementation
ergonomics, not correctness):

- **Snapshot transport.** Today the user copies snapshot files between
  laptops manually (USB / S3 / scp). Out of scope for this task; the script
  takes a local path.
- **Resume after partial `merge ingest` failure.** If a `SAVEPOINT` batch
  fails mid-table after 50k of 200k rows are committed, on retry should the
  script skip already-merged rows automatically (via `ON CONFLICT DO NOTHING`
  semantics, which is already in place) or expose `--from-batch N`? Default:
  rely on ON CONFLICT idempotency; add `--from-batch` only if T4 testing
  reveals replay performance issues.
- **Concurrent `merge ingest` invocations.** Forbidden — script takes a
  Postgres advisory lock at start. Worth asserting in T-something.
- **Source DB live during merge.** The ADR assumes source is dumped first,
  not connected live. Live-source merge is a future variant (skip pg_dump
  step), out of scope here.

**Cross-references** for SQL clauses every step uses (line numbers from
verifier passes underlying ADR 0040, not from the ADR text itself; useful
when re-reading `crates/indexer/src/handler/persist/write.rs` to confirm
the exact `ON CONFLICT` wording before transcribing it into the merge
script):
518 (ledgers), 595–596 (transactions), 649 (transaction_hash_index),
86–125 (accounts), 410–427 (soroban_contracts), 158–164 (wasm), 1077–1329
(assets 4 paths), 1490–1497 (nfts LWW), 1567 (nft_ownership append), 700
(transaction_participants), 806 (operations_appearances), 902
(soroban_events_appearances), 1060 (soroban_invocations_appearances),
1643–1645 (liquidity_pools), 1702 (lp_snapshots), 1749–1754 (lp_positions),
1866–1948 (account_balances_current).
