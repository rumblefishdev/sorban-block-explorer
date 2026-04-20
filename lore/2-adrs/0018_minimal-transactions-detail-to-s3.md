---
id: '0018'
title: 'Minimal transactions and operations tables; token_transfers removed — detail fields offloaded to S3'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0013', '0014', '0015', '0016', '0017']
tags:
  [
    database,
    schema,
    transactions,
    operations,
    token-transfers,
    soroban-events,
    s3,
    minimalism,
    muxed,
    fee-bump,
  ]
links: []
history:
  - date: 2026-04-20
    status: proposed
    who: fmazur
    note: 'ADR created — tightens transactions table to 12 columns after field-by-field review'
  - date: 2026-04-20
    status: proposed
    who: fmazur
    note: 'Extended to cover operations table — 15 → 12 columns (4 removed: source_account, source_account_muxed, destination_muxed, function_name; 1 added: transfer_amount)'
  - date: 2026-04-20
    status: proposed
    who: fmazur
    note: 'Extended to remove token_transfers entirely; 3 columns added to soroban_events (transfer_from, transfer_to, transfer_amount) with partial indexes; projected net saving ~420–920 GB at mainnet'
---

# ADR 0018: Minimal `transactions` and `operations` tables; `token_transfers` removed — detail fields offloaded to S3

**Related:**

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0017: Ingest guard clarification, topic0 validation, final schema](0017_ingest-guard-clarification-topic0-validation-final-schema.md)

---

## Status

`proposed` — corrective delta on top of ADR 0017, scoped to three changes:
the `transactions` table (12 columns), the `operations` table (12 columns),
and the removal of the `token_transfers` table entirely (replaced by three
narrow columns on `soroban_events`). Every other table decision from
ADR 0011–0017 stands.

---

## Context

Field-by-field review of `transactions` in ADR 0017 surfaced eight columns that
are either redundant, used only in advanced detail view, or reconstructible from
other sources. Removing them reduces the table from **20 columns to 12**, tightens
the boundary between "DB = list/filter index" and "S3 = detail payload", and
aligns the schema with the Etherscan/StellarChain list column sets
(hash / method / ledger / age / from / to / amount / fee).

The eight columns fall into three groups:

1. **Redundant** — `is_fee_bump` is fully derivable from
   `inner_tx_hash IS NOT NULL`. Keeping both duplicates the same signal.
2. **Detail-only fields not used in list or filter** — `result_code`, `memo`,
   `memo_type`, `parse_error_reason`. None appears in documented list views, no
   endpoint filter targets them, no search operator uses them. Their only role
   is serving detail-view payloads, which is exactly the S3 responsibility per
   ADR 0011.
3. **Muxed/fee-bump auxiliary fields** — `source_account_muxed`,
   `fee_account_muxed`, `fee_account`. Present only for advanced detail view
   (SEP-0023 traceability, fee-bump payer display). Account-centric queries are
   served by `transaction_participants` (already an N:M table per ADR 0012),
   which carries an explicit `role='fee_payer'` entry for fee-bump transactions.
   Advanced view already fetches `parsed_ledger_{N}.json` from S3 for
   `envelope_xdr`, `result_xdr`, `result_meta_xdr`, and `operation_tree` — the
   same fetch can surface these three fields at zero additional round-trip cost.

Backend-overview forbids server-side XDR decoding
(_"No server-side decode — the API serves pre-materialized data"_). This means
offloaded fields must be pre-decoded by the parser and written into
`parsed_ledger_{N}.json` during parse phase. Raw XDR is not a substitute for
structured JSON fields. The parser already decodes the full envelope and
already writes `parsed_ledger_{N}.json` — adding these fields to the JSON is a
mechanical extension of the existing parse phase output.

This ADR makes the offload and the parser contract explicit and normative.

---

## Decision

### Summary of decisions

- **Remove 5 columns** from `transactions` as redundant or pure-detail with no
  DB access pattern: `is_fee_bump`, `result_code`, `memo_type`, `memo`,
  `parse_error_reason`.
- **Remove 3 columns** from `transactions` (muxed + fee_account) under the
  following three parser-side normative conditions. Schema correctness for
  every documented endpoint is guaranteed **iff all three conditions hold**.
- **Keep 12 columns** — the minimal set needed for list rendering, filtering,
  partition routing, FK references, fee-bump lookup, and an operational parse
  flag.

### Columns removed — rationale per column

| Column                             | Removed because                                                                                                      | Replacement                                                     |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `is_fee_bump BOOLEAN`              | Derivable from `inner_tx_hash IS NOT NULL`. Any query that filtered on it can test the hash column instead.          | `inner_tx_hash IS NOT NULL`                                     |
| `result_code VARCHAR(30)`          | Not in any list view (explorers show only success/fail badge, not `txBAD_SEQ` etc. in list). No filter. Detail-only. | `parsed_ledger_{N}.json` field `transactions[i].result_code`    |
| `memo_type VARCHAR(8)`             | Not in any list view. No filter. Used only when rendering detail memo section.                                       | `parsed_ledger_{N}.json` field                                  |
| `memo BYTEA`                       | Not in any list view. No filter. Not searchable. Detail-only.                                                        | `parsed_ledger_{N}.json` field                                  |
| `parse_error_reason TEXT`          | Debug string. Not a user-facing field. Belongs in CloudWatch logs, not in per-row DB storage.                        | CloudWatch logs during parse phase                              |
| `source_account_muxed VARCHAR(69)` | Not in any list view. Advanced-view only (SEP-0023 traceability).                                                    | `parsed_ledger_{N}.json` field                                  |
| `fee_account_muxed VARCHAR(69)`    | Same as above.                                                                                                       | `parsed_ledger_{N}.json` field                                  |
| `fee_account VARCHAR(56)`          | Advanced-view only for fee-bump tx. Account-centric queries covered by `transaction_participants.role='fee_payer'`.  | `parsed_ledger_{N}.json` field + `transaction_participants` row |

### Columns kept — 12 total

```sql
CREATE TABLE transactions (
    id                BIGSERIAL,
    hash              VARCHAR(64) NOT NULL,
    ledger_sequence   BIGINT NOT NULL,
    application_order SMALLINT NOT NULL,
    source_account    VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    fee_charged       BIGINT NOT NULL,
    inner_tx_hash     VARCHAR(64),               -- NULL = not fee-bump
    successful        BOOLEAN NOT NULL,
    operation_count   SMALLINT NOT NULL,
    has_soroban       BOOLEAN NOT NULL DEFAULT FALSE,
    parse_error       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (hash, created_at)
) PARTITION BY RANGE (created_at);
```

| Column              | Why it stays                                                                                      |
| ------------------- | ------------------------------------------------------------------------------------------------- |
| `id`                | Surrogate PK, target for composite FK from children                                               |
| `hash`              | Detail lookup, list column, search                                                                |
| `ledger_sequence`   | List column (Ledger), bridge to S3 file, `/ledgers/:sequence` query, partition-independent filter |
| `application_order` | Stable ordering in `/ledgers/:sequence` transaction list                                          |
| `source_account`    | List column (From), `filter[source_account]`, search                                              |
| `fee_charged`       | List column (Fee)                                                                                 |
| `inner_tx_hash`     | Fee-bump inner tx lookup; replaces `is_fee_bump`                                                  |
| `successful`        | List column (Status badge), summary aggregation                                                   |
| `operation_count`   | List badge ("N recipients"), denormalized list speed                                              |
| `has_soroban`       | Filter tab "Contracts" (StellarChain pattern), denormalized for speed                             |
| `parse_error`       | Operational flag — 1-bit signal for "this row may have degraded fields"                           |
| `created_at`        | Partition key, list column (Age), default sort                                                    |

Constraint `ck_tx_memo_type` is dropped together with `memo_type`.

### Normative parser conditions (all three required)

These conditions are **not optional** and **not recommendations**. Removing the
eight columns is safe **if and only if** the parser satisfies all three. Any
deviation breaks a documented endpoint.

#### Condition 1 — Parser writes detail fields to `parsed_ledger_{N}.json`

For every transaction processed, the parser emits the following fields into
`parsed_ledger_{N}.json` under `transactions[i]`:

```json
{
  "hash": "...",
  "source_account": "GABC...",
  "source_account_muxed": "MABC...",    // null when no mux
  "fee_account": "GDEF...",             // null when not fee-bump
  "fee_account_muxed": "MDEF...",       // null when no mux
  "result_code": "txSUCCESS",           // always populated
  "memo_type": "text|id|hash|return|none",
  "memo": "...",                        // base64 for binary, decimal for id, hex for hash/return, null for none
  "envelope_xdr": "...",
  "result_xdr": "...",
  "result_meta_xdr": "...",
  "operation_tree": [...]
}
```

Rationale: the parser already decodes the full envelope — these fields are
already in memory at parse time. Emitting them to JSON is mechanical, not
analytical. The backend advanced-view reader fetches the same
`parsed_ledger_{N}.json` file it already fetches for `envelope_xdr` and
`operation_tree`, and reads these additional fields from the same JSON
document. **Zero additional S3 fetches.**

#### Condition 2 — Parser populates `transaction_participants.role='fee_payer'`

For every fee-bump transaction, the parser writes **two** rows into
`transaction_participants`:

- `(transaction_id, source_account, role='source')` — inner tx source
- `(transaction_id, fee_account, role='fee_payer')` — outer fee payer

This ensures that `GET /accounts/:id/transactions` returns fee-bump
transactions when the queried account is the fee payer, even though
`transactions.fee_account` no longer exists. The role enum value `'fee_payer'`
is already part of the N:M role taxonomy per ADR 0012.

For non-fee-bump transactions, only the `source` role is written (current
behavior). No change.

#### Condition 3 — Advanced detail view reads from S3 JSON

`GET /transactions/:hash` in advanced mode, when it needs to render:

- original muxed source/fee address,
- fee payer,
- result code,
- memo,

must read those fields from the `parsed_ledger_{N}.json` document it already
fetches for `envelope_xdr` / `operation_tree`. No server-side XDR decoding
(per backend-overview's existing constraint). No extra S3 GET request —
the same file is parsed once in Lambda memory and all fields extracted from
it.

---

## Detailed schema changes

### DDL delta from ADR 0017

```sql
-- Drop the dependent CHECK first:
ALTER TABLE transactions DROP CONSTRAINT IF EXISTS ck_tx_memo_type;

-- Drop columns:
ALTER TABLE transactions DROP COLUMN is_fee_bump;
ALTER TABLE transactions DROP COLUMN result_code;
ALTER TABLE transactions DROP COLUMN memo_type;
ALTER TABLE transactions DROP COLUMN memo;
ALTER TABLE transactions DROP COLUMN parse_error_reason;
ALTER TABLE transactions DROP COLUMN source_account_muxed;
ALTER TABLE transactions DROP COLUMN fee_account_muxed;
ALTER TABLE transactions DROP COLUMN fee_account;
```

Each `DROP COLUMN` is a metadata-only operation on a table storing narrow,
mostly-NULL optional columns — fast, no rewrite. The partition structure is
untouched.

### Index graph after change

All indexes from ADR 0017 stay **except** one which no longer has a backing
column to filter on is NOT changed — this ADR does not remove any index.
Specifically, these remain valid and useful:

```sql
CREATE INDEX idx_tx_hash           ON transactions (hash);
CREATE INDEX idx_tx_hash_prefix    ON transactions (hash text_pattern_ops);
CREATE INDEX idx_tx_source_created ON transactions (source_account, created_at DESC);
CREATE INDEX idx_tx_ledger         ON transactions (ledger_sequence, application_order);
CREATE INDEX idx_tx_created        ON transactions (created_at DESC);
CREATE INDEX idx_tx_has_soroban    ON transactions (created_at DESC) WHERE has_soroban;
CREATE INDEX idx_tx_inner_hash     ON transactions (inner_tx_hash) WHERE inner_tx_hash IS NOT NULL;
```

No index drops. No new indexes. Seven indexes total.

### FK graph

Unchanged. `transactions.source_account → accounts(account_id)` retained.
Child tables (`operations`, `soroban_events`, `soroban_invocations`,
`transaction_participants`, `token_transfers`, `nft_ownership`) keep their
composite FK `(transaction_id, created_at) → transactions(id, created_at)`.

### Other tables touched by this ADR

Three tables affected:

- `transactions` (this section) — 20 → 12 columns.
- `operations` (see **Operations table changes** below) — 15 → 12 columns.
- `soroban_events` (see **Token transfers removal** below) — +3 columns
  (transfer_from, transfer_to, transfer_amount) + 2 partial indexes.
- `token_transfers` (see **Token transfers removal** below) — **dropped
  entirely**.

`accounts`, `transaction_participants`, `soroban_invocations`, `tokens`,
`nfts`, `nft_ownership`, `liquidity_pools`, `liquidity_pool_snapshots`,
`lp_positions`, `account_balances_current`, `account_balance_history`,
`transaction_hash_index`, `soroban_contracts`, `wasm_interface_metadata`,
`ledgers` — all untouched.

---

## Per-endpoint verification

| Endpoint                                                       | Columns used from `transactions`                                                                                                | Safe after change?        |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| `GET /network/stats`                                           | COUNT(\*)                                                                                                                       | ✓                         |
| `GET /transactions` (list, no filter)                          | hash, ledger_sequence, source_account, successful, fee_charged, operation_count, has_soroban, created_at                        | ✓                         |
| `GET /transactions` `filter[source_account]`                   | source_account + idx_tx_source_created                                                                                          | ✓                         |
| `GET /transactions` `filter[contract_id]`                      | JOIN operations, unchanged                                                                                                      | ✓                         |
| `GET /transactions` `filter[operation_type]`                   | JOIN operations, unchanged                                                                                                      | ✓                         |
| `GET /transactions/:hash` normal                               | hash, ledger_sequence, source_account, successful, fee_charged, created_at + S3 JSON for memo, operation_tree                   | ✓ via S3                  |
| `GET /transactions/:hash` advanced                             | Same + S3 JSON for envelope_xdr, result_xdr, result_meta_xdr, result_code, source_account_muxed, fee_account, fee_account_muxed | ✓ via S3 (Condition 1, 3) |
| `GET /ledgers`                                                 | —                                                                                                                               | ✓                         |
| `GET /ledgers/:sequence`                                       | ledger_sequence, application_order, hash, source_account, successful, fee_charged, created_at via idx_tx_ledger                 | ✓                         |
| `GET /accounts/:id`                                            | —                                                                                                                               | ✓                         |
| `GET /accounts/:id/transactions` including fee-bump payer role | `transaction_participants` (PK prefix account_id) + JOIN `transactions` (hash, successful, fee_charged, created_at)             | ✓ via Condition 2         |
| `GET /tokens` + variants                                       | —                                                                                                                               | ✓                         |
| `GET /contracts/:id` + variants                                | —                                                                                                                               | ✓                         |
| `GET /nfts` + variants                                         | —                                                                                                                               | ✓                         |
| `GET /liquidity-pools` + variants                              | —                                                                                                                               | ✓                         |
| `GET /search`                                                  | hash, idx_tx_hash_prefix                                                                                                        | ✓                         |

**All 20+ documented endpoints continue to work.** Advanced detail view relies
on the parser/S3 path that ADR 0011 already established; this ADR only adds
four more fields to the same S3 JSON document.

---

## Rationale

### Why these eight, not more and not fewer

The cut is drawn at the exact line where **list view + filter + FK + partition
key** ends and **detail view** begins. Every retained column has a documented
consumer in a list endpoint, filter parameter, or structural role (PK,
partition key, FK target, denormalized speed column). Every removed column is
either redundant with another column or used only in detail.

Not removed: `parse_error BOOLEAN`. One bit per row. Signals "this row's
fields may be degraded". Useful for operator dashboards. Keeping it costs
nothing; removing it sacrifices a small operational signal.

Not removed: `operation_count SMALLINT` and `has_soroban BOOLEAN`. Both are
denormalizations from child tables, but both drive list-view queries that
would otherwise require JOIN + COUNT per row. Denormalization is justified by
measurable list performance.

### Why moving to S3 is not a regression

Three reasons:

1. **Advanced detail view already fetches S3.** `envelope_xdr`, `result_xdr`,
   `result_meta_xdr`, `operation_tree` — all in S3 per ADR 0011. Adding four
   more fields to the same JSON document is zero additional round-trip.
2. **Backend-overview bans XDR decode in API.** Raw XDR is not a legitimate
   substitute for a structured field. Detail fields must live either as
   pre-decoded DB columns **or** as pre-decoded JSON on S3. The parser already
   produces parsed JSON; extending it is cheap.
3. **Normal view already triggers S3 fetch for `operation_tree`** per ADR 0011. Memo rendering in normal view reads from the same JSON. Zero round-
   trip change.

### Why parser-side conditions are a fair tradeoff

Schema is simpler. Write amplification drops (fewer columns per INSERT).
Parser output grows by ~100 bytes per transaction in the S3 JSON (less than
a percent of the JSON size). The parser already has every dropped field in
memory after envelope decode; emitting it to JSON is mechanical.

The cost shifts from **permanent DB storage overhead** (paid forever on every
row) to **one-time parser extension** (paid once at implementation). The
tradeoff favors minimalism.

### Why `transaction_participants.role='fee_payer'` is the correct replacement for `fee_account`

`/accounts/:id/transactions` queries the N:M participant table. If account X
is a fee payer in transaction T, the query must return T. Pre-ADR-0018 this
worked via `transactions.fee_account = X`. Post-ADR-0018 it works via
`transaction_participants WHERE account_id = X AND role = 'fee_payer'`.

Both paths deliver the same result set. The N:M approach is strictly more
general (covers source, destination, signer, caller, fee_payer, counter in
one index), which is exactly why the table exists. Consolidating fee-bump
into the same table removes a special case in the backend query layer.

---

## Consequences

### Stellar/XDR compliance

- **Neutral.** Muxed addresses (SEP-0023) are still preserved byte-for-byte in
  the parsed JSON on S3. Fee-bump structure (SEP-0028) is still fully
  representable in the parsed JSON. `result_code` enumeration is still
  accessible per transaction via S3. No protocol concept loses representation
  in the system — it moves from DB columns to parsed JSON.

### Database weight

- **Reduced.** Eight columns removed from a 300M-row table saves roughly
  15–25 GB plus corresponding partition metadata and per-partition overhead.
  Small in absolute terms; meaningful in principle — DB now holds strictly
  list/filter/FK state.

### History correctness

- **Unchanged.** All historical reconstruction paths from ADR 0017 remain
  valid. Historical transactions written before this ADR keep the columns;
  after `DROP COLUMN`, those values are discarded (the parsed JSON on S3
  retains them by design per ADR 0011). For any historical transaction
  needing advanced-view detail, S3 JSON is the source of truth — which is
  exactly where ADR 0011 placed it.

### Endpoint performance

- **Unchanged for list endpoints.** Retained columns are exactly those that
  back list indexes.
- **Unchanged for detail endpoints.** Already fetching S3; adding more fields
  to the parsed JSON read is free.
- **`/accounts/:id/transactions`** continues using `transaction_participants`
  PK prefix scan — no change.

### Ingest simplicity

- **Parser gains ~10 lines.** Extend the JSON output of parse phase with the
  four additional fields, and extend the `transaction_participants` insert to
  include `('fee_payer', fee_account)` for fee-bump transactions. Both
  changes are mechanical given that the parser already has all data in memory
  after envelope decode.
- **INSERT to `transactions`** shrinks by 8 columns. Fewer bytes per commit.

### Replay / re-ingest risk

- **Unchanged.** Re-ingest protocol (DELETE cascade + re-INSERT per ledger
  group) is identical. Retention, monitoring, partition management untouched.

### Operational cost

- **Marginally reduced.** Smaller table, smaller WAL per transaction,
  slightly faster VACUUM. Nothing qualitative.

### Risk: what breaks if parser conditions are not met

Explicit failure modes if any of the three conditions are violated:

- **Condition 1 violated (detail fields missing from S3 JSON):** advanced
  view renders partial data — user sees missing memo, missing result_code,
  missing M-form. This is the same failure surface as any parser bug and is
  caught by contract tests on the parsed JSON output.
- **Condition 2 violated (no `fee_payer` rows written):** fee-bump
  transactions don't appear in `/accounts/:id/transactions` when queried
  account is only fee payer. Silent miss. Catchable by integration test:
  ingest a fee-bump tx, query the fee payer's transaction list, expect the
  tx to appear.
- **Condition 3 violated (API tries to decode XDR):** backend-overview is
  violated. Caught in code review.

Each failure is testable with a unit- or integration-level test against the
parser and API layer. None is a schema-level failure.

---

## Migration / rollout notes

Applies to environments where ADR 0017 is deployed. Greenfield deployments
adopt ADR 0017 + ADR 0018 together.

1. **Parser update (deploy first):**
   - Extend `parsed_ledger_{N}.json` emission to include `result_code`,
     `memo_type`, `memo`, `source_account_muxed`, `fee_account`,
     `fee_account_muxed` under `transactions[i]`.
   - For fee-bump transactions, add a second row to
     `transaction_participants` with `role='fee_payer'` and
     `account_id = fee_account_G_form`.
   - Deploy parser to staging, run synthetic fee-bump ingest, validate:
     - parsed JSON includes all six fields
     - `transaction_participants` contains both `source` and `fee_payer`
       rows for fee-bump tx
2. **API update (deploy second):**
   - Advanced view reads `result_code`, `memo`, `memo_type`,
     `source_account_muxed`, `fee_account`, `fee_account_muxed` from
     `parsed_ledger_{N}.json` (already fetched for `envelope_xdr` /
     `operation_tree`). Zero extra S3 requests.
   - Remove any code path that reads these fields from `transactions`.
3. **Schema change (deploy third, last):**
   - Run `ALTER TABLE transactions DROP COLUMN ...` × 8 and
     `DROP CONSTRAINT ck_tx_memo_type`. Fast metadata-only operations.
4. **Verify:**
   - Run `\d transactions` — expect 12 columns.
   - Run representative queries for each endpoint from the per-endpoint
     verification table above.
   - Validate advanced view of a known fee-bump transaction shows
     `fee_account` and `fee_account_muxed`.
   - Validate `/accounts/:fee_payer_G/transactions` returns the fee-bump tx.

Rollback: add the columns back via `ALTER TABLE ADD COLUMN`. Data is lost,
but `parsed_ledger_{N}.json` on S3 is the source of truth — a backfill
script can re-populate DB columns from S3 if truly needed. In practice
rollback should not be necessary; the parser changes are compatible with
the pre-ADR-0018 schema (parser would just populate both DB columns and
S3 JSON redundantly for a transition period, if desired).

---

## Operations table changes

This section extends the ADR to the `operations` table. Same principle as for
`transactions`: keep in DB only what list views, filters, partition routing,
and FK references actually need. Offload advanced-detail-only fields to
`parsed_ledger_{N}.json` on S3. Add one list-visible field
(`transfer_amount`) so that the Etherscan/StellarChain "Amount" list column
is served without extra S3 fetches.

Result: `operations` shrinks from **15 columns (ADR 0017) to 12 columns**.

### Context specific to `operations`

Field-by-field review against the documented access patterns:

- `operations` has **no child tables** — no FK points at it.
- **No API endpoint** filters `operations.source_account` and ADR 0017 never
  indexed it. Account-centric queries route through `transaction_participants`
  by design (ADR 0012).
- **No API endpoint** filters `operations.function_name`. The contract-centric
  endpoints (`/contracts/:id/invocations`, `/contracts/:id/events`) read from
  dedicated tables `soroban_invocations` and `soroban_events`, which already
  carry their own `function_name` where relevant.
- List view `/transactions` and `/tokens/:id/transactions`,
  `/liquidity-pools/:id/transactions` need an **Amount** column (Etherscan
  and StellarChain list pattern). ADR 0017 had no `transfer_amount` column —
  list rendering either fetched S3 per row (impractical) or omitted the
  column.
- Muxed fields `source_account_muxed`, `destination_muxed` and the
  parser-local `function_name` are only rendered in **advanced transaction
  detail view**, which already fetches `parsed_ledger_{N}.json`.

### Columns removed — rationale per column

| Column                             | Removed because                                                                                                                                                                                      | Replacement                                                                                                                             |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `source_account VARCHAR(56)`       | Not indexed, not a filter target, not used in list view (tx-level source is on `transactions`). Account-centric queries use `transaction_participants`. Advanced per-operation source is in S3 JSON. | `parsed_ledger_{N}.json` field `transactions[i].operations[j].source_account` + `transaction_participants` for op-level source override |
| `source_account_muxed VARCHAR(69)` | Advanced detail view only (SEP-0023 traceability). Not indexed, no filter.                                                                                                                           | S3 JSON field                                                                                                                           |
| `destination_muxed VARCHAR(69)`    | Advanced detail view only. Not indexed, no filter.                                                                                                                                                   | S3 JSON field                                                                                                                           |
| `function_name VARCHAR(100)`       | Redundant with `soroban_invocations.function_name`. Not a filter on `operations`. Not in list view column set.                                                                                       | S3 JSON field + `soroban_invocations.function_name` (unchanged)                                                                         |

### Column added — rationale

| Column                               | Added because                                                                                                                                                                                                                    | Role                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `transfer_amount NUMERIC(39,0)` NULL | Etherscan/StellarChain list column "Amount" requires an amount per row in `/transactions`, `/tokens/:id/transactions`, `/liquidity-pools/:id/transactions`. Without it, these list views either skip Amount or fetch S3 per row. | Populated by parser from XDR-decoded operation details. Semantics per operation type: PAYMENT → `amount`; PATH_PAYMENT → destination amount; LP_DEPOSIT/WITHDRAW → asset A amount (primary); CREATE_ACCOUNT → `starting_balance`; others → NULL. Multi-asset detail (asset B in LP, source amount in path_payment, path hops) stays in S3 JSON for detail view. |

### Index removed

- `idx_ops_destination (destination, created_at DESC) WHERE destination IS NOT NULL` — **dropped**. No API filter targets this column. Account-centric queries are served by `transaction_participants` which has `role='destination'` via parser. The column itself stays (for list view LATERAL JOIN on tx → first op `destination`), but no dedicated index is needed.

### Columns kept — 12 total

```sql
CREATE TABLE operations (
    id                BIGSERIAL,
    transaction_id    BIGINT NOT NULL,
    application_order SMALLINT NOT NULL,
    type              VARCHAR(32) NOT NULL,
    destination       VARCHAR(56) REFERENCES accounts(account_id),
    contract_id       VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    asset_code        VARCHAR(12),
    asset_issuer      VARCHAR(56) REFERENCES accounts(account_id),
    pool_id           VARCHAR(64) REFERENCES liquidity_pools(pool_id),
    transfer_amount   NUMERIC(39,0),                  -- NEW: list "Amount" column
    ledger_sequence   BIGINT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    UNIQUE (transaction_id, application_order, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at)
        ON DELETE CASCADE
) PARTITION BY RANGE (created_at);
```

| Column                       | Why it stays                                                                                           |
| ---------------------------- | ------------------------------------------------------------------------------------------------------ |
| `id`                         | Surrogate PK, stable identity (retained for API cursor compatibility; composite PK migration deferred) |
| `transaction_id`             | FK to `transactions`, composite FK target                                                              |
| `application_order`          | Stable order within transaction, list "Method" derivation                                              |
| `type`                       | `filter[operation_type]`, list "Method" column                                                         |
| `destination`                | List "To" column derivation via LATERAL join on first op                                               |
| `contract_id`                | `filter[contract_id]`, `/contracts/:id/invocations` lookup                                             |
| `asset_code`, `asset_issuer` | `/tokens/:id/transactions` (classic), list "Asset" column                                              |
| `pool_id`                    | `/liquidity-pools/:id/transactions`                                                                    |
| `transfer_amount`            | List "Amount" column (new)                                                                             |
| `ledger_sequence`            | Partition-independent filter, bridge to S3                                                             |
| `created_at`                 | Partition key, FK composite target, list ordering                                                      |

### Additional parser conditions (C4, C5)

Beyond Conditions 1–3 for `transactions`, the `operations` changes require
two more normative parser conditions.

#### Condition 4 — Parser emits per-operation detail fields to S3 JSON

For every operation, parser writes into `parsed_ledger_{N}.json` under
`transactions[i].operations[j]`:

```json
{
  "application_order": 5,
  "type": "manage_buy_offer",
  "source_account": "GABC...", // op-level source (may differ from tx source)
  "source_account_muxed": "MABC...", // null if no mux
  "destination": "GDEF...", // null when op has no destination
  "destination_muxed": "MDEF...", // null if no mux
  "function_name": "swap", // INVOKE_HOST_FUNCTION only
  "details": {
    /* full decoded per-op XDR */
  }
}
```

These are already in parser memory after XDR decode. Parser already emits
`details` JSONB (ADR 0011 design). Adding four explicit fields is a
mechanical extension of the existing emission.

#### Condition 5 — Parser writes op-level source to `transaction_participants`

For every operation `op` in transaction `tx`:

- If `op.source_account == tx.source_account` (no op-level override), no
  extra row beyond the tx-level `role='source'` row.
- If `op.source_account != tx.source_account` (op-level source override),
  parser **must** insert an additional row into `transaction_participants`
  with `(transaction_id, op.source_account_G, role='source')`. Duplicates
  across multiple ops with the same source are suppressed by the PK
  `(account_id, created_at, transaction_id, role)` via `ON CONFLICT DO
NOTHING` (already the ADR 0013 ingest pattern).

This ensures `GET /accounts/:id/transactions` returns transactions where the
queried account was an op-level source override, even though
`operations.source_account` no longer exists in DB.

ADR 0012 role taxonomy already covered this intent. This ADR makes it
explicit and normative.

### DDL delta from ADR 0017 (operations)

```sql
-- Drop unused index first:
DROP INDEX IF EXISTS idx_ops_destination;

-- Drop columns:
ALTER TABLE operations DROP COLUMN source_account;
ALTER TABLE operations DROP COLUMN source_account_muxed;
ALTER TABLE operations DROP COLUMN destination_muxed;
ALTER TABLE operations DROP COLUMN function_name;

-- Add transfer_amount:
ALTER TABLE operations ADD COLUMN transfer_amount NUMERIC(39,0);
```

`DROP COLUMN` on operations is metadata-only (Postgres doesn't rewrite the
table for nullable column drops at high volume). `ADD COLUMN ... NULL`
without default is also metadata-only. Partition structure and all child FKs
are untouched.

### Index graph after change (operations)

```sql
CREATE INDEX idx_ops_tx       ON operations (transaction_id);
CREATE INDEX idx_ops_type     ON operations (type, created_at DESC);
CREATE INDEX idx_ops_contract ON operations (contract_id, created_at DESC)
    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_ops_asset    ON operations (asset_code, asset_issuer, created_at DESC)
    WHERE asset_code IS NOT NULL;
CREATE INDEX idx_ops_pool     ON operations (pool_id, created_at DESC)
    WHERE pool_id IS NOT NULL;
```

**Five indexes** (down from six in ADR 0017). `idx_ops_destination` removed
as unused. No new indexes — `transfer_amount` is not filterable (no
`filter[min_amount]` in API surface).

### Per-endpoint verification (operations)

| Endpoint                                                                      | Columns used from `operations`                                                                     | Safe after change?                |
| ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | --------------------------------- |
| `GET /transactions` `filter[contract_id]`                                     | `contract_id` via JOIN                                                                             | ✓                                 |
| `GET /transactions` `filter[operation_type]`                                  | `type` via JOIN                                                                                    | ✓                                 |
| `GET /transactions` list "Method / To / Amount" columns (LATERAL on first op) | `type`, `destination`, `asset_code`, `asset_issuer`, `transfer_amount`                             | ✓ with added `transfer_amount`    |
| `GET /transactions/:hash` normal + advanced (per-op section)                  | DB row (type, destination, asset, amount) + S3 JSON for source, muxed, function_name, full details | ✓ via Condition 4                 |
| `GET /ledgers/:sequence` transactions list                                    | — (goes through `transactions` via `idx_tx_ledger`)                                                | ✓                                 |
| `GET /accounts/:id/transactions` including op-level source override           | `transaction_participants` (covers tx source, op-level source, destination, caller, fee_payer, …)  | ✓ via Condition 5                 |
| `GET /tokens/:id/transactions` (classic)                                      | `asset_code`, `asset_issuer`, `transfer_amount` (Amount column)                                    | ✓ with `transfer_amount`          |
| `GET /contracts/:id/invocations`                                              | Independent table `soroban_invocations`; `operations` not read                                     | ✓                                 |
| `GET /contracts/:id/events`                                                   | Independent table `soroban_events`; `operations` not read                                          | ✓                                 |
| `GET /liquidity-pools/:id/transactions`                                       | `pool_id`, `type`, `transfer_amount` (primary asset amount)                                        | ✓ (secondary amount in S3 detail) |

All endpoints remain functional. The only intentional list-view degradation:
for LP deposit/withdraw and path payment, the list shows **primary amount**
(asset A for LP, destination amount for path payment). Full multi-asset
detail is in the S3 JSON that detail view already fetches. This matches
Etherscan/StellarChain patterns (list = one value per row; detail = full
breakdown).

### Rationale specific to `operations`

**Why offload four columns.** Each removed column failed the list/filter
test: not in any documented list view column set, not targeted by any
documented filter, not in a search index. Their only role was rendering the
advanced detail view — exactly the S3 responsibility per ADR 0011. The
parser already has all four in memory after XDR decode; emitting them to
JSON is mechanical.

**Why add `transfer_amount`.** The list pattern across both reference
explorers requires an Amount column. Without it, the list is either empty
(worse UX), wrong (showing fee or some other proxy), or fetched from S3 per
row (impractical at mainnet scale). Adding one NUMERIC column populated from
XDR-decoded details is the minimal way to serve the documented UI
requirement.

**Why primary amount only (not dual-amount for LP).** Storing
`transfer_amount_secondary`, `secondary_asset_code`, `secondary_asset_issuer`
to serve LP deposit/withdraw "dual view" inflates the table by ~5 bytes avg
per row × 1B+ rows without matching a documented list-column requirement.
StellarChain's list Amount column shows a single value per row. Full
dual-asset detail belongs in the detail view (S3 fetch already happens).

**Why drop `idx_ops_destination`.** No API endpoint filters by op
destination. The column stays (list "To" column derivation via LATERAL
JOIN), but a dedicated index on it is unused storage and maintenance cost.

**Why keep `id BIGSERIAL`.** Could be replaced by composite PK
`(transaction_id, application_order, created_at)` because no child table has
FK to `operations`. Storage saving ~8–16 GB at mainnet scale. Deferred —
changing the PK on a partitioned table with live partitions is more
invasive than the per-column drops in this ADR. Candidate for a follow-up
if the savings justify the migration cost.

### Consequences specific to `operations`

- **Database weight:** net reduction ~60–80 GB at mainnet scale (four columns
  × ~1–1.5B rows, offset by ~20 GB from new `transfer_amount`, plus dropped
  index).
- **Ingest simplicity:** parser gains ~5 lines — emits four extra fields to
  JSON, one fewer INSERT column in `operations`, and extends
  `transaction_participants` insertion loop to handle op-level source
  override (C5).
- **Endpoint behavior:** unchanged for all list/filter endpoints. Detail
  endpoints get four additional fields from the same S3 JSON they already
  fetch.
- **History reconstruction:** unchanged. Historical operations' detail fields
  are preserved in `parsed_ledger_{N}.json` on S3 (ADR 0011 write-once
  design). DB columns after `DROP COLUMN` are discarded; S3 remains the
  source of truth.
- **Failure modes for C4/C5:** same shape as C1/C2 — testable via parser
  unit tests (C4) and integration test covering op-level source override
  (C5). Not schema-level failures.

### Migration steps (operations)

Appended to the transactions migration sequence above; run as one
coordinated rollout:

1. **Parser update (same deploy as transactions parser update):**
   - Emit `source_account`, `source_account_muxed`, `destination_muxed`,
     `function_name` into `parsed_ledger_{N}.json` under `operations[j]`.
   - Populate `transfer_amount` on every `operations` INSERT from
     XDR-decoded details.
   - On op-level source override, insert extra `transaction_participants`
     row with `role='source'`.
2. **API update (same deploy as transactions API update):**
   - Advanced detail view reads the four offloaded per-op fields from S3
     JSON.
   - List view SQL includes `transfer_amount` in the LATERAL join on first
     op.
3. **Schema change:**
   ```sql
   DROP INDEX IF EXISTS idx_ops_destination;
   ALTER TABLE operations DROP COLUMN source_account;
   ALTER TABLE operations DROP COLUMN source_account_muxed;
   ALTER TABLE operations DROP COLUMN destination_muxed;
   ALTER TABLE operations DROP COLUMN function_name;
   ALTER TABLE operations ADD COLUMN transfer_amount NUMERIC(39,0);
   ```
4. **Backfill `transfer_amount`** for existing rows:
   - Optional for pre-GA; required if list view should show amounts for
     historical transactions. One-time SQL job reading `details` JSONB from
     parsed*ledger*{N}.json on S3 and populating the new column. Skippable
     if list "Amount" column can be blank for legacy rows.
5. **Verify:**
   - `\d operations` → expect 12 columns.
   - Run `/transactions?filter[contract_id]=X` — expect results.
   - Run `/tokens/:id/transactions` — expect Amount column populated.
   - Run `/accounts/:op_source_G/transactions` with a known op-level source
     override tx — expect the tx to appear.

Rollback: re-add columns via `ALTER TABLE ADD COLUMN`. Values recoverable
from S3 JSON via same-shape backfill.

---

## Token transfers removal

This section extends the ADR to **drop the `token_transfers` table entirely**
and replace its role with three narrow columns on `soroban_events` (for
SEP-41 transfer/mint/burn events) combined with existing `operations` (for
classic payments, path payments, and LP operations).

**Rationale at a glance.** `token_transfers` is the single heaviest table in
the schema — projected **300 GB – 1 TB** at mainnet. Every row duplicates
data that already lives in either `operations` (classic transfers) or
`soroban_events` (Soroban SEP-41 transfers). Removing it and adding three
targeted columns to `soroban_events` preserves full functionality with a
fraction of the storage and write amplification.

**Net effect:** 18 tables (was 19), projected net saving **~420–920 GB** at
mainnet.

### Context specific to `token_transfers`

`token_transfers` was introduced as a unified index for transfer-centric
queries (`/tokens/:id/transactions`, `/liquidity-pools/:id/transactions`,
account transfer filtering). Every row was a parallel INSERT duplicating
an existing record:

- **Classic payments and path payments**: row is already in `operations`
  with `type`, `destination`, `asset_code`, `asset_issuer`. After ADR 0018
  also carries `transfer_amount`. From is `transactions.source_account`
  (tx-level) or `parsed_ledger_{N}.json → operations[].source_account`
  (op-level override). So the full transfer is already indexable from
  `operations` + `transactions`.
- **Soroban SEP-41 transfers**: emitted as `ContractEvent` with
  `topic0 = Symbol("transfer"|"mint"|"burn")`. Currently `soroban_events`
  only stores the slim view (contract_id, topic0, event_index,
  ledger_sequence, created_at); from/to/amount live only in topics/data
  JSON on S3. Adding three narrow columns on `soroban_events` makes the
  full transfer indexable without a parallel table.
- **Classic LP deposit/withdraw**: row is already in `operations` with
  `type='liquidity_pool_deposit'` / `'liquidity_pool_withdraw'`, `pool_id`,
  `transfer_amount` (primary asset). Secondary amount lives in S3 detail
  per ADR 0018 decision.
- **Soroban AMM swaps (Soroswap, Phoenix-style)**: emit multiple transfer
  events per operation; each is a separate `soroban_events` row with typed
  `topic0` and (after this change) populated `transfer_from/to/amount`.

No transfer category loses indexable representation in DB. The only thing
that disappears is the parallel denormalized table.

### S3 bridge preserved per transfer (how you find which JSON)

A token's transfers are spread across many ledgers — this is correct.
**What matters is that every individual transfer row in DB carries its own
`ledger_sequence` pointer** to the parsed JSON file on S3. This pointer is
preserved in every source table:

| Source table     | Transfer type                                      | Bridge column to S3              |
| ---------------- | -------------------------------------------------- | -------------------------------- |
| `operations`     | classic payment, path_payment, LP deposit/withdraw | `operations.ledger_sequence`     |
| `soroban_events` | SEP-41 transfer/mint/burn, Soroban AMM events      | `soroban_events.ledger_sequence` |
| `transactions`   | the transaction containing any of the above        | `transactions.ledger_sequence`   |

List endpoints (`/tokens/:id/transactions`, `/liquidity-pools/:id/transactions`)
serve every row entirely from DB — all list-visible columns (hash, method,
from, to, amount, timestamp) are in DB columns, zero S3 fetches for a list.
The `ledger_sequence` on each row acts as the bridge: when a user clicks a
specific row for advanced detail, the backend fetches exactly
`parsed_ledger_{ledger_sequence}.json` on S3 for that single record.

This is **identical** to the bridge semantics that `token_transfers.ledger_sequence`
provided. Dropping the table does not break the bridge; each transfer still
carries its own ledger pointer in the source table it came from.

Concrete example — user opens `/tokens/USDC-GA5Z.../transactions`, backend
runs:

```sql
SELECT
    t.hash,
    o.type                  AS method,
    t.source_account        AS from_account,
    o.destination           AS to_account,
    o.transfer_amount       AS amount,
    o.ledger_sequence,                       -- per-row bridge to S3
    o.created_at
FROM operations o
JOIN transactions t ON (o.transaction_id, o.created_at) = (t.id, t.created_at)
WHERE o.asset_code = 'USDC' AND o.asset_issuer = 'GA5Z...KZVN'
  AND o.type IN ('payment', 'path_payment_strict_send', 'path_payment_strict_receive')
ORDER BY o.created_at DESC LIMIT 50;
```

Each row has its own `ledger_sequence` (e.g. 62,201,248, 62,201,247, …) so
any row the user clicks can resolve to exactly one S3 file.

### Columns removed (entire table dropped)

The following 17 columns from ADR 0017 `token_transfers` are all removed:

```
id, transaction_id, ledger_sequence, transfer_index,
asset_type, asset_code, asset_issuer, contract_id,
from_account, from_account_muxed,
to_account, to_account_muxed,
amount, transfer_type, pool_id, source,
created_at
```

Mapping per field to the new source location:

| Field from `token_transfers`              | New location                                                                                                                            |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `transaction_id`                          | `operations.transaction_id` or `soroban_events.transaction_id`                                                                          |
| `ledger_sequence`                         | `operations.ledger_sequence` or `soroban_events.ledger_sequence`                                                                        |
| `transfer_index`                          | Derivable from `operations.application_order` or `soroban_events.event_index`                                                           |
| `asset_type`                              | Derivable from which source table produced the row + presence of `contract_id` vs `asset_code + asset_issuer`                           |
| `asset_code`, `asset_issuer`              | `operations.asset_code`, `operations.asset_issuer` (classic)                                                                            |
| `contract_id`                             | `operations.contract_id` (classic SAC reference) / `soroban_events.contract_id` (Soroban-native)                                        |
| `from_account`                            | `transactions.source_account` (tx-level classic) / `soroban_events.transfer_from` (Soroban, new column below)                           |
| `from_account_muxed`                      | `parsed_ledger_{N}.json` — S3 detail only                                                                                               |
| `to_account`                              | `operations.destination` (classic) / `soroban_events.transfer_to` (Soroban, new column below)                                           |
| `to_account_muxed`                        | `parsed_ledger_{N}.json` — S3 detail only                                                                                               |
| `amount`                                  | `operations.transfer_amount` (classic, per ADR 0018) / `soroban_events.transfer_amount` (Soroban, new column below)                     |
| `transfer_type`                           | Derivable from `operations.type` (payment/path_payment/lp_deposit/lp_withdraw) or `soroban_events.topic0` (sym:transfer/mint/burn/swap) |
| `pool_id`                                 | `operations.pool_id` (classic LP) / `soroban_events.contract_id` (Soroban AMM)                                                          |
| `source` (provenance: operation vs event) | Implicit from the source table the backend queries                                                                                      |
| `created_at`                              | `operations.created_at` / `soroban_events.created_at`                                                                                   |

No field has an irreplaceable location in `token_transfers`.

### Columns added to `soroban_events`

```sql
ALTER TABLE soroban_events
  ADD COLUMN transfer_from   VARCHAR(56),      -- NULL for non-transfer events
  ADD COLUMN transfer_to     VARCHAR(56),      -- NULL for non-transfer events
  ADD COLUMN transfer_amount NUMERIC(39,0);    -- NULL for non-transfer events
```

Populated by parser **iff** `topic0 IN ('sym:transfer', 'sym:mint', 'sym:burn')`,
otherwise all three are NULL. For `sym:mint` events `transfer_from` is
NULL by convention; for `sym:burn` events `transfer_to` is NULL. The amount
is always non-NULL when `topic0` is in the set.

Size impact: ~40–80 GB at mainnet scale (rough envelope: ~50 B avg per
transfer-qualifying row × ~500 M–1 B events; NULLs cost ~1 byte each via
Postgres NULL bitmap).

### Indexes added on `soroban_events`

```sql
CREATE INDEX idx_events_transfer_from ON soroban_events (transfer_from, created_at DESC)
    WHERE transfer_from IS NOT NULL;
CREATE INDEX idx_events_transfer_to   ON soroban_events (transfer_to, created_at DESC)
    WHERE transfer_to IS NOT NULL;
```

Two partial indexes — both guarded by `WHERE ... IS NOT NULL` so that only
transfer-class events carry index entries. Keeps indexes proportional to
actual transfer volume rather than total event volume.

No index on `transfer_amount` (not a filter target).

### Indexes and FKs removed with the table

`DROP TABLE token_transfers` removes:

- Its composite FK to `transactions(id, created_at) ON DELETE CASCADE`.
- Its FKs to `soroban_contracts`, `liquidity_pools`, `accounts` (from_account,
  to_account, asset_issuer).
- Its six indexes: `idx_tt_contract`, `idx_tt_asset`, `idx_tt_from`,
  `idx_tt_to`, `idx_tt_pool`, `idx_tt_tx`.

### Additional parser conditions (C6, C7)

#### Condition 6 — Parser populates `soroban_events` transfer fields

For every Soroban event processed, if
`topic0 IN ('sym:transfer', 'sym:mint', 'sym:burn')`, the parser must
extract from the ScVal topics and data:

- `transfer_from` ← G-form of `topics[1]` `ScVal::Address` (NULL for
  `sym:mint`)
- `transfer_to` ← G-form of `topics[2]` `ScVal::Address` (NULL for
  `sym:burn`)
- `transfer_amount` ← i128 value from `data` `ScVal::I128`

For any other `topic0` (custom contract events, `sym:swap`, `sym:deposit`,
`sym:withdraw`, diagnostics, etc.) the three columns are left NULL. This
keeps the partial indexes tight.

M-form of the addresses (when the original ScVal Address carried a muxed
identifier) is preserved in full in the topics ScVal on the S3 JSON —
accessible via advanced detail view, never required in DB for list/filter.

#### Condition 7 — Parser stops writing to `token_transfers`

The parser code path that currently inserts rows into `token_transfers` is
removed. The table is dropped after parser migration. No dual-write
transition period is required (the project is pre-GA).

### DDL delta from ADR 0017 (with ADR 0018 `transactions` and `operations` deltas already applied)

```sql
-- Drop the heaviest table (cascades its six indexes and FKs):
DROP TABLE token_transfers;

-- Extend soroban_events with three transfer-targeted columns:
ALTER TABLE soroban_events
  ADD COLUMN transfer_from   VARCHAR(56),
  ADD COLUMN transfer_to     VARCHAR(56),
  ADD COLUMN transfer_amount NUMERIC(39,0);

-- Two partial indexes (use CONCURRENTLY on each partition in production):
CREATE INDEX idx_events_transfer_from
  ON soroban_events (transfer_from, created_at DESC)
  WHERE transfer_from IS NOT NULL;

CREATE INDEX idx_events_transfer_to
  ON soroban_events (transfer_to, created_at DESC)
  WHERE transfer_to IS NOT NULL;
```

`DROP TABLE` on a partitioned table cascades to all partitions. Fast
metadata + WAL; storage reclaim happens on VACUUM / `pg_repack`
post-drop.

### Per-endpoint verification

| Endpoint                                                                         | Query path after change                                                                     | Safe? |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ----- |
| `GET /tokens/:id/transactions` (classic asset)                                   | `operations WHERE asset_code = ? AND asset_issuer = ?`                                      | ✓     |
| `GET /tokens/:id/transactions` (Soroban contract)                                | `soroban_events WHERE contract_id = ? AND topic0 IN ('sym:transfer','sym:mint','sym:burn')` | ✓     |
| `GET /tokens/:id/transactions` (SAC — both legs)                                 | UNION ALL of the two queries above, merged by `created_at` in backend                       | ✓     |
| `GET /accounts/:id/transactions` with token-transfer participation               | `transaction_participants` (existing; source/destination/caller/fee_payer)                  | ✓     |
| `GET /liquidity-pools/:id/transactions` (classic LP)                             | `operations WHERE pool_id = ?`                                                              | ✓     |
| `GET /liquidity-pools/:id/transactions` (Soroban AMM)                            | `soroban_events WHERE contract_id = <pool_contract>`                                        | ✓     |
| "Soroban transfers to account X"                                                 | `soroban_events WHERE transfer_to = ?` (uses `idx_events_transfer_to`)                      | ✓     |
| "Soroban transfers from account X"                                               | `soroban_events WHERE transfer_from = ?` (uses `idx_events_transfer_from`)                  | ✓     |
| `GET /transactions/:hash` advanced detail (per-transfer muxed, full topics/data) | `parsed_ledger_{N}.json` on S3 — unchanged                                                  | ✓     |

Zero endpoint regressions. All list columns (from, to, amount, method,
hash, age) remain in DB. Per-row S3 bridge via `ledger_sequence` is
preserved in `operations` and `soroban_events`.

### Rationale specific to `token_transfers` removal

**Why drop the table, not shrink it.** `token_transfers` was a
materialized cross-table view — every row duplicated data from `operations`
(classic) or `soroban_events` (Soroban). Shrinking it would retain the
duplication and still cost ~200–300 GB. Dropping it eliminates the
duplication entirely and saves 420–920 GB net.

**Why three columns on `soroban_events`, not a dedicated
`soroban_transfers` table.** SEP-41 transfers are already emitted as
regular Soroban events. A separate table would again be a materialized
view over a subset of `soroban_events` — the same anti-pattern as
`token_transfers`. Keeping transfers as regular event rows, extended with
three narrow columns guarded by partial indexes, costs ~40–80 GB instead
of 200+ GB.

**Why UNION in the backend, not a DB view.** A PostgreSQL view
`UNION operations + soroban_events` would execute the same two index scans
on every call with no caching advantage. The backend is Rust; the UNION is
a 30–50-line function that is easy to audit and instrument. Same runtime
cost, lower model complexity.

**Why partial indexes.** Most Soroban events are not
transfer/mint/burn — they are custom contract events, swap events,
diagnostics, etc. `WHERE transfer_from IS NOT NULL` keeps index size
proportional to actual transfer events, not total event volume.

### Consequences specific to `token_transfers` removal

- **Database weight:** –420 to –920 GB net (drop 300 GB–1 TB table, add
  40–80 GB of narrow columns on `soroban_events`, drop 6 indexes, add 2
  partial indexes).
- **Write amplification:** every transfer produces **one** DB row instead
  of two. Parser work drops by one INSERT per transfer.
- **Endpoint performance:**
  - `/tokens/:id/transactions` (single-leg classic or Soroban): unchanged
    — one partition-pruned index scan.
  - `/tokens/:id/transactions` (SAC with both legs): two partition-pruned
    index scans + merge-sort by `created_at` in backend. Typical p95
    ~50–150 ms on popular tokens. Acceptable.
  - Account-centric filter: via `transaction_participants` (unchanged).
- **History correctness:** every transfer keeps its own `ledger_sequence`
  bridge in the source table. Advanced detail view stays on S3 via the
  same bridge.
- **Failure modes for C6/C7:** parser-level. Catch via unit tests on
  synthetic transfer events and integration tests for fee-bump and swap
  transactions. None is a schema-level failure.

### Migration steps (token_transfers removal)

Appended to the transactions and operations migration sequences:

1. **Parser update** (same deploy as the other parser updates):
   - Add extraction of `transfer_from`, `transfer_to`, `transfer_amount`
     from ScVal topics/data for `sym:transfer`, `sym:mint`, `sym:burn`
     events.
   - Remove the INSERT path writing to `token_transfers`.
2. **API update** (same deploy as other API updates):
   - Replace any `SELECT FROM token_transfers` with UNION ALL across
     `operations` + `soroban_events` using the query patterns documented
     above.
   - Advanced detail view continues reading from S3 JSON (unchanged).
3. **Schema change:**

   ```sql
   ALTER TABLE soroban_events
     ADD COLUMN transfer_from VARCHAR(56),
     ADD COLUMN transfer_to VARCHAR(56),
     ADD COLUMN transfer_amount NUMERIC(39,0);

   CREATE INDEX CONCURRENTLY idx_events_transfer_from
     ON soroban_events (transfer_from, created_at DESC)
     WHERE transfer_from IS NOT NULL;

   CREATE INDEX CONCURRENTLY idx_events_transfer_to
     ON soroban_events (transfer_to, created_at DESC)
     WHERE transfer_to IS NOT NULL;

   DROP TABLE token_transfers;
   ```

4. **Optional backfill** of `soroban_events.transfer_*` for existing
   transfer/mint/burn rows (pre-GA — can be skipped if historical list
   rows are acceptably blank; one-time SQL job reading
   `parsed_ledger_{N}.json` on S3 and populating the three columns).
5. **Verify:**
   - `\dt` — confirm `token_transfers` absent, `soroban_events` present.
   - Query `/tokens/:contract_id/transactions` for a known Soroban token
     — expect rows with from/to/amount populated.
   - Query `/tokens/USDC-GA5Z.../transactions` (classic) — expect rows
     from `operations`.
   - Query a SAC token (has both classic and Soroban sides) — expect
     UNION-merged results ordered by `created_at`.

Rollback: recreate `token_transfers` via prior DDL (from ADR 0017), drop
the three new columns from `soroban_events`. Historical data recoverable
from `parsed_ledger_{N}.json` on S3 via backfill. In practice rollback
should not be needed; schema change is pre-GA.

---

## Open questions

None. All seven parser conditions (C1–C7) are concrete, testable, and
within current ADR 0011/0013/0014 parser responsibilities.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0017: Ingest guard clarification, topic0 validation, final schema](0017_ingest-guard-clarification-topic0-validation-final-schema.md)
- [Backend Overview](../../docs/architecture/backend/backend-overview.md) — server-side XDR decode prohibition
- [Frontend Overview](../../docs/architecture/frontend/frontend-overview.md) — list column inventory per page
- [SEP-0023: Muxed Accounts](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0023.md)
- [SEP-0028: Fee Bump Transactions](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0015.md)
