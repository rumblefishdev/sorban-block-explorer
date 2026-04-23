---
id: '0021'
title: 'Schema ↔ endpoint ↔ frontend coverage matrix (post ADR 0011–0020)'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs:
  [
    '0011',
    '0012',
    '0013',
    '0014',
    '0015',
    '0016',
    '0017',
    '0018',
    '0019',
    '0020',
  ]
tags:
  [
    database,
    schema,
    api,
    endpoints,
    frontend,
    coverage,
    verification,
    reference,
  ]
links: []
history:
  - date: 2026-04-20
    status: proposed
    who: fmazur
    note: 'ADR created — comprehensive schema / endpoint / frontend verification. Documents the final 18-table schema after ADRs 0011–0020 and maps every one of the 22 backend endpoints to its DB / S3 data sources and to the frontend-overview.md view that consumes it.'
---

# ADR 0021: Schema ↔ endpoint ↔ frontend coverage matrix (post ADR 0011–0020)

**Related:**

- [ADR 0019: Schema snapshot and sizing at 11M ledgers](0019_schema-snapshot-and-sizing-11m-ledgers.md) — baseline snapshot
- [ADR 0020: transaction_participants cut to 3 cols; soroban_contracts index cut](0020_tp-drop-role-and-soroban-contracts-index-cut.md) — most recent delta

---

## Status

`proposed` — **verification document**, not a decision. Freezes the
result of ADR 0011–0020 iterative schema tightening and demonstrates
closure against the full endpoint surface in
[`backend-overview.md`](../../docs/architecture/backend/backend-overview.md) §6
and the full page surface in
[`frontend-overview.md`](../../docs/architecture/frontend/frontend-overview.md) §6.

Goal: confirm that every one of the 22 documented endpoints is
realizable from the final schema + S3 (`parsed_ledger_{N}.json`), and
that every visible element in every frontend view has a concrete source
of truth.

---

## Part I — Final schema (post ADR 0020)

### Table inventory

|  #  | Table                             |  Partitioned  | Purpose                                                                                                                          |
| :-: | --------------------------------- | :-----------: | -------------------------------------------------------------------------------------------------------------------------------- |
|  1  | `ledgers`                         |      no       | Chain head / history anchor                                                                                                      |
|  2  | `accounts`                        |      no       | Account identity + seen-range                                                                                                    |
|  3  | `transactions`                    | yes (monthly) | Transaction core, indexed for browsing / filter                                                                                  |
|  4  | `transaction_hash_index`          |      no       | Global hash uniqueness + lookup                                                                                                  |
|  5  | `operations`                      |      yes      | Per-operation slim columns (type, dest, asset, pool, transfer amount)                                                            |
|  6  | `transaction_participants`        |      yes      | `(account, tx)` edge — 3 cols only after ADR 0020                                                                                |
|  7  | `soroban_contracts`               |      no       | Contract identity + deployer + WASM hash + SAC flag + type + name                                                                |
|  8  | `wasm_interface_metadata`         |      no       | WASM ABI keyed by natural `wasm_hash`                                                                                            |
|  9  | `soroban_events`                  |      yes      | Events; carries transfer_from/to/amount for fungible/NFT transfer events                                                         |
| 10  | `soroban_invocations_appearances` |      yes      | Contract invocation appearance index (ADR 0034): (contract, tx, ledger, caller_id, amount); per-node detail at read time via XDR |
| 11  | `assets`                          |      no       | Canonical asset registry (classic_credit / SAC / Soroban / native)                                                               |
| 12  | `nfts`                            |      no       | NFT identity + current owner                                                                                                     |
| 13  | `nft_ownership`                   |      yes      | NFT ownership history (mint / transfer / burn)                                                                                   |
| 14  | `liquidity_pools`                 |      no       | Pool identity + assets + fee                                                                                                     |
| 15  | `liquidity_pool_snapshots`        |      yes      | Per-ledger pool state + derived TVL/volume/fees                                                                                  |
| 16  | `lp_positions`                    |      no       | Current LP shares per (pool, account)                                                                                            |
| 17  | `account_balances_current`        |      no       | Current balance per (account, asset)                                                                                             |
| 18  | `account_balance_history`         |      yes      | Balance history per (account, ledger, asset)                                                                                     |

**Bridges to S3:** every time-series table carries `ledger_sequence`.
Detail lookup path is always `ledger_sequence → parsed_ledger_{N}.json`.
`ledgers` is intentionally not a relational hub (no FK from other tables).

### Final DDL reference

Consolidated in [ADR 0019 §Full schema snapshot](0019_schema-snapshot-and-sizing-11m-ledgers.md)
with **one amendment from ADR 0020**:

```sql
-- transaction_participants (overrides ADR 0019 §6):
CREATE TABLE transaction_participants (
    transaction_id  BIGINT NOT NULL,
    account_id      VARCHAR(56) NOT NULL REFERENCES accounts(account_id),
    created_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, created_at, transaction_id),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tp_tx ON transaction_participants (transaction_id);

-- soroban_contracts indexes (overrides ADR 0019 §7 — remove idx_contracts_deployer):
-- KEEP: PK on contract_id, idx_contracts_type, idx_contracts_wasm,
--       idx_contracts_search, idx_contracts_prefix
-- REMOVE: idx_contracts_deployer
```

### S3 payload shape (parsed*ledger*{N}.json)

Every ledger produces one JSON file holding the full ledger content:

```
parsed_ledger_{N}.json:
  - ledger.{sequence, hash, closed_at, protocol_version, base_fee, ...}
  - transactions[]:
      - hash, source_account, fee_charged, successful
      - memo.{type, content}                            ← S3-only (ADR 0018)
      - envelope.signatures[]{signer, weight, hex}      ← S3-only (ADR 0018)
      - envelope.fee_bump.{fee_source, ...}             ← S3-only (fee-bump only)
      - envelope_xdr, result_xdr, result_meta_xdr       ← S3-only, advanced view
      - operations[]:
          - full operation body with source_account, muxed fields,
            raw parameters, return values
      - events[]: full decoded event payload (topic[1..N], data)
      - invocations[]: full call tree (args, return, sub-calls)
```

When an endpoint lists S3 as a source, the lookup is:

1. From the indexed row (e.g. `transactions.ledger_sequence` +
   `transactions.application_order`), resolve the S3 object key.
2. Fetch `parsed_ledger_{N}.json` via S3 GetObject.
3. Index by `application_order` to reach the specific transaction.

---

## Part II — Endpoint coverage matrix

For each endpoint, this section documents:

- **Sources** — SQL query skeleton and any S3 fetch.
- **Frontend consumer** — which page renders it, which fields are shown,
  quoted from `frontend-overview.md`.
- **Schema headroom** — what the schema _could_ surface if the spec
  expands later, without schema changes.

---

### E1. `GET /network/stats`

**Consumer:** Home (frontend-overview §6.2) — chain overview panel.

**Displayed per spec:**

> Chain overview — current ledger sequence, transactions per second,
> total accounts, total contracts.

**Sources (all DB):**

```sql
-- current ledger sequence + latest closed_at
SELECT sequence, closed_at
  FROM ledgers
 ORDER BY sequence DESC
 LIMIT 1;

-- total accounts (exact)
SELECT count(*) FROM accounts;

-- total contracts (exact)
SELECT count(*) FROM soroban_contracts;

-- TPS (last 1 min window)
SELECT count(*)::float / 60
  FROM transactions
 WHERE created_at > now() - interval '1 minute';
```

**Schema headroom:** could add "total transactions indexed" (trivial
`COUNT` over `transaction_hash_index`), "total tokens", "total pools",
"block time" (avg gap between last N ledgers).

---

### E2. `GET /transactions`

**Consumer:** Transactions page (§6.3) — paginated, filterable table.

**Displayed per spec:**

> Transaction table — hash, ledger sequence, source account, operation
> type, status badge (success/failed), fee, timestamp.
> Filters — source account, contract ID, operation type.
> Cursor-based pagination controls.

**Sources (all DB):**

```sql
-- No filter: recent first, cursor on (created_at, id)
SELECT t.id, t.hash, t.ledger_sequence, t.source_account,
       t.successful, t.fee_charged, t.created_at
  FROM transactions t
 WHERE (t.created_at, t.id) < (:cursor_created_at, :cursor_id)
 ORDER BY t.created_at DESC, t.id DESC
 LIMIT :limit;

-- filter[source_account] → hits idx_tx_source_created
WHERE t.source_account = :account_id

-- filter[contract_id] → join operations ... WHERE contract_id = :cid
SELECT DISTINCT t.*
  FROM transactions t
  JOIN operations o
       ON o.transaction_id = t.id AND o.created_at = t.created_at
 WHERE o.contract_id = :contract_id
 ORDER BY t.created_at DESC;

-- filter[operation_type] → join operations with type predicate (hits idx_ops_type)
WHERE o.type = :op_type
```

**Operation-type display per row** ("operation type" column): the row
shows a single representative type. Strategy: `operations.type` for the
first operation, or a summary string when `operation_count > 1`. Both
resolvable from `transactions.operation_count` + a secondary index fetch
of the first `operations` row.

**Schema headroom:** could filter by asset (`idx_ops_asset`), pool
(`idx_ops_pool`), or Soroban-only (`idx_tx_has_soroban`, already
defined), without new indexes.

---

### E3. `GET /transactions/:hash`

**Consumer:** Transaction page (§6.4) — both **Normal** and **Advanced** modes.

**Displayed per spec (base fields shared by both modes):**

> Transaction hash (full, copyable), status badge (success/failed),
> ledger sequence (link), timestamp.
> Fee charged (XLM + stroops), source account (link), memo (type + content).
> Signatures — signer, weight, signature hex.

**Normal mode:** operation graph/tree with human-readable summaries
("Sent 1,250 USDC to GD2M...K8J1"), Soroban invocation call tree.

**Advanced mode:** raw parameters, argument values, return values,
events (type, topics, raw data), diagnostic events, collapsible raw
`envelope_xdr`, `result_xdr`, `result_meta_xdr`.

**Sources:**

| Field                           | Source                                                                                                                                                           |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hash`                          | `transaction_hash_index` → `transactions`                                                                                                                        |
| `status badge`                  | `transactions.successful`                                                                                                                                        |
| `ledger sequence`               | `transactions.ledger_sequence`                                                                                                                                   |
| `timestamp`                     | `transactions.created_at` + `ledgers.closed_at`                                                                                                                  |
| `fee charged`                   | `transactions.fee_charged` (stroops; XLM = /1e7)                                                                                                                 |
| `source account`                | `transactions.source_account`                                                                                                                                    |
| `memo (type + content)`         | **S3** `parsed_ledger_{N}.json`.transactions[app_order].memo                                                                                                     |
| `signatures[]`                  | **S3** `parsed_ledger_{N}.json`.transactions[app_order].envelope.signatures                                                                                      |
| Normal mode operations summary  | `operations` (type, destination, asset, pool, transfer_amount) + **S3** for rich text ("sent X to Y")                                                            |
| Normal mode Soroban tree        | `soroban_invocations_appearances` (does this tx have invocations? + `amount` display) + **Read-time XDR** for per-node detail and call-tree hierarchy (ADR 0034) |
| Advanced mode raw params        | **S3**                                                                                                                                                           |
| Advanced mode events            | `soroban_events_appearances` (does this tx have contract events? + `amount` display) + **S3** for type, topics, data, event_index (ADR 0033)                     |
| Advanced mode diagnostic events | **S3**                                                                                                                                                           |
| Advanced mode XDR               | **S3** (`envelope_xdr`, `result_xdr`, `result_meta_xdr`)                                                                                                         |

**Hash-lookup resolver (fail-fast per ADR 0016):**

```sql
SELECT h.hash, h.ledger_sequence, h.created_at
  FROM transaction_hash_index h
 WHERE h.hash = :hash;
-- If miss → 404 immediately.

-- Then fetch the row:
SELECT t.* FROM transactions t
 WHERE t.hash = :hash AND t.created_at = :created_at;
```

**Schema headroom:** `has_soroban` flag lets the backend skip
`soroban_events_appearances` + `soroban_invocations_appearances` joins
for classic-only tx.
`parse_error` flag lets advanced mode surface parse warnings.
`inner_tx_hash` signals fee-bump (advanced mode shows outer fee-payer from S3).

---

### E4. `GET /ledgers`

**Consumer:** Ledgers page (§6.5).

**Displayed per spec:**

> Ledger table — sequence, hash (truncated), closed_at, protocol version,
> transaction count. Cursor-based pagination controls.

**Source (DB only):**

```sql
SELECT sequence, hash, closed_at, protocol_version, transaction_count
  FROM ledgers
 WHERE sequence < :cursor_sequence
 ORDER BY sequence DESC
 LIMIT :limit;
```

Directly covered by `idx_ledgers_closed_at` and PK.

**Schema headroom:** `base_fee` available for display, and "gap to
previous ledger" derivable from consecutive `closed_at`.

---

### E5. `GET /ledgers/:sequence`

**Consumer:** Ledger page (§6.6).

**Displayed per spec:**

> Ledger summary — sequence, hash, closed_at, protocol version,
> transaction count, base fee.
> Transactions in ledger — paginated table of all transactions in this
> ledger.
> Previous / next ledger navigation.

**Sources (DB only):**

```sql
-- Summary
SELECT * FROM ledgers WHERE sequence = :sequence;

-- Prev / next navigation
SELECT sequence FROM ledgers
 WHERE sequence < :sequence ORDER BY sequence DESC LIMIT 1;
SELECT sequence FROM ledgers
 WHERE sequence > :sequence ORDER BY sequence ASC  LIMIT 1;

-- Transactions in ledger (uses idx_tx_ledger)
SELECT id, hash, source_account, successful, fee_charged,
       application_order, operation_count, created_at
  FROM transactions
 WHERE ledger_sequence = :sequence
 ORDER BY application_order
 LIMIT :limit OFFSET :offset;
```

**Schema headroom:** could aggregate fee stats per ledger (`SUM(fee_charged)`),
op-type breakdown (`GROUP BY type` on operations filtered by
`ledger_sequence`).

---

### E6. `GET /accounts/:account_id`

**Consumer:** Account page (§6.7) — summary + balances section.

**Displayed per spec:**

> Account summary — account ID (full, copyable), sequence number,
> first seen ledger, last seen ledger.
> Balances — native XLM balance and trustline/token balances.

**Sources (DB only):**

```sql
-- Summary
SELECT account_id, sequence_number, first_seen_ledger, last_seen_ledger, home_domain
  FROM accounts WHERE account_id = :account_id;

-- Balances
SELECT asset_type, asset_code, issuer, balance, last_updated_ledger
  FROM account_balances_current
 WHERE account_id = :account_id
 ORDER BY (asset_type = 'native') DESC, asset_code;
```

**Schema headroom:** `home_domain` exposed as "Home Domain" row.
`account_balance_history` supports a future "balance over time" chart.
`lp_positions` supports a future "pool positions" tab.

---

### E7. `GET /accounts/:account_id/transactions`

**Consumer:** Account page (§6.7) — recent transactions table.

**Displayed per spec:**

> Recent transactions — paginated table of transactions involving this
> account. Linked transactions should reuse the same visual conventions
> as the global transactions page.

Same columns as §6.3: hash, ledger sequence, source account, operation
type, status, fee, timestamp.

**Sources (DB only — this is the sole consumer of `transaction_participants`):**

```sql
SELECT t.id, t.hash, t.ledger_sequence, t.source_account,
       t.successful, t.fee_charged, t.created_at
  FROM transaction_participants tp
  JOIN transactions t
       ON t.id = tp.transaction_id AND t.created_at = tp.created_at
 WHERE tp.account_id = :account_id
   AND (tp.created_at, tp.transaction_id) < (:cursor_ca, :cursor_id)
 ORDER BY tp.created_at DESC, tp.transaction_id DESC
 LIMIT :limit;
```

The JOIN hits `transaction_participants` PK `(account_id, created_at,
transaction_id)` directly — no sort needed beyond index order. Post-ADR
0020 this table is ~160 GB (vs ~420 GB before), dramatically cheaper
per-query.

**Schema headroom:** adding role information to this view requires the
resolver documented in ADR 0020 §Role reconstruction (4 roles from DB,
2 from S3). Not required by current spec.

---

### E8. `GET /tokens`

**Consumer:** Tokens page (§6.8) — paginated list.

**Displayed per spec:**

> Token table — asset code, issuer / contract ID, type (classic / SAC /
> Soroban), total supply, holder count.
> Filters — type (classic, SAC, Soroban), asset code search.

**Sources:**

| Field                  | Source                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------- |
| `asset_code`           | `assets.asset_code`                                                                                           |
| `issuer / contract ID` | `assets.issuer_address` or `assets.contract_id`                                                               |
| `type`                 | `assets.asset_type` (native/classic_credit/sac/soroban)                                                       |
| `total_supply`         | **computed** — `SUM(balance) FROM account_balances_current WHERE asset matches`                               |
| `holder_count`         | **computed** — `COUNT(DISTINCT account_id) FROM account_balances_current WHERE asset matches AND balance > 0` |

Filter `filter[type]` uses `idx_assets_type`.
Filter `filter[code]` uses `idx_assets_code_trgm` (trigram substring).

```sql
SELECT t.id, t.asset_type, t.asset_code, t.issuer_address,
       t.contract_id, t.name
  FROM assets t
 WHERE (:type IS NULL OR t.asset_type = :type)
   AND (:code IS NULL OR t.asset_code ILIKE '%' || :code || '%')
 ORDER BY t.id DESC
 LIMIT :limit OFFSET :offset;

-- For each row (or in a batched query):
SELECT SUM(balance) AS supply, COUNT(DISTINCT account_id) AS holders
  FROM account_balances_current
 WHERE asset_type = :asset_type
   AND asset_code = :asset_code
   AND issuer     = :issuer;
```

Holder-count / supply are hot, denormalizable later if the aggregate
cost is high (not addressed by this ADR — see ADR 0135 task for holder
count tracking if promoted).

**Schema headroom:** `assets.search_vector` (GIN) supports full-text
token name search. `assets.decimals` supports formatted-amount display.

---

### E9. `GET /tokens/:id`

**Consumer:** Token page (§6.9) — summary + metadata + recent tx.

**Displayed per spec:**

> Token summary — asset code, issuer or contract ID (copyable), type
> badge, total supply, holder count, deployed at ledger (if Soroban).
> Metadata — name, description, icon (if available), domain/home page.
> Latest transactions — paginated table of recent transactions
> involving this token.

**Sources:**

| Field                                         | Source                                                                      |
| --------------------------------------------- | --------------------------------------------------------------------------- |
| `asset_code`, `issuer`, `contract_id`, `type` | `assets`                                                                    |
| `total_supply`, `holder_count`                | see E8 (computed from `account_balances_current`)                           |
| `deployed at ledger` (Soroban only)           | `soroban_contracts.deployed_at_ledger` via `assets.contract_id`             |
| `name`                                        | `assets.name`                                                               |
| `description`, `icon`, `home page`            | **S3** from `parsed_ledger_{metadata_ledger}.json` (stellar asset metadata) |
| `home domain` (classic)                       | `accounts.home_domain` via `assets.issuer_address`                          |

**Schema headroom:** `assets.metadata_ledger` pinpoints the S3 file that
carries full metadata. No extra indexes needed.

---

### E10. `GET /tokens/:id/transactions`

**Consumer:** Token page (§6.9) — recent transactions tab.

**Displayed per spec:** same row shape as §6.3.

**Sources:**

For **classic assets**: JOIN through `operations` on `(asset_code, asset_issuer)`:

```sql
SELECT DISTINCT t.id, t.hash, t.ledger_sequence, t.source_account,
       t.successful, t.fee_charged, t.created_at
  FROM operations o
  JOIN transactions t
       ON t.id = o.transaction_id AND t.created_at = o.created_at
 WHERE o.asset_code   = :asset_code
   AND o.asset_issuer = :issuer
 ORDER BY t.created_at DESC
 LIMIT :limit;
```

For **Soroban tokens** (contract-based): JOIN through
`soroban_events_appearances` on `contract_id` for the list of candidate
transactions; the "is this a transfer?" classification lives on the read
path, not on a DB column (ADR 0033):

```sql
SELECT DISTINCT t.id, t.hash, t.ledger_sequence, t.source_account,
       t.successful, t.fee_charged, t.created_at
  FROM soroban_events_appearances e
  JOIN transactions t
       ON t.id = e.transaction_id AND t.created_at = e.created_at
 WHERE e.contract_id = :contract_id
 ORDER BY t.created_at DESC
 LIMIT :limit;
```

Transfer-only filtering is performed after the DB step by fetching the
relevant ledgers' XDR from the public archive and running
`xdr_parser::is_transfer_event` against each event. This trades a tiny
per-ledger archive fetch for removing the `transfer_amount IS NOT NULL`
DB column that ADR 0033 dropped.

(UNION ALL at the API layer if a token has both classic and Soroban
representations — rare but possible for wrapped assets.)

Indexes hit: `idx_ops_asset`, `idx_sea_contract_ledger`.

---

### E11. `GET /contracts/:contract_id`

**Consumer:** Contract page (§6.10) — summary card.

**Displayed per spec:**

> Contract summary — contract ID (full, copyable), deployer account
> (link), deployed at ledger (link), WASM hash, SAC badge if applicable.

Plus: _Stats — total invocations count, unique callers._

**Sources:** ADR 0034 collapses the invocation-stats side of this endpoint
to `soroban_invocations_appearances`. Stats are aggregated over per-trio
rows: `SUM(amount)` replaces `COUNT(*)`, `COUNT(DISTINCT caller_id)` is
preserved bit-for-bit because the appearance's `caller_id` is the
root-level caller of the trio — matching the pre-refactor staging filter
(ADR 0034 §3, §6). Summary-card columns stay on `soroban_contracts`
(unchanged).

```sql
SELECT contract_id, deployer_account, deployed_at_ledger,
       wasm_hash, is_sac, contract_type, name
  FROM soroban_contracts
 WHERE contract_id = :contract_id;

-- Stats (cacheable)
SELECT SUM(amount)              AS total_invocations,
       COUNT(DISTINCT caller_id) AS unique_callers
  FROM soroban_invocations_appearances
 WHERE contract_id = :contract_id;
```

PK lookup on `soroban_contracts.contract_id`. Stats uses
`idx_sia_contract_ledger`.

**Schema headroom:** `name` available for "Contract name" header.
`contract_type` (`nft`/`fungible`/`token`/`other`) available for badge
expansion beyond SAC.

---

### E12. `GET /contracts/:contract_id/interface`

**Consumer:** Contract page (§6.10) — interface section.

**Displayed per spec:**

> Contract interface — list of public functions with parameter names
> and types, allowing users to understand the contract's API without
> reading source code.

**Sources (DB only):**

```sql
SELECT wim.name, wim.contract_type
  FROM soroban_contracts sc
  JOIN wasm_interface_metadata wim ON wim.wasm_hash = sc.wasm_hash
 WHERE sc.contract_id = :contract_id;
```

Full interface (function signatures, parameter names/types, return
types) is stored in `wasm_interface_metadata` — note: `wasm_interface_metadata`
in the final schema holds only `name`, `uploaded_at_ledger`,
`contract_type`. **Richer ABI (function list, params, returns) must be
added** — planned as a JSONB column or companion table once the
WASM-analysis pipeline ships. Not blocked by the current minimal
schema; extension is non-breaking.

**Action item outside this ADR:** add `wasm_interface_metadata.abi
JSONB` column in a future small migration when WASM parsing lands.

---

### E13. `GET /contracts/:contract_id/invocations`

**Consumer:** Contract page (§6.10) — Invocations tab.

**Displayed per spec:**

> Invocations tab — recent invocations table (function name, caller
> account, status, ledger, timestamp).

**Sources:** ADR 0034 collapses E13's DB side to an appearance index;
per-node detail (function name, per-node index, success flag, args,
return value, depth) is materialised at read time from the public
archive. The appearance row carries `caller_id` as an unindexed payload
(preserved for the E11 stat), but per-row caller rendering in E13 comes
from the parser output so that sub-invocation contract-callers — which
staging filters to NULL in the DB — are visible per tree node.

| Field                                                  | Source                                                                        |
| ------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `ledger_sequence`                                      | `soroban_invocations_appearances.ledger_sequence`                             |
| `transaction_id` / `transaction_hash`                  | `soroban_invocations_appearances.transaction_id` → `transactions`             |
| `amount` (invocation tree nodes in this trio)          | `soroban_invocations_appearances.amount`                                      |
| `function_name`, `caller_account`, `successful`, depth | **Read-time XDR** — `xdr_parser::extract_invocations` filtered by contract_id |
| `function_args`, `return_value`                        | **Read-time XDR** — same parser call                                          |

```sql
SELECT contract_id, transaction_id, ledger_sequence, amount, created_at
  FROM soroban_invocations_appearances
 WHERE contract_id = :contract_id
   AND (ledger_sequence, transaction_id) < (:cursor_ledger, :cursor_tx)
 ORDER BY ledger_sequence DESC, transaction_id DESC
 LIMIT :limit;
```

Hits `idx_sia_contract_ledger`. For each distinct `ledger_sequence` in
the page, one public-archive `GetObject` + `xdr_parser::extract_invocations`
decodes every invocation tree the contract participates in for that
ledger; each appearance row then expands into its `amount` consecutive
nodes (depth-first, as the parser emits). Request-scope memoisation
keeps a ledger parsed once per page — shared with E14's read path.

---

### E14. `GET /contracts/:contract_id/events`

**Consumer:** Contract page (§6.10) — Events tab.

**Displayed per spec:**

> Events tab — recent events table (event type, topics, data, ledger).

**Sources:** ADR 0033 collapses E14's DB side to an appearance index; all
parsed event detail (type, full topics array, data, per-event index, transfer
triple) is materialised at read time from the public archive.

| Field                                                         | Source                                                             |
| ------------------------------------------------------------- | ------------------------------------------------------------------ |
| `ledger_sequence`                                             | `soroban_events_appearances.ledger_sequence`                       |
| `transaction_id` / `transaction_hash`                         | `soroban_events_appearances.transaction_id` → `transactions`       |
| `amount` (events in this trio)                                | `soroban_events_appearances.amount`                                |
| `event_type`, `topics[0..N]`, `data`, per-event `event_index` | **S3** — `xdr_parser::extract_events` filtered by `contract_id`    |
| transfer summary (from / to / amount)                         | **S3** — `xdr_parser::parse_transfer` over the decoded topics/data |

```sql
SELECT contract_id, transaction_id, ledger_sequence, amount, created_at
  FROM soroban_events_appearances
 WHERE contract_id = :contract_id
   AND (ledger_sequence, transaction_id) < (:cursor_ledger, :cursor_tx)
 ORDER BY ledger_sequence DESC, transaction_id DESC
 LIMIT :limit;
```

Hits `idx_sea_contract_ledger`. For each distinct `ledger_sequence` in the
page, one public-archive `GetObject` + `xdr_parser::extract_events` decodes
every event the contract emitted in that ledger; each appearance row then
expands into its `amount` consecutive events. Request-scope memoisation
keeps a ledger parsed once per page.

---

### E15. `GET /nfts`

**Consumer:** NFTs page (§6.11).

**Displayed per spec:**

> NFT table — name/identifier, collection name, contract ID, owner,
> preview image.
> Filters — collection, contract ID.

**Sources (DB only):**

```sql
SELECT n.id, n.name, n.token_id, n.collection_name, n.contract_id,
       n.current_owner, n.media_url
  FROM nfts n
 WHERE (:collection IS NULL OR n.collection_name = :collection)
   AND (:contract_id IS NULL OR n.contract_id = :contract_id)
 ORDER BY n.id DESC
 LIMIT :limit;
```

Filter `collection` hits `idx_nfts_collection`. Filter `contract_id` hits
UNIQUE index on `(contract_id, token_id)`.

---

### E16. `GET /nfts/:id`

**Consumer:** NFT page (§6.12).

**Displayed per spec:**

> NFT summary — name, identifier/token ID, collection name, contract ID
> (link), owner account (link).
> Media preview — image, video, or other media.
> Metadata — full attribute list (traits, properties).

**Sources:**

| Field                                                                              | Source                |
| ---------------------------------------------------------------------------------- | --------------------- |
| `name`, `token_id`, `collection_name`, `contract_id`, `current_owner`, `media_url` | `nfts` (PK lookup)    |
| `metadata` (traits, properties)                                                    | `nfts.metadata` JSONB |

```sql
SELECT * FROM nfts WHERE id = :id;
```

All metadata is already in the DB as JSONB (decoded at ingest time).

---

### E17. `GET /nfts/:id/transfers`

**Consumer:** NFT page (§6.12) — transfer history section.

**Displayed per spec:**

> Transfer history — table of ownership changes.

**Sources (DB only):**

```sql
SELECT no.event_type, no.owner_account, no.ledger_sequence,
       no.created_at, t.hash
  FROM nft_ownership no
  JOIN transactions t
       ON t.id = no.transaction_id AND t.created_at = no.created_at
 WHERE no.nft_id = :nft_id
 ORDER BY no.created_at DESC, no.event_order DESC
 LIMIT :limit;
```

`nft_ownership` PK `(nft_id, created_at, ledger_sequence, event_order)`
makes this a direct index scan.

---

### E18. `GET /liquidity-pools`

**Consumer:** Liquidity Pools page (§6.13).

**Displayed per spec:**

> Pool table — pool ID (truncated), asset pair (e.g. XLM/USDC), total
> shares, reserves per asset, fee percentage.
> Filters — asset pair, minimum TVL.

**Sources (DB — latest snapshot per pool):**

```sql
SELECT lp.pool_id, lp.asset_a_code, lp.asset_a_issuer,
       lp.asset_b_code, lp.asset_b_issuer, lp.fee_bps,
       s.reserve_a, s.reserve_b, s.total_shares, s.tvl
  FROM liquidity_pools lp
  JOIN LATERAL (
         SELECT reserve_a, reserve_b, total_shares, tvl
           FROM liquidity_pool_snapshots
          WHERE pool_id = lp.pool_id
          ORDER BY created_at DESC
          LIMIT 1
       ) s ON TRUE
 WHERE (:min_tvl IS NULL OR s.tvl >= :min_tvl)
   AND (:asset_a_code IS NULL OR lp.asset_a_code = :asset_a_code)
 ORDER BY s.tvl DESC NULLS LAST
 LIMIT :limit;
```

Hits `idx_lps_pool` (pool_id, created_at DESC) for LATERAL, and
`idx_lps_tvl` for the outer sort. Asset filter hits `idx_pools_asset_a`
/ `idx_pools_asset_b`.

---

### E19. `GET /liquidity-pools/:id`

**Consumer:** Liquidity Pool page (§6.14) — summary + participants.

**Displayed per spec:**

> Pool summary — pool ID (full, copyable), asset pair, fee percentage,
> total shares, reserves per asset.
> Pool participants — table of liquidity providers and their share.

**Sources (DB only):**

```sql
-- Summary + latest state
SELECT lp.*, s.reserve_a, s.reserve_b, s.total_shares, s.tvl, s.created_at
  FROM liquidity_pools lp
  JOIN LATERAL (
         SELECT * FROM liquidity_pool_snapshots
          WHERE pool_id = lp.pool_id
          ORDER BY created_at DESC LIMIT 1
       ) s ON TRUE
 WHERE lp.pool_id = :pool_id;

-- Participants
SELECT account_id, shares, first_deposit_ledger, last_updated_ledger
  FROM lp_positions
 WHERE pool_id = :pool_id AND shares > 0
 ORDER BY shares DESC
 LIMIT :limit;
```

Participants use `idx_lpp_shares` (pool_id, shares DESC WHERE shares > 0).

---

### E20. `GET /liquidity-pools/:id/transactions`

**Consumer:** Liquidity Pool page (§6.14) — recent transactions.

**Displayed per spec:**

> Recent transactions — deposits, withdrawals, and trades involving
> this pool.

**Sources (DB only):**

```sql
SELECT DISTINCT t.id, t.hash, o.type, t.ledger_sequence,
       t.source_account, t.successful, t.created_at
  FROM operations o
  JOIN transactions t
       ON t.id = o.transaction_id AND t.created_at = o.created_at
 WHERE o.pool_id = :pool_id
 ORDER BY t.created_at DESC
 LIMIT :limit;
```

Hits `idx_ops_pool`.

**Schema headroom:** `operations.type` carries the trade vs
deposit/withdraw distinction ("change_trust", "path_payment_strict_send",
"liquidity_pool_deposit", etc.). Badge rendering is a UI concern.

---

### E21. `GET /liquidity-pools/:id/chart`

**Consumer:** Liquidity Pool page (§6.14) — time-series charts.

**Displayed per spec:**

> Charts — TVL over time, volume over time, fee revenue.

**Sources (DB only):**

```sql
-- For interval = '1d':
SELECT date_trunc('day', created_at) AS bucket,
       AVG(tvl)         AS tvl,
       SUM(volume)      AS volume,
       SUM(fee_revenue) AS fee_revenue
  FROM liquidity_pool_snapshots
 WHERE pool_id = :pool_id
   AND created_at BETWEEN :from AND :to
 GROUP BY bucket
 ORDER BY bucket;
```

All three series come from `liquidity_pool_snapshots` which is per-ledger
materialized. Monthly partitioning keeps long-range scans cheap.

---

### E22. `GET /search?q=&type=...`

**Consumer:** Search Results page (§6.15).

**Displayed per spec:**

> Generic search across all entity types. For exact matches, redirects
> to detail page. Otherwise displays grouped results (transactions,
> contracts, tokens, accounts, NFTs, liquidity pools).

**Sources (DB only, classified per query shape):**

| Classification             | Target table                      | Index                                       |
| -------------------------- | --------------------------------- | ------------------------------------------- |
| Full hex hash (64 chars)   | `transaction_hash_index` (exact)  | PK                                          |
| Starts with `G` (56 chars) | `accounts` (exact or prefix)      | `idx_accounts_prefix` (text_pattern_ops)    |
| Starts with `C` (56 chars) | `soroban_contracts`               | `idx_contracts_prefix`                      |
| Pool ID prefix             | `liquidity_pools`                 | `idx_pools_prefix`                          |
| Short asset code           | `assets` full-text + trigram      | `idx_assets_search`, `idx_assets_code_trgm` |
| Contract name              | `soroban_contracts.search_vector` | `idx_contracts_search` (GIN)                |
| NFT name / collection      | `nfts` trigram / collection       | `idx_nfts_name_trgm`, `idx_nfts_collection` |

Exact-match redirect is driven by length + character-class tests on
`q`. Grouped broad search uses UNION ALL across the above targets with
bounded per-type limits.

---

## Part III — Coverage summary

|  #  | Endpoint                                  | DB only? |                                      S3 needed?                                      |                                   All spec fields realized?                                   |
| :-: | ----------------------------------------- | :------: | :----------------------------------------------------------------------------------: | :-------------------------------------------------------------------------------------------: |
| E1  | `GET /network/stats`                      |   yes    |                                          no                                          |                                              yes                                              |
| E2  | `GET /transactions`                       |   yes    |                                          no                                          |                                              yes                                              |
| E3  | `GET /transactions/:hash`                 |    no    | **yes** (memo, signatures, full op params, XDR, diagnostic events, full topics/data) |                                              yes                                              |
| E4  | `GET /ledgers`                            |   yes    |                                          no                                          |                                              yes                                              |
| E5  | `GET /ledgers/:sequence`                  |   yes    |                                          no                                          |                                              yes                                              |
| E6  | `GET /accounts/:account_id`               |   yes    |                                          no                                          |                                              yes                                              |
| E7  | `GET /accounts/:account_id/transactions`  |   yes    |                                          no                                          |                                              yes                                              |
| E8  | `GET /tokens`                             |   yes    |                                          no                                          |                                 yes (supply/holders computed)                                 |
| E9  | `GET /tokens/:id`                         | partial  |                   **yes** (description, icon, home page metadata)                    |                                              yes                                              |
| E10 | `GET /tokens/:id/transactions`            |   yes    |                                          no                                          |                                              yes                                              |
| E11 | `GET /contracts/:contract_id`             |   yes    |                                          no                                          |                                              yes                                              |
| E12 | `GET /contracts/:contract_id/interface`   |   yes    |                                          no                                          | **requires future `wasm_interface_metadata.abi` column** (non-breaking; tracked as follow-up) |
| E13 | `GET /contracts/:contract_id/invocations` | partial  |  **yes** (function_name, caller_account, successful, args, return, depth per node)   |                                              yes                                              |
| E14 | `GET /contracts/:contract_id/events`      | partial  |                           **yes** (topic[1..N], raw data)                            |                                              yes                                              |
| E15 | `GET /nfts`                               |   yes    |                                          no                                          |                                              yes                                              |
| E16 | `GET /nfts/:id`                           |   yes    |                                          no                                          |                                              yes                                              |
| E17 | `GET /nfts/:id/transfers`                 |   yes    |                                          no                                          |                                              yes                                              |
| E18 | `GET /liquidity-pools`                    |   yes    |                                          no                                          |                                              yes                                              |
| E19 | `GET /liquidity-pools/:id`                |   yes    |                                          no                                          |                                              yes                                              |
| E20 | `GET /liquidity-pools/:id/transactions`   |   yes    |                                          no                                          |                                              yes                                              |
| E21 | `GET /liquidity-pools/:id/chart`          |   yes    |                                          no                                          |                                              yes                                              |
| E22 | `GET /search`                             |   yes    |                                          no                                          |                                              yes                                              |

### S3 dependencies (aggregated)

S3 fetches are bounded to **three kinds of views**, matching the S3
offload principle of ADR 0011 and ADR 0018:

1. **Transaction detail (E3)** — memo, signatures, advanced mode raw params, XDR.
2. **Token detail metadata (E9)** — description, icon, home domain (classic asset metadata page).
3. **Event detail expansion (E14)** — topic[1..N] and raw data when user expands a specific event.

No list endpoint requires S3. Every paginated browsing experience is
served by indexed columns alone.

### Frontend coverage summary

Every displayed element in frontend-overview.md §6 is mapped:

| Page                                   | §ref |                                        Covered?                                        |
| -------------------------------------- | :--: | :------------------------------------------------------------------------------------: |
| Home                                   | 6.2  |                                          yes                                           |
| Transactions list                      | 6.3  |                                          yes                                           |
| Transaction detail (normal + advanced) | 6.4  |                                          yes                                           |
| Ledgers list                           | 6.5  |                                          yes                                           |
| Ledger detail                          | 6.6  |                                          yes                                           |
| Account detail                         | 6.7  |                                          yes                                           |
| Tokens list                            | 6.8  |                                          yes                                           |
| Token detail                           | 6.9  |                                          yes                                           |
| Contract detail                        | 6.10 | yes (ABI interface needs `wasm_interface_metadata.abi` column — non-breaking addition) |
| NFTs list                              | 6.11 |                                          yes                                           |
| NFT detail                             | 6.12 |                                          yes                                           |
| Liquidity Pools list                   | 6.13 |                                          yes                                           |
| Liquidity Pool detail                  | 6.14 |                                          yes                                           |
| Search results                         | 6.15 |                                          yes                                           |

---

## Part IV — Known follow-ups (tracked, not blocking)

1. **`wasm_interface_metadata.abi`** — the current schema stores only
   `name` / `contract_type` / `uploaded_at_ledger`. The function-list
   rendering required by §6.10 Contract interface and E12 endpoint
   needs a JSONB (or companion table) holding parsed ABI. Adding this
   column is non-breaking and independent of ADRs 0011–0020.

2. **Token `supply` / `holder_count`** (E8, E9) — currently computed
   via aggregate over `account_balances_current`. At mainnet scale
   this will likely require denormalized counters on `assets`. Task
   0135 (FEATURE_token-holder-count-tracking) handles this out of
   band.

3. **SEP-0050 NFT detection false positives** — `nfts` is currently
   polluted by fungible transfers misclassified as NFT events. Task
   0118 (BUG_nft-false-positives-fungible-transfers) handles this.
   Not a schema-shape concern.

4. **Fee-bump role rendering** — `transactions.inner_tx_hash IS NOT
NULL` signals fee-bump. The `feeSource` and full signer list
   require S3 fetch for the detail view (aligned with ADR 0018
   S3-offload decision).

These items are recorded for traceability. None is a gap in the
18-table schema itself.

---

## Decision

Accept the 18-table schema (ADR 0011–0020 consolidated) as the
reference surface for the pre-GA build. Verified via:

- 22/22 endpoints covered.
- 14/14 frontend pages covered.
- S3 dependency confined to transaction detail, token metadata, event
  detail expansion — all consistent with ADR 0011/0018 offload
  principle.
- No silent gaps surfaced by this walk.

Further schema changes are expected to be additive (e.g. `abi JSONB`
on `wasm_interface_metadata`, denormalized counters on `assets`) and
do not invalidate the structure documented here.

---

## References

- [backend-overview.md](../../docs/architecture/backend/backend-overview.md) — §6 endpoint inventory
- [frontend-overview.md](../../docs/architecture/frontend/frontend-overview.md) — §6 pages and §7 shared UI
- [ADR 0011: S3 offload model](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0018: Minimal transactions and operations; token_transfers removed](0018_minimal-transactions-detail-to-s3.md)
- [ADR 0019: Schema snapshot and sizing at 11M ledgers](0019_schema-snapshot-and-sizing-11m-ledgers.md)
- [ADR 0020: transaction_participants cut; soroban_contracts index cut](0020_tp-drop-role-and-soroban-contracts-index-cut.md)
