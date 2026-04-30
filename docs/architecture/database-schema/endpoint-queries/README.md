# Endpoint SQL query reference set

Hand-tuned read queries — **one script per public REST endpoint** defined in
[`backend-overview.md §6.2`](../../backend/backend-overview.md#62-endpoint-inventory).
Schema reference: [ADR 0037](../../../../lore/2-adrs/0037_current-schema-snapshot.md).
Driving task: [0167](../../../../lore/1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md).

These files are the canonical Postgres-side read plan that `crates/api` modules
implement against (via `sqlx::query!`/`query_as!`). They are **reference SQL**,
not migration scripts — nothing in this directory is executed by the runtime.

## Conventions

Every file in this directory must:

- carry the header block defined in task **0167** (Endpoint / Purpose / Source /
  Schema / Data sources / Inputs / Indexes / Notes)
- partition-prune every read against the seven partitioned tables
  (`transactions`, `operations_appearances`, `transaction_participants`,
  `soroban_events_appearances`, `soroban_invocations_appearances`,
  `nft_ownership`, `liquidity_pool_snapshots`) by carrying a `created_at`
  predicate
- resolve every StrKey route parameter to its `BIGINT` surrogate
  ([ADR 0026](../../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md))
  via a leading CTE and join back to `accounts.account_id` /
  `soroban_contracts.contract_id` in the final projection
- compare enum columns to `SMALLINT` literals in `WHERE` and decode via
  `*_name(smallint)` helpers only in the projection
  ([ADR 0031](../../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
- use keyset (cursor) pagination — never `OFFSET`, never full-history `COUNT(*)`
- declare expected indexes in the header so a reviewer can confirm the plan
  without running `EXPLAIN`

## Data source boundary

Per task 0167 §"Data Source Boundary":

| Source                                                                                                                            | Used by                                                                                 |
| --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| **DB-only** (Postgres)                                                                                                            | All list endpoints + most detail endpoints                                              |
| **DB + Public Stellar ledger archive** ([ADR 0029](../../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)) | E3 (envelope/result/result_meta + parsed invocation tree), E14 (full event topics/data) |
| **DB + S3 per-ledger blob** (`s3://<bucket>/parsed_ledger_{N}.json`)                                                              | E5 (transactions-in-ledger sublist)                                                     |
| **DB + S3 per-entity blob** (`s3://<bucket>/assets/{id}.json`)                                                                    | E9 (`description`, `home_page`) — task **0164**                                         |

The SQL never reaches into S3 / archive. It surfaces the bridge column the API
layer uses to fetch the blob and marks the off-DB fields with
`-- not in DB: <field> — <source — ADR ref>`.

## Index

| #   | File                                                                                 | Endpoint                                  | Source         |
| --- | ------------------------------------------------------------------------------------ | ----------------------------------------- | -------------- |
| 01  | [`01_get_network_stats.sql`](01_get_network_stats.sql)                               | `GET /network/stats`                      | DB-only        |
| 02  | [`02_get_transactions_list.sql`](02_get_transactions_list.sql)                       | `GET /transactions`                       | DB-only        |
| 03  | [`03_get_transactions_by_hash.sql`](03_get_transactions_by_hash.sql)                 | `GET /transactions/:hash`                 | DB + Archive   |
| 04  | [`04_get_ledgers_list.sql`](04_get_ledgers_list.sql)                                 | `GET /ledgers`                            | DB-only        |
| 05  | [`05_get_ledgers_by_sequence.sql`](05_get_ledgers_by_sequence.sql)                   | `GET /ledgers/:sequence`                  | DB + S3 ledger |
| 06  | [`06_get_accounts_by_id.sql`](06_get_accounts_by_id.sql)                             | `GET /accounts/:account_id`               | DB-only        |
| 07  | [`07_get_accounts_transactions.sql`](07_get_accounts_transactions.sql)               | `GET /accounts/:account_id/transactions`  | DB-only        |
| 08  | [`08_get_assets_list.sql`](08_get_assets_list.sql)                                   | `GET /assets`                             | DB-only        |
| 09  | [`09_get_assets_by_id.sql`](09_get_assets_by_id.sql)                                 | `GET /assets/:id`                         | DB + S3 entity |
| 10  | [`10_get_assets_transactions.sql`](10_get_assets_transactions.sql)                   | `GET /assets/:id/transactions`            | DB-only        |
| 11  | [`11_get_contracts_by_id.sql`](11_get_contracts_by_id.sql)                           | `GET /contracts/:contract_id`             | DB-only        |
| 12  | [`12_get_contracts_interface.sql`](12_get_contracts_interface.sql)                   | `GET /contracts/:contract_id/interface`   | DB-only        |
| 13  | [`13_get_contracts_invocations.sql`](13_get_contracts_invocations.sql)               | `GET /contracts/:contract_id/invocations` | DB-only        |
| 14  | [`14_get_contracts_events.sql`](14_get_contracts_events.sql)                         | `GET /contracts/:contract_id/events`      | DB + Archive   |
| 15  | [`15_get_nfts_list.sql`](15_get_nfts_list.sql)                                       | `GET /nfts`                               | DB-only        |
| 16  | [`16_get_nfts_by_id.sql`](16_get_nfts_by_id.sql)                                     | `GET /nfts/:id`                           | DB-only        |
| 17  | [`17_get_nfts_transfers.sql`](17_get_nfts_transfers.sql)                             | `GET /nfts/:id/transfers`                 | DB-only        |
| 18  | [`18_get_liquidity_pools_list.sql`](18_get_liquidity_pools_list.sql)                 | `GET /liquidity-pools`                    | DB-only        |
| 19  | [`19_get_liquidity_pools_by_id.sql`](19_get_liquidity_pools_by_id.sql)               | `GET /liquidity-pools/:id`                | DB-only        |
| 20  | [`20_get_liquidity_pools_transactions.sql`](20_get_liquidity_pools_transactions.sql) | `GET /liquidity-pools/:id/transactions`   | DB-only        |
| 21  | [`21_get_liquidity_pools_chart.sql`](21_get_liquidity_pools_chart.sql)               | `GET /liquidity-pools/:id/chart`          | DB-only        |
| 22  | [`22_get_search.sql`](22_get_search.sql)                                             | `GET /search`                             | DB-only        |
| 23  | [`23_get_liquidity_pools_participants.sql`](23_get_liquidity_pools_participants.sql) | `GET /liquidity-pools/:id/participants`   | DB-only        |

## Cursor encoding (shared convention)

Every list endpoint uses **keyset pagination** on its natural ORDER BY tuple.
The API layer base64-encodes / decodes the tuple; the SQL accepts the tuple as
typed `:cursor_*` parameters and uses row-value comparison
(`(a, b) < ($cursor_a, $cursor_b)`) so the planner can use the existing
ordered indexes.

A first-page request passes `NULL` for every `:cursor_*` parameter; the SQL
treats `($cursor_a IS NULL OR (a, b) < ($cursor_a, $cursor_b))` so a single
prepared statement covers both first and subsequent pages.

## Statement separator

Files containing more than one statement separate them with the literal token
`-- @@ split @@` on its own line. The API loader splits on this token. Single-
statement files have no separator.

## Multi-statement vs single-statement

Default is **one statement per file**. Multiple are allowed only when a
StrKey resolve must precede the partition-pruned read for performance, or
when a detail endpoint genuinely needs N independent reads (e.g. E3 fans out
from a resolved `(transaction_id, created_at)` tuple to four child tables).

---

## Endpoint response shapes

For each endpoint, this section lists the fields the API serves and where
they come from. `DB →` means the SQL projects the column directly. `S3 →`
or `Archive →` means the SQL surfaces a bridge column and the API fetches
the blob using that bridge, then merges the requested fields into the
response. The architectural rule (task 0167 §"Data Source Boundary") is:
list endpoints are DB-only; detail endpoints may overlay; the documented
exception is `/ledgers/:sequence` whose embedded transactions list is fully
S3-served.

### 01. `GET /network/stats`

**Source:** DB-only.

**Final response:**

| Field                     | Source                                                                                          |
| ------------------------- | ----------------------------------------------------------------------------------------------- |
| `latest_ledger_sequence`  | DB → `ledgers.sequence` (newest by `closed_at`)                                                 |
| `latest_ledger_closed_at` | DB → `ledgers.closed_at` (newest) — drives §7 polling indicator                                 |
| `generated_at`            | DB → `NOW()` at SELECT time; preserved across cache hits so client can split lag from staleness |
| `tps_60s`                 | DB → `SUM(transaction_count)/window_seconds` over trailing 60 s, cast `::float8`                |
| `total_accounts`          | DB → `pg_class.reltuples` for `accounts` (planner estimate, not exact)                          |
| `total_contracts`         | DB → `pg_class.reltuples` for `soroban_contracts`                                               |

### 02. `GET /transactions`

**Source:** DB-only.

**Final response (per row):**

| Field               | Source                                                                                                                                                         |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hash`              | DB → `encode(transactions.hash, 'hex')`                                                                                                                        |
| `ledger_sequence`   | DB → `transactions.ledger_sequence`                                                                                                                            |
| `application_order` | DB → `transactions.application_order`                                                                                                                          |
| `source_account`    | DB → `accounts.account_id` (G-StrKey) via FK join on `source_id`                                                                                               |
| `fee_charged`       | DB → `transactions.fee_charged` (stroops)                                                                                                                      |
| `inner_tx_hash`     | DB → `encode(transactions.inner_tx_hash, 'hex')` (NULL when not fee-bumped)                                                                                    |
| `successful`        | DB → `transactions.successful` (drives status badge)                                                                                                           |
| `operation_count`   | DB → `transactions.operation_count`                                                                                                                            |
| `has_soroban`       | DB → `transactions.has_soroban`                                                                                                                                |
| `operation_types[]` | DB → `array_agg(DISTINCT op_type_name(oa.type))` from a LATERAL into `operations_appearances` keyed on the composite FK; powers §6.3's "operation type" column |
| `created_at`        | DB → `transactions.created_at`                                                                                                                                 |
| `cursor`            | DB → `(created_at, id)` pair, opaque to clients                                                                                                                |

Two statement variants in the file: A (no contract / op_type filter) and B (driven from `operations_appearances`).

### 03. `GET /transactions/:hash`

**Source:** DB + Archive XDR (ADR 0029).

The endpoint is the heaviest in the system. The API runs **6 SQL statements**, fetches the transaction's `.xdr.zst` slice from the public Stellar ledger archive, parses it with `crates/xdr-parser`, and merges the decoded payload into the response.

**Step 1 — DB resolve & header (statements A → B):**

| Field                                                                                                                                                                                                      | Source                                       |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| `transaction_id`, `hash`, `ledger_sequence`, `application_order`, `source_account` (G-StrKey), `fee_charged`, `inner_tx_hash`, `successful`, `operation_count`, `has_soroban`, `parse_error`, `created_at` | DB → `transactions` row + join to `accounts` |

**Step 2 — Archive overlay on the header:**

The API uses the resolved `(ledger_sequence, hash)` to fetch the per-ledger `.xdr.zst`, decompresses and parses, then extracts:

| Field                                                | Source                                                                                               |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `envelope_xdr`, `result_xdr`, `result_meta_xdr`      | Archive → raw XDR strings (Advanced mode collapsible sections)                                       |
| `memo_type`, `memo_content`                          | Archive → decoded from `envelope_xdr`                                                                |
| `signatures[]` (`signer`, `weight`, `signature_hex`) | Archive → decoded from `envelope_xdr`                                                                |
| `operation_tree`                                     | Archive → parsed invocation/op tree from `envelope_xdr` + `result_meta_xdr` (Normal mode graph/tree) |

**Step 3 — DB operations (statement C):**

| Field                                                                                                                                                                                     | Source                                                                                                                                                                                                               |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Per op: `appearance_id`, `type` (SMALLINT), `type_name`, `source_account`, `destination_account`, `contract_id`, `asset_code`, `asset_issuer`, `pool_id`, `ledger_sequence`, `created_at` | DB → `operations_appearances` + joins for StrKey resolution. `operations_appearances.amount` is a fold count (task 0163), not a stroop value — per-op stroop amounts come from the archive overlay below (ADR 0029). |

**Step 4 — Archive overlay on operations:**

Using the same parsed envelope as step 2, the API enriches each op row:

| Field                                  | Source                                                                  |
| -------------------------------------- | ----------------------------------------------------------------------- |
| `parameters` (raw operation arguments) | Archive → per-op entry in `envelope_xdr`                                |
| `return_value`                         | Archive → per-op entry in `result_meta_xdr`                             |
| Operation position 1..N                | Result-set row index (NOT `appearance_id`, which is a global BIGSERIAL) |

**Step 5 — DB participants (D):**

| Field                        | Source                                               |
| ---------------------------- | ---------------------------------------------------- |
| `participants[]` (G-StrKeys) | DB → `transaction_participants` joined to `accounts` |

**Step 6 — DB events (E) + Archive overlay:**

| Field                                                     | Source                                                                           |
| --------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Per event: `contract_id`, `ledger_sequence`, `created_at` | DB → `soroban_events_appearances` joined to `soroban_contracts`                  |
| `event_type`, `topics`, `data`                            | Archive → filter parsed envelope's `events[]` by `(transaction_id, contract_id)` |
| `diagnostic_events[]`                                     | Archive → from `result_meta_xdr` (if exposed)                                    |

**Step 7 — DB invocations (F) + Archive overlay:**

| Field                                                                                       | Source                                                                                           |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Per invocation: `contract_id`, `caller_account` (G-StrKey), `ledger_sequence`, `created_at` | DB → `soroban_invocations_appearances`                                                           |
| `function_name`, `args`, `return_value`                                                     | Archive → decoded invocation entry from `envelope_xdr` / `result_meta_xdr`                       |
| `invocation_index`, `parent_invocation_id` (call hierarchy for Normal-mode tree)            | Archive → reconstructed by walking the invocation tree in the parsed XDR; not pre-computed in DB |

### 04. `GET /ledgers`

**Source:** DB-only.

**Final response (per row):**

| Field               | Source                             |
| ------------------- | ---------------------------------- |
| `sequence`          | DB → `ledgers.sequence`            |
| `hash`              | DB → `encode(ledgers.hash, 'hex')` |
| `closed_at`         | DB → `ledgers.closed_at`           |
| `protocol_version`  | DB → `ledgers.protocol_version`    |
| `transaction_count` | DB → `ledgers.transaction_count`   |
| `base_fee`          | DB → `ledgers.base_fee`            |

### 05. `GET /ledgers/:sequence`

**Source:** DB header + **S3 per-ledger blob** (`s3://<bucket>/parsed_ledger_{N}.json`). This is the documented exception case where a list-like field on a detail endpoint is fully S3-served.

**Step 1 — DB header:**

| Field                                                                                | Source                                                                                                |
| ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| `sequence`, `hash`, `closed_at`, `protocol_version`, `transaction_count`, `base_fee` | DB → `ledgers` row                                                                                    |
| `prev_sequence`, `next_sequence`                                                     | DB → two LATERALs over `idx_ledgers_closed_at`                                                        |
| `ledger_sequence_s3_bridge`                                                          | DB → equal to `sequence`; explicit alias telling the API which value to plug into the S3 key template |

**Step 2 — S3 overlay:**

The API constructs the key `s3://<bucket>/parsed_ledger_{ledger_sequence_s3_bridge}.json` and fetches the blob (per-ledger layout from ADR 0011 / ADR 0037 §11). From that blob, the API extracts:

| Field            | Source                                                                                                                                                                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `transactions[]` | S3 → the parsed-ledger JSON's transactions array, each with at minimum `hash`, `application_order`, `source_account`, `fee_charged`, `successful`, `operation_count`, `has_soroban`, plus whatever the parsed-ledger schema carries for embedded display |

The SQL does **not** query the `transactions` partition for this list. The S3 blob is the single source of truth for the embedded sublist.

### 06. `GET /accounts/:account_id`

**Source:** DB-only.

**Step 1 — Account header (statement A):**

| Field                                                                                              | Source              |
| -------------------------------------------------------------------------------------------------- | ------------------- |
| `account_id` (G-StrKey), `sequence_number`, `first_seen_ledger`, `last_seen_ledger`, `home_domain` | DB → `accounts` row |

**Step 2 — Current balances (statement B):**

| Field                                                                                                                                        | Source                                                                           |
| -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Per balance: `asset_type_name`, `asset_type` (SMALLINT), `asset_code`, `asset_issuer` (G-StrKey), `balance` (NUMERIC), `last_updated_ledger` | DB → `account_balances_current` rows + LEFT JOIN to `accounts` for issuer StrKey |

Native XLM balance is the row with `asset_type = 0` and NULL `asset_code` / `asset_issuer`. Credit-asset balances have all three set.

### 07. `GET /accounts/:account_id/transactions`

**Source:** DB-only.

**Final response (per row):**

| Field                                                                                                                                                              | Source                                    |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------- |
| `hash`, `ledger_sequence`, `application_order`, `source_account`, `fee_charged`, `successful`, `operation_count`, `has_soroban`, `operation_types[]`, `created_at` | DB → same shape as E2 (transactions list) |

The driver is `transaction_participants` (which per ADR 0020 / task 0163 includes the source account too, so no UNION is needed). `operation_types[]` is built from the same LATERAL pattern as E2.

### 08. `GET /assets`

**Source:** DB-only.

**Final response (per row):**

| Field                           | Source                                                                   |
| ------------------------------- | ------------------------------------------------------------------------ |
| `id`                            | DB → `assets.id` (surrogate SERIAL)                                      |
| `asset_type_name`, `asset_type` | DB → `token_asset_type_name(asset_type)` + raw SMALLINT                  |
| `asset_code`                    | DB → `assets.asset_code` (NULL for native and soroban-native)            |
| `issuer`                        | DB → `accounts.account_id` (G-StrKey) via LEFT JOIN                      |
| `contract_id`                   | DB → `soroban_contracts.contract_id` (C-StrKey) via LEFT JOIN            |
| `name`                          | DB → `assets.name`                                                       |
| `total_supply`                  | DB → `assets.total_supply` (NUMERIC(28,7))                               |
| `holder_count`                  | DB → `assets.holder_count` — **may be NULL/stale until task 0135 lands** |
| `icon_url`                      | DB → `assets.icon_url` (list-thumbnail column)                           |

### 09. `GET /assets/:id`

**Source:** DB header + **S3 per-entity blob** (`s3://<bucket>/assets/{id}.json`) per ADR 0037 §11 / task 0164.

**Step 1 — DB row:**

| Field                                                                                                                                                                 | Source                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `id`, `asset_type_name`, `asset_type`, `asset_code`, `issuer`, `contract_id`, `name`, `total_supply`, `holder_count`, `icon_url`, `deployed_at_ledger` (Soroban only) | DB → same shape as E8 + `soroban_contracts.deployed_at_ledger` for SAC / soroban-native |

**Step 2 — S3 overlay:**

The API constructs the key `s3://<bucket>/assets/{id}.json` (`{id}` = the surrogate from step 1) and fetches the blob (off-chain SEP-1 enrichment, not derived from XDR; missing blob renders the two fields as blank per the §6.9 "tolerate partial availability" expectation).

| Field         | Source                               |
| ------------- | ------------------------------------ |
| `description` | S3 → SEP-1-derived asset description |
| `home_page`   | S3 → SEP-1-derived issuer home page  |

### 10. `GET /assets/:id/transactions`

**Source:** DB-only.

**Final response (per row):**

| Field                                                                                                                                         | Source                     |
| --------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| `hash`, `ledger_sequence`, `source_account`, `fee_charged`, `successful`, `operation_count`, `has_soroban`, `operation_types[]`, `created_at` | DB → same shape as E2 / E7 |

Two variants in the file: classic identity path (uses `idx_ops_app_asset` on `(asset_code, asset_issuer_id)`) and contract identity path (uses `idx_ops_app_contract`). SAC assets are reachable by both; the API merges and dedupes on `transaction_id`. Native XLM has no canonical row-level filter — the API may return empty or fall back per UX.

### 11. `GET /contracts/:contract_id`

**Source:** DB-only.

**Step 1 — Contract header (statement A):**

| Field                                                                                                                                                                                            | Source                                                        |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------- |
| `contract_pk`                                                                                                                                                                                    | DB → `soroban_contracts.id` (used to thread into statement B) |
| `contract_id` (C-StrKey), `wasm_hash`, `wasm_uploaded_at_ledger`, `deployer` (G-StrKey via LEFT JOIN), `deployed_at_ledger`, `contract_type_name`, `contract_type`, `is_sac`, `metadata` (JSONB) | DB → `soroban_contracts` row + joins                          |

**Step 2 — Recent-window stats (statement B):**

| Field                   | Source                                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ |
| `recent_invocations`    | DB → `COUNT(*)` from `soroban_invocations_appearances` bounded by `created_at >= NOW() - $stats_window`      |
| `recent_unique_callers` | DB → `COUNT(DISTINCT caller_id)` over the same window                                                        |
| `stats_window`          | DB → echoes the chosen interval so the frontend can label the stats accurately ("invocations (last 7 days)") |

The window is configurable; the API picks a default (recommended: 7 days) and is responsible for telling the frontend what window was used. The numbers are deliberately NOT full-history.

### 12. `GET /contracts/:contract_id/interface`

**Source:** DB-only.

**Final response:**

| Field                        | Source                                                      |
| ---------------------------- | ----------------------------------------------------------- |
| `contract_id` (C-StrKey)     | DB → `soroban_contracts.contract_id`                        |
| `wasm_hash`                  | DB → `encode(soroban_contracts.wasm_hash, 'hex')`           |
| `interface_metadata` (JSONB) | DB → `wasm_interface_metadata.metadata`, projected verbatim |

The frontend renders the function list from the JSONB. The JSONB shape itself is set by the indexer at ingest time (parsed from the WASM custom section) and is currently NOT documented in `docs/architecture/**` — that's a follow-up. SAC contracts have NULL `wasm_hash` and therefore NULL `interface_metadata`; the API translates that to "no interface declared" or a synthesized SAC stub.

### 13. `GET /contracts/:contract_id/invocations`

**Source:** DB index + **per-row Archive XDR overlay** (ADR 0034).

**Step 1 — DB appearance index:**

| Field                                                                                                                                           | Source                                                                                        |
| ----------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Per row: `transaction_hash`, `ledger_sequence`, `caller_account` (G-StrKey via LEFT JOIN), `amount`, `created_at`, `successful`, `cursor_tx_id` | DB → `soroban_invocations_appearances` joined to `transactions` (composite FK) and `accounts` |

**Step 2 — Archive overlay (per row):**

Bridge: `(transaction_id, created_at, contract_id)`. The API fetches the transaction's `.xdr.zst` slice (one fetch per ledger; rows in the same ledger share it) and extracts:

| Field           | Source                                             |
| --------------- | -------------------------------------------------- |
| `function_name` | Archive → invocation entry in the parsed envelope  |
| `args`          | Archive → invocation arguments                     |
| `return_value`  | Archive → invocation return from `result_meta_xdr` |

### 14. `GET /contracts/:contract_id/events`

**Source:** DB index + **per-row Archive XDR overlay** (ADR 0033, ADR 0029).

**Step 1 — DB appearance index:**

| Field                                                                                                  | Source                                                     |
| ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------- |
| Per row: `ledger_sequence`, `transaction_id`, `transaction_hash`, `successful`, `amount`, `created_at` | DB → `soroban_events_appearances` joined to `transactions` |

**Step 2 — Archive overlay (per row):**

Bridge: `(ledger_sequence, transaction_id, created_at)` identifies the transaction's `.xdr.zst` slice; `contract_id` (the route parameter) selects which entries to keep from the parsed `events[]`.

| Field        | Source                                       |
| ------------ | -------------------------------------------- |
| `event_type` | Archive → event entry from `result_meta_xdr` |
| `topics[]`   | Archive → event topics                       |
| `data`       | Archive → event payload                      |

### 15. `GET /nfts`

**Source:** DB-only.

**Final response (per row):**

| Field                                                                              | Source                                                                    |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `id`                                                                               | DB → `nfts.id` (surrogate SERIAL — used by frontend to route `/nfts/:id`) |
| `contract_id` (C-StrKey)                                                           | DB → `soroban_contracts.contract_id` via FK join                          |
| `token_id`, `collection_name`, `name`, `media_url`, `metadata`, `minted_at_ledger` | DB → `nfts` row                                                           |
| `current_owner` (G-StrKey)                                                         | DB → `accounts.account_id` via LEFT JOIN (NULL on burned NFT)             |
| `current_owner_ledger`                                                             | DB → `nfts.current_owner_ledger`                                          |

### 16. `GET /nfts/:id`

**Source:** DB-only (today). If a per-NFT S3 enrichment layout lands later (parallel to `assets/{id}.json`), this endpoint will pick up an overlay step.

**Final response:**

| Field                                                                                                                                                    | Source                  |
| -------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| `id`, `contract_id`, `token_id`, `collection_name`, `name`, `media_url`, `metadata` (JSONB), `minted_at_ledger`, `current_owner`, `current_owner_ledger` | DB → `nfts` row + joins |

The `metadata` JSONB is contract-defined at mint time. Its shape is NOT standardized in `docs/architecture/**`; the frontend either implements a defensive walker or follows a documented per-collection convention. This is a known schema-doc follow-up, not a SQL gap.

### 17. `GET /nfts/:id/transfers`

**Source:** DB-only.

**Final response (per row):**

| Field                                          | Source                                                                                                                                                                                       |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `created_at`, `ledger_sequence`, `event_order` | DB → `nft_ownership` PK columns                                                                                                                                                              |
| `event_type_name`, `event_type`                | DB → `nft_event_type_name(event_type)` + raw SMALLINT (mint / transfer / burn)                                                                                                               |
| `from_owner` (G-StrKey)                        | DB → `LAG(owner) OVER (PARTITION BY nft_id ORDER BY created_at DESC, ledger_sequence DESC, event_order DESC)` — synthesized in SQL because `nft_ownership` stores only the new owner per row |
| `to_owner` (G-StrKey)                          | DB → `accounts.account_id` via LEFT JOIN (NULL on burn)                                                                                                                                      |
| `transaction_hash`                             | DB → `encode(transactions.hash, 'hex')` via composite-FK join                                                                                                                                |

Pagination caveat: `LAG()` resets at every page boundary. The API stitches this by passing the previous page's last `to_owner` back into the new page's first row as `from_owner` — see file 17 header for the contract.

### 18. `GET /liquidity-pools`

**Source:** DB-only.

**Final response (per row):**

| Field                                                                                                                    | Source                                                                                                                                                                                                |
| ------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pool_id`                                                                                                                | DB → `encode(liquidity_pools.pool_id, 'hex')` (32-byte natural PK)                                                                                                                                    |
| `asset_a_type_name`, `asset_a_type`, `asset_a_code`, `asset_a_issuer`                                                    | DB → `liquidity_pools` + LEFT JOIN to `accounts` for issuer StrKey                                                                                                                                    |
| `asset_b_type_name`, `asset_b_type`, `asset_b_code`, `asset_b_issuer`                                                    | DB → same for the B leg                                                                                                                                                                               |
| `fee_bps`, `fee_percent`                                                                                                 | DB → raw basis points + derived percentage (`fee_bps / 100`)                                                                                                                                          |
| `created_at_ledger`                                                                                                      | DB → pool creation ledger                                                                                                                                                                             |
| `latest_snapshot_ledger`, `reserve_a`, `reserve_b`, `total_shares`, `tvl`, `volume`, `fee_revenue`, `latest_snapshot_at` | DB → latest row from `liquidity_pool_snapshots` via LATERAL `LIMIT 1`; clients read `latest_snapshot_at` to judge freshness. `tvl`/`volume`/`fee_revenue` are NULL today (future TVL-ingestion task). |

### 19. `GET /liquidity-pools/:id`

**Source:** DB-only.

**Final response:**

| Field                                                             | Source                                                                                               |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Same shape as a single E18 row (header + latest-snapshot lateral) | DB → `liquidity_pools` + LATERAL into `liquidity_pool_snapshots` + accounts joins for issuer StrKeys |

### 20. `GET /liquidity-pools/:id/transactions`

**Source:** DB-only.

**Final response (per row):**

| Field                                                                                                                                         | Source                                                                                                                                                                                                                        |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hash`, `ledger_sequence`, `source_account`, `fee_charged`, `successful`, `operation_count`, `has_soroban`, `operation_types[]`, `created_at` | DB → driven from `operations_appearances WHERE pool_id = $1` (uses `idx_ops_app_pool`), joined to `transactions` and `accounts`. `operation_types[]` lets the frontend distinguish trade vs LP-management activity per §6.14. |

### 21. `GET /liquidity-pools/:id/chart`

**Source:** DB-only.

**Final response (per bucket):**

| Field               | Source                                                                                                                          |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `bucket`            | DB → `date_trunc($interval, created_at)` over `[from, to]`                                                                      |
| `tvl`               | DB → LAST tvl in the bucket via `array_agg(... ORDER BY created_at DESC, ledger_sequence DESC)[1]` (TVL is a state, not a flow) |
| `volume`            | DB → `SUM(volume)` over the bucket (flow)                                                                                       |
| `fee_revenue`       | DB → `SUM(fee_revenue)` over the bucket (flow)                                                                                  |
| `samples_in_bucket` | DB → `COUNT(*)` of snapshots that landed in the bucket; useful debug metadata for the frontend                                  |

### 22. `GET /search`

**Source:** DB-only.

**Final response (per row):**

| Field          | Source                                                                                                                                                                                          |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `entity_type`  | DB → one of `transaction` / `contract` / `asset` / `account` / `nft` / `pool`                                                                                                                   |
| `identifier`   | DB → display text (hex hash for tx/pool, StrKey for contract/account, asset code or `XLM` for asset, NFT name). NOT a unique key for assets and NFTs — frontend uses `surrogate_id` for routing |
| `label`        | DB → entity-specific brief context (ledger #, contract metadata name, asset type, home_domain, collection name, asset pair)                                                                     |
| `surrogate_id` | DB → routing key for assets / NFTs / contracts / accounts; NULL for entities routed by their natural key (tx hash, pool_id)                                                                     |

API layer is responsible for: shape classification of the input (hash vs StrKey vs free text), exact-match redirect when a single CTE returns one row equal to the input, and union-merging across the per-type buckets.

### 23. `GET /liquidity-pools/:id/participants`

**Source:** DB-only.

**Final response (per row):**

| Field                  | Source                                                                                            |
| ---------------------- | ------------------------------------------------------------------------------------------------- |
| `account` (G-StrKey)   | DB → `accounts.account_id` via FK join                                                            |
| `shares`               | DB → `lp_positions.shares` (NUMERIC(28,7))                                                        |
| `share_percentage`     | DB → `(shares * 100) / latest_snapshot.total_shares`, NULL if no snapshot in the freshness window |
| `first_deposit_ledger` | DB → `lp_positions.first_deposit_ledger`                                                          |
| `last_updated_ledger`  | DB → `lp_positions.last_updated_ledger`                                                           |

Powers the §6.14 "Pool participants" table; sorted by share size DESC.
