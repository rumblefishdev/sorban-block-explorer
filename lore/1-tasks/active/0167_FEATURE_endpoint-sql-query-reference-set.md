---
id: '0167'
title: 'API: hand-tuned SQL query reference set, one script per endpoint'
type: FEATURE
status: active
related_adr:
  ['0037', '0025', '0026', '0029', '0030', '0031', '0033', '0034', '0036']
related_tasks: ['0043', '0050', '0123']
tags: [api, sql, performance, reference, postgres]
links:
  - 'docs/architecture/backend/backend-overview.md §6.2'
  - 'docs/architecture/frontend/frontend-overview.md §6'
  - 'lore/2-adrs/0037_current-schema-snapshot.md'
history:
  - date: 2026-04-27
    status: backlog
    who: fmazur
    note: 'Task created — produce one hand-tuned SQL script per public REST endpoint, sourced from ADR 0037 schema'
  - date: 2026-04-27
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
---

# API: hand-tuned SQL query reference set, one script per endpoint

## Summary

Produce a self-contained directory of hand-written, performance-tuned SQL scripts —
**one file per public REST endpoint** defined in `backend-overview.md §6.2`. Each
script must answer the read shape of its endpoint as a senior DB engineer would write it
against the live schema in [ADR 0037](../../2-adrs/0037_current-schema-snapshot.md):
partition-pruned, index-aware, surrogate-key joined, no client-side post-processing.

These scripts become the canonical reference the `crates/api` modules implement against
(via `sqlx::query!`/`query_as!`). This task ships **only the SQL** — no Rust wiring.

## Status: Active

**Current state:** Specification ready. SQL scripts not yet authored.

## Context

`crates/api` is still skeletal (per `backend-overview.md §10`). Before route handlers
get wired, every endpoint needs a deliberate, reviewable read plan against the live
schema. The schema has been reshaped 12 migrations deep (ADRs 0020, 0023–0026, 0030–0036
plus task 0163's `operations_appearances` collapse), so naive queries written from the
endpoint contract alone will miss the surrogate-key boundary
([ADR 0026](../../2-adrs/0026_accounts-surrogate-bigint-id.md),
[ADR 0030](../../2-adrs/0030_contracts-surrogate-bigint-id.md)),
the `SMALLINT` enum decoding helpers ([ADR 0031](../../2-adrs/0031_enum-columns-smallint-with-rust-enum.md)),
the BYTEA-32 hash convention ([ADR 0024](../../2-adrs/0024_hashes-bytea-binary-storage.md)),
the appearance-index read paths ([ADRs 0033](../../2-adrs/0033_soroban-events-appearances-read-time-detail.md),
[0034](../../2-adrs/0034_soroban-invocations-appearances-read-time-detail.md)), and
the read-time XDR fetch boundary ([ADR 0029](../../2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)).

Producing the SQL up front, in one place, lets us:

- review every endpoint's query plan as a senior eng would (one PR, one perf pass)
- confirm every partition-pruned read carries a `created_at` predicate
- catch missing indexes early (feeds task **0132**)
- give the API author a copy-pasteable starting point that already encodes the
  surrogate-resolve / decode / enrich pattern

## Data Source Boundary (DB vs S3 vs Archive)

The SQL set encodes a deliberate split between Postgres (the index / summary
layer) and external blob stores (the heavy-payload layer):

- **List endpoints — DB only.** Every `GET /<resource>` and every nested
  `GET /<resource>/:id/<sublist>` answers entirely from Postgres. No S3 fetch,
  no archive fetch. The DB columns must be sufficient to render the list rows
  end-to-end (table-style browsing per `frontend-overview.md §6`).
- **Detail endpoints — DB + on-demand S3 / archive overlay.** A single-entity
  `GET /<resource>/:id` typically reads a header row from Postgres and then the
  API enriches the response with a per-entity blob fetched from S3, using a
  bridge column from the DB row as the S3 key. The SQL's job is to surface
  that bridge column; it does **not** reach into the blob.
- **Exception — list-like content embedded in a detail endpoint.** A few
  detail endpoints expose a list whose full payload lives off-DB. The
  canonical example is **`GET /ledgers/:sequence`**: the body includes the
  list of transactions in that ledger, which is loaded from
  `s3://.../parsed_ledger_{ledger_sequence}.json` (the S3 layout per ADR 0011 /
  ADR 0037 §11 note on `ledger_sequence` as bridge column), **not** from a
  partition-pruned read of the `transactions` table. For these the SQL
  returns the header row + the S3-pointer column only; it does not query the
  embedded sublist from Postgres at all.

The two categories of off-DB sources are distinct and must be kept apart in
the file headers:

| Source                            | Path / shape                                                                   | Used by                                                                             | Driver                                                                        |
| --------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Public Stellar ledger archive** | `<ledger>.xdr.zst`, decompressed + parsed at request time                      | E3 envelope/result/result_meta + parsed invocation tree; E14 full event topics/data | [ADR 0029](../../2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) |
| **Explorer S3 — per-ledger blob** | `s3://<bucket>/parsed_ledger_{N}.json`                                         | E5 transactions-in-ledger sublist                                                   | ADR 0037 §11 (bridge column note)                                             |
| **Explorer S3 — per-entity blob** | `s3://<bucket>/assets/{id}.json` (and similar per-entity layouts as they land) | E9 `description` / `home_page`                                                      | ADR 0037 §11 / task **0164**                                                  |

Implementation rule for every `.sql` file: the header block must carry a
`Data sources:` line listing **DB / S3 / Archive** and naming exactly which
response fields come from each. If a field doesn't come from the DB, the file
must also leave a `-- not in DB: <field> — see <ADR / task>` comment in the
projection so the API author can't miss the overlay step.

## Deliverable

A new directory **`docs/architecture/database-schema/endpoint-queries/`** containing
**22 SQL files**, one per endpoint, plus a `README.md` index.

Filename convention: `NN_<method>_<slug>.sql` — numbering matches the table below so
files sort in endpoint-inventory order.

```
docs/architecture/database-schema/endpoint-queries/
├── README.md
├── 01_get_network_stats.sql
├── 02_get_transactions_list.sql
├── 03_get_transactions_by_hash.sql
├── 04_get_ledgers_list.sql
├── 05_get_ledgers_by_sequence.sql
├── 06_get_accounts_by_id.sql
├── 07_get_accounts_transactions.sql
├── 08_get_assets_list.sql
├── 09_get_assets_by_id.sql
├── 10_get_assets_transactions.sql
├── 11_get_contracts_by_id.sql
├── 12_get_contracts_interface.sql
├── 13_get_contracts_invocations.sql
├── 14_get_contracts_events.sql
├── 15_get_nfts_list.sql
├── 16_get_nfts_by_id.sql
├── 17_get_nfts_transfers.sql
├── 18_get_liquidity_pools_list.sql
├── 19_get_liquidity_pools_by_id.sql
├── 20_get_liquidity_pools_transactions.sql
├── 21_get_liquidity_pools_chart.sql
└── 22_get_search.sql
```

### File structure (each `.sql`)

```sql
-- Endpoint:     GET /<path>
-- Purpose:      one-line summary of what response field-set this returns
-- Source:       backend-overview.md §6.2 / frontend-overview.md §6.<N>
-- Schema:       ADR 0037
-- Data sources: DB-only  |  DB + S3 per-entity blob (path)
--                       |  DB + S3 per-ledger blob (path)
--                       |  DB + Archive XDR (ADR 0029)
--               One line per source actually used. Name which response fields
--               come from each. List endpoints must read "DB-only".
-- Inputs:       :param_1 (TYPE), :param_2 (TYPE), ... (sqlx-style placeholders)
-- Indexes:      list of indexes the planner is expected to use
-- Notes:        partition-prune key, surrogate-resolve approach, anything non-obvious

-- Optional CTEs / surrogate resolves
WITH resolved AS (...)
SELECT
    ...,
    -- not in DB: <field> — see <ADR / task>     -- only when the API overlays from S3/archive
FROM ...
WHERE ...
ORDER BY ...
LIMIT :limit;
```

If an endpoint genuinely needs multiple round-trips for performance (e.g. resolve a
StrKey to a surrogate id, then run the partition-pruned query), the file may contain
**multiple statements separated by `-- @@ split @@`**. Default is one statement per file.

## Implementation Plan

### Step 1: Lock the per-endpoint contract from the docs

For each of the 22 endpoints, read off `backend-overview.md §6.3` plus the matching
frontend page in `frontend-overview.md §6.x` and write down — in the `Notes` section
of this task — the columns the response actually needs. The frontend page is the
authority on what gets rendered; the backend page is the authority on filters, params,
and the dual normal/advanced contract for `/transactions/:hash`.

### Step 2: Write each query against ADR 0037

Apply these conventions everywhere:

- **Partition pruning.** Every read against `transactions`, `operations_appearances`,
  `transaction_participants`, `soroban_events_appearances`,
  `soroban_invocations_appearances`, `nft_ownership`, `liquidity_pool_snapshots`
  must carry a `created_at` predicate. For "by hash" lookups, resolve via
  `transaction_hash_index` first to get `(ledger_sequence, created_at)`, then
  query the partitioned table with `created_at = $resolved_created_at`.
- **Surrogate-key boundary** ([ADR 0026](../../2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../2-adrs/0030_contracts-surrogate-bigint-id.md)). StrKey route
  parameters resolve to `accounts.id` / `soroban_contracts.id` via the unique
  index at the request boundary; every internal join uses the BIGINT.
  Response StrKeys come from a `JOIN accounts ON ...` (or contracts) at the very
  end. Encode this explicitly with a leading `WITH acc AS (SELECT id FROM accounts WHERE account_id = $1)`.
- **Enum decoding** ([ADR 0031](../../2-adrs/0031_enum-columns-smallint-with-rust-enum.md)).
  Use the `*_name(smallint)` helpers in the projection only; never in `WHERE`.
  `WHERE` clauses always compare against the SMALLINT literal so indexes are usable.
- **Hash inputs.** Every hash parameter is `BYTEA` (32 bytes). Document the
  expected encoding (raw bytes vs hex-decoded) in the file header.
- **Cursor pagination** ([ADR 0025](../../2-adrs/0025_final-schema-and-endpoint-realizability.md),
  task 0043). Use keyset pagination on `(created_at DESC, id DESC)` (or the
  table's natural ordering key). No `OFFSET`. No `COUNT(*)`. Cursor is opaque.
- **Filters.** Predicates that match the partial / functional indexes in ADR 0037
  must use the same predicate shape. Examples:
  - `idx_tx_has_soroban` is a partial — soroban-only filters use `WHERE has_soroban`.
  - `idx_ops_app_*` partials — only filter on the column the partial covers.
  - `idx_assets_code_trgm` / `idx_nfts_name_trgm` — code/name search uses
    `gin_trgm_ops` operators (`%`, `LIKE`/`ILIKE` with leading `%`).
  - `idx_contracts_search` — full-text uses `search_vector @@ plainto_tsquery(...)`.
- **No SELECT \*.** Project the exact columns required by the endpoint response.
- **`EXPLAIN (ANALYZE, BUFFERS)`-friendly.** Avoid patterns that defeat partition
  pruning (e.g. wrapping `created_at` in functions, using non-sargable predicates).

### Step 3: Per-endpoint specifics

The 22 endpoints. Notes below capture the non-obvious bits — the file headers
must restate them.

| #   | Endpoint                                  | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| --- | ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 01  | `GET /network/stats`                      | **DB-only.** Single roundtrip, ideally one statement: `MAX(sequence)`, `COUNT(*)` from `accounts`, `COUNT(*)` from `soroban_contracts`, plus a TPS window over the last N ledgers (`WHERE closed_at >= now() - interval '60 seconds'`). Fast & cacheable.                                                                                                                                                                                                                                                                                                                                                                        |
| 02  | `GET /transactions`                       | **DB-only.** Partitioned scan with `created_at` window and keyset cursor. Filters: `source_id` (resolve from StrKey first), `contract_id` (forces a route through `operations_appearances`/`soroban_invocations_appearances` — document the join strategy), `type` SMALLINT. Project `accounts.account_id` for source via final join.                                                                                                                                                                                                                                                                                            |
| 03  | `GET /transactions/:hash`                 | **DB + Archive.** Two statements minimum: (a) resolve `transaction_hash_index → (ledger_sequence, created_at)`; (b) hydrate from `transactions` partition + linked `operations_appearances`, `soroban_events_appearances`, `soroban_invocations_appearances`, `transaction_participants`. Per [ADR 0029](../../2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) the **raw envelope/result/result_meta XDR + the parsed invocation tree are fetched from the public ledger archive at request time, not the DB** — leave a `-- not in DB: envelope_xdr/result_xdr/result_meta_xdr/operation_tree — ADR 0029` comment. |
| 04  | `GET /ledgers`                            | **DB-only.** Pure `ledgers` keyset on `closed_at DESC` (uses `idx_ledgers_closed_at`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| 05  | `GET /ledgers/:sequence`                  | **DB header + S3 per-ledger blob — exception case.** Return only the `ledgers` row plus `ledger_sequence` as the S3 bridge key (`s3://<bucket>/parsed_ledger_{ledger_sequence}.json`); the API loads the embedded transaction list from S3, not from the `transactions` partition. **Do not** query `transactions` here — the per-ledger blob is the single source of truth for the embedded list (ADR 0037 §11 bridge-column note, ADR 0011 layout). Leave a `-- not in DB: transactions[] — S3 parsed_ledger_{N}.json` comment.                                                                                                |
| 06  | `GET /accounts/:account_id`               | **DB-only.** Resolve account StrKey → id, then read `accounts` row + `account_balances_current` rows. Use the partial unique indexes `uidx_abc_native` / `uidx_abc_credit`. Issuer StrKeys come from a final join back to `accounts`.                                                                                                                                                                                                                                                                                                                                                                                            |
| 07  | `GET /accounts/:account_id/transactions`  | **DB-only.** Resolve account → id, then keyset on `transaction_participants (account_id, created_at, transaction_id)` joined to `transactions` (composite FK `(transaction_id, created_at)`). The PK ordering on `transaction_participants` is the natural cursor key.                                                                                                                                                                                                                                                                                                                                                           |
| 08  | `GET /assets`                             | **DB-only.** `assets` keyset; filters: `asset_type` SMALLINT, `asset_code ILIKE :pattern` via `idx_assets_code_trgm`. Issuer StrKey via final join to `accounts`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| 09  | `GET /assets/:id`                         | **DB + S3 per-entity blob.** Single-row by `assets.id` + final joins for issuer StrKey and contract StrKey. The DB row provides `name` / `icon_url` / type / supply / holder count; `description` and `home_page` are overlaid by the API from `s3://<bucket>/assets/{id}.json` (ADR 0037 §11 / task **0164**). Leave a `-- not in DB: description, home_page — S3 assets/{id}.json` comment.                                                                                                                                                                                                                                    |
| 10  | `GET /assets/:id/transactions`            | **DB-only.** Read the asset row to derive the identity tuple `(asset_code, issuer_id)` (or contract for SAC/Soroban), then keyset on `operations_appearances` filtered by either `(asset_code, asset_issuer_id)` (uses `idx_ops_app_asset`) or `contract_id` (uses `idx_ops_app_contract`), joined to `transactions` via `(transaction_id, created_at)`. Document the asset-type branching as two query variants in the file.                                                                                                                                                                                                    |
| 11  | `GET /contracts/:contract_id`             | **DB-only.** Resolve contract StrKey → id, then `soroban_contracts` row + deployer join + cheap stats (`COUNT(*)` from `soroban_invocations_appearances` / `COUNT(DISTINCT caller_id)`) **scoped to a recent partition window** — never a full-history count. Document the time window in the header.                                                                                                                                                                                                                                                                                                                            |
| 12  | `GET /contracts/:contract_id/interface`   | **DB-only.** `soroban_contracts.wasm_hash → wasm_interface_metadata.metadata` (JSONB). Project the function-list slice of the JSONB; keep all `jsonb_*` work in the projection.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| 13  | `GET /contracts/:contract_id/invocations` | **DB-only.** Resolve contract → id, then keyset on `soroban_invocations_appearances (contract_id, ledger_sequence DESC)` joined to `transactions` via `(transaction_id, created_at)` for tx hash + status. Caller StrKey via final join to `accounts`.                                                                                                                                                                                                                                                                                                                                                                           |
| 14  | `GET /contracts/:contract_id/events`      | **DB index + Archive XDR.** Resolve contract → id, then keyset on `soroban_events_appearances` (uses `idx_sea_contract_ledger`). Per [ADR 0033](../../2-adrs/0033_soroban-events-appearances-read-time-detail.md) the table is the **appearance index only** — full event detail (topics, data) is parsed from the archive XDR at request time (ADR 0029). The SQL returns the appearance rows + per-row `(ledger_sequence, transaction_id, created_at)` so the API can fan out to the archive. Leave a `-- not in DB: topics, data — Archive XDR (ADR 0029, ADR 0033)` comment.                                                 |
| 15  | `GET /nfts`                               | **DB-only.** `nfts` keyset; filters: `collection_name` (uses `idx_nfts_collection`), `contract_id` (resolve StrKey first), `name ILIKE` via `idx_nfts_name_trgm`. Owner StrKey via final join to `accounts`.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| 16  | `GET /nfts/:id`                           | **DB-only (today).** Single-row `nfts` + owner join. `metadata` is JSONB — project it as-is for the API to shape. If a per-NFT S3 enrichment layout lands later (parallel to assets/{id}.json), revise the file to overlay; until then the JSONB row is the full source.                                                                                                                                                                                                                                                                                                                                                         |
| 17  | `GET /nfts/:id/transfers`                 | **DB-only.** Keyset on `nft_ownership` PK `(nft_id, created_at, ledger_sequence, event_order)`. Both former and current owner StrKeys come from joins to `accounts`. Decode `event_type` via `nft_event_type_name`.                                                                                                                                                                                                                                                                                                                                                                                                              |
| 18  | `GET /liquidity-pools`                    | **DB-only.** Two-step query: latest snapshot per pool via `DISTINCT ON (pool_id)` against `liquidity_pool_snapshots` ordered by `(pool_id, created_at DESC)` within a recent window, joined to `liquidity_pools`. Filter `min_tvl` uses `idx_lps_tvl`. Document the recent-window predicate.                                                                                                                                                                                                                                                                                                                                     |
| 19  | `GET /liquidity-pools/:id`                | **DB-only.** `liquidity_pools` row + latest snapshot via `DISTINCT ON (pool_id)` (or `ORDER BY ledger_sequence DESC LIMIT 1` with explicit `created_at` window). Issuer StrKeys via final joins.                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 20  | `GET /liquidity-pools/:id/transactions`   | **DB-only.** Keyset on `operations_appearances WHERE pool_id = $1` (uses `idx_ops_app_pool`) + tx join.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| 21  | `GET /liquidity-pools/:id/chart`          | **DB-only.** Time-bucketed aggregation on `liquidity_pool_snapshots`: `date_trunc(:interval, created_at)` over `[from, to]`, returning per-bucket TVL/volume/fee_revenue. `interval` is a parameter from the endpoint (`1h`/`1d`/`1w`); the SQL accepts a `text` interval and uses `date_trunc`.                                                                                                                                                                                                                                                                                                                                 |
| 22  | `GET /search`                             | **DB-only.** One union-style script with a CTE per entity type, each CTE using the right index: `idx_accounts_prefix` (account StrKey prefix), `idx_contracts_prefix` (contract StrKey prefix), `transaction_hash_index` (exact hash by `BYTEA`), `idx_assets_code_trgm` (code), `idx_nfts_name_trgm` (name), `idx_contracts_search` (full-text via `tsvector @@ plainto_tsquery`). Each CTE bounded `LIMIT N` so the union is small. The endpoint also classifies queries — capture the classification in SQL only where it can be expressed cheaply (e.g. by-shape detection lives in the API layer, not SQL).                 |

### Step 4: Write `README.md` index

A short index (~50 lines) listing each script with a one-line description and the
endpoint it implements. Cross-link to ADR 0037 and the two architecture docs.

### Step 5: Sanity check against indexes

Before closing the task, run each query under `EXPLAIN` against the local Docker
Postgres (the user runs benchmarks themselves — prepare the scripts; do not run
them). Each query plan should show the expected index from the per-endpoint table
above. Any plan that does a `Seq Scan` on a partitioned table is a bug — either
the predicate is wrong or there's a missing index (feed the latter into task **0132**).

## Acceptance Criteria

- [ ] Directory `docs/architecture/database-schema/endpoint-queries/` exists with
      22 `.sql` files matching the filename convention above.
- [ ] Every file has a header comment block listing endpoint, purpose, source,
      schema reference (ADR 0037), **`Data sources:` line (DB-only / DB+S3 /
      DB+Archive) naming exactly which response fields come from each**,
      inputs, expected indexes, and notes.
- [ ] Every list endpoint (E2, E4, E7, E8, E10, E13, E14-as-list, E15, E17,
      E18, E20, E22) reads `Data sources: DB-only` and answers entirely from
      Postgres — no S3, no archive (E14 is the documented exception: the
      list rows themselves are DB, the per-row enrichment is archive XDR).
- [ ] Every detail endpoint with off-DB overlay (E3, E5, E9, E14) carries the
      matching `-- not in DB: <field> — <S3 path / Archive — ADR ref>`
      comment directly in the projection so the API author can't miss the
      overlay step. E5 in particular **must not** query the `transactions`
      partition for the embedded list — only the `ledgers` row plus
      `ledger_sequence` as the S3 bridge key.
- [ ] Every query against a partitioned table carries a `created_at` predicate
      capable of partition pruning.
- [ ] Every StrKey input parameter is resolved to its surrogate id via a leading
      CTE; every StrKey in the result set comes from a final join to
      `accounts.account_id` / `soroban_contracts.contract_id`.
- [ ] No `SELECT *`, no `OFFSET` for pagination, no `COUNT(*)` on full history.
- [ ] Filters on enum columns compare to SMALLINT literals (not `*_name(...)`).
- [ ] Endpoints E3 and E14 carry an explicit `-- not in DB: ... — ADR 0029`
      comment for the XDR-derived fields.
- [ ] `README.md` index exists and links each script to its endpoint plus
      back-links to `backend-overview.md §6.2`, `frontend-overview.md §6`, and
      ADR 0037.
- [ ] **Docs updated** — N/A — this task **adds** a docs subtree (`docs/architecture/database-schema/endpoint-queries/`).
      No existing doc shape changes; per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md)
      the task itself is the doc delta. If endpoint contracts in `backend-overview.md §6` shift while
      writing the SQL, update those docs in the same PR.

## Notes

- **No Rust here.** The handler wiring (sqlx `query_as!`, response DTOs) is
  out of scope; that lands per-module in tasks like 0050 (Contracts), 0123
  (XDR decoding service), and follow-ups. This task delivers a reviewable,
  copy-pasteable SQL reference set only.
- **Out of scope (do not auto-spawn follow-up tasks for these — owner
  decides):**
  - missing-index findings → existing task **0132**
  - holder-count freshness → existing task **0135**
  - LP price oracle / TVL → existing task **0125**
  - read-time XDR service → existing task **0123**
- **Schema reference is authoritative.** If anything in `backend-overview.md`
  conflicts with ADR 0037 (e.g. column names, types), the schema wins and
  the conflict is logged as a doc-drift note in the task PR.
