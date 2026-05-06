---
id: '0192'
title: 'operations_appearances order by oa.id != on-chain apply order'
type: BUG
status: active
related_adr: ['0029', '0033', '0034']
related_tasks: ['0163', '0172']
tags:
  [
    priority-medium,
    effort-small,
    layer-indexer,
    layer-db,
    correctness,
    bug,
    frontend-impact,
  ]
links:
  - docs/architecture/database-schema/endpoint-queries/03_get_transactions_by_hash.sql
  - crates/indexer/src/handler/persist.rs
  - crates/db/migrations/0003_transactions_and_operations.sql
  - lore/1-tasks/archive/0163_REFACTOR_operations-as-appearance-index.md
  - lore/1-tasks/archive/0172_REFACTOR_application-order-1-based-ecosystem-parity.md
history:
  - date: '2026-05-05'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned during /compare-with-stellar-api E03 Statement C (operations)
      verification. Empirical test on tx 1b8c6cb7… (24-op PAYMENT bulk) shows
      DB returns ops sorted alphabetically by asset_code when ORDER BY
      operations_appearances.id ASC, while XDR/Horizon return them in
      on-chain apply order (NVIDIA, ALPHABET, APPLE, MICROSOFT, AMAZON, ...).
      The SQL header in 03_get_transactions_by_hash.sql:112-117 claims
      oa.id ordering "IS the operation application order within a tx" —
      this empirical disagreement contradicts that claim. Task 0163
      dropped the operations_appearances.application_order column on the
      premise no endpoint reads it; this finding suggests endpoint 03
      Statement C implicitly relies on ordering that the schema cannot
      guarantee.
  - date: '2026-05-06'
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Prerequisite to running the 374k-ledger audit-harness
      backfill — fixing this first avoids ingesting millions of rows
      with broken ordering and a second backfill to repair them.
---

# `operations_appearances ORDER BY oa.id` ≠ on-chain apply order

## Summary

Endpoint 03 Statement C ([03_get_transactions_by_hash.sql](../../docs/architecture/database-schema/endpoint-queries/03_get_transactions_by_hash.sql))
returns operation rows in `operations_appearances.id` (BIGSERIAL) ASC
order, with the SQL header claiming this matches on-chain operation
apply order within a tx. Empirically, for a 24-op PAYMENT-bulk tx, the
DB order is **alphabetical by `asset_code`** while XDR/Horizon expose
the **on-chain apply order**. Frontend §6.4 Advanced mode and Normal
mode tree both render ops in the order returned by the API; the current
ordering will mis-display ops as alphabetical instead of apply-order
for any tx where the indexer's INSERT order diverges from envelope
operation order.

## Context

Task 0163 (archived, REFACTOR) collapsed the previous `operations`
table into the lighter `operations_appearances` index, dropping the
`application_order SMALLINT` column "no API endpoint reads" (per its
0163 README §Implementation). The reasoning was sound at the time —
the API simply ordered by `oa.id`, and ingest order was assumed to be
apply order.

Endpoint 03 Statement C SQL header at line 112-117 makes this assumption
explicit:

> Operation ordering: `appearance_id` (oa.id) is a global BIGSERIAL across
> all ledgers/partitions, NOT a within-tx index. The result-set order
> from `ORDER BY oa.id` is monotone with ingest order, which IS the
> operation application order within a tx (operations land sequentially
> during a single ingest). Frontend §6.4 Advanced mode "operation IDs"
> should display row-position within the result set (1..N), not oa.id.

**The "ingest order = apply order" half of the claim is false** for
the test case below.

## Reproduction

DB clone @ snapshot ledger 62046000 (sbe-fresh-postgres-1):

```sql
SELECT
  ROW_NUMBER() OVER (ORDER BY oa.id) AS db_order,
  oa.asset_code
FROM operations_appearances oa
WHERE oa.transaction_id = 10118780  -- tx 1b8c6cb7…
  AND oa.created_at = '2026-04-09 21:54:22+00'::timestamptz
ORDER BY oa.id;
```

| db_order | asset_code |
| -------- | ---------- |
| 1        | ABBVIE     |
| 2        | ALPHABET   |
| 3        | AMAZON     |
| 4        | AMD        |
| 5        | APPLE      |
| 6        | BAC        |
| 7        | BRKB       |
| 8        | BROADCOM   |
| 9        | COSTCO     |
| 10       | EXXONMOBIL |
| 11       | HOMEDEPOT  |
| 12       | JNJ        |
| 13       | JPMORGAN   |
| 14       | MASTERCARD |
| 15       | META       |
| 16       | MICRONTECH |
| 17       | MICROSOFT  |
| 18       | NETFLIX    |
| 19       | NVIDIA     |
| 20       | ORACLE     |
| 21       | PALANTIR   |
| 22       | TESLA      |
| 23       | VISA       |
| 24       | WALMART    |

DB ordering is **strictly alphabetical** by `asset_code`.

XDR ground truth (decoded with py-stellar-sdk 14.0.0 from
`https://horizon.stellar.org/transactions/1b8c6cb7…`'s `envelope_xdr`)
gives `tx.operations[0..14]`:

```
NVIDIA, ALPHABET, APPLE, MICROSOFT, AMAZON, META, BROADCOM, TESLA,
BRKB, WALMART, JPMORGAN, VISA, EXXONMOBIL, JNJ, …
```

Horizon `/transactions/1b8c6cb7…/operations` paginates the same
on-chain order. Both XDR and Horizon agree; DB does not.

The set of (asset_code, dest, value) per op matches on both sides —
only the **ordering** is wrong.

## Hypothesis

The indexer's per-op extraction probably iterates a HashMap/BTreeMap or
groups by asset somewhere along the persist pipeline before INSERTing
into `operations_appearances`. A `BTreeMap<String, …>` keyed by
`asset_code` would produce alphabetical INSERT order. Likely suspects:

- `crates/indexer/src/handler/persist.rs` — UNNEST insert vec ordering
- `crates/xdr-parser/src/operations.rs` (or equivalent) — how it walks
  `tx.operations[]`
- Any `.iter()` over a `HashMap` upstream that gets collected before
  INSERT

## Implementation Plan

### Step 1: confirm root cause

Add a small audit script (or extend `crates/audit-harness/src/bin/`)
that, for a sample of N≥20 multi-op txs from the live DB, compares
`SELECT asset_code FROM operations_appearances WHERE transaction_id=$1
ORDER BY oa.id` against the XDR-decoded `envelope_xdr.tx.operations[].body`
order. Confirm the divergence is systematic (not just one tx).

### Step 2: choose remediation

Three viable paths:

A. **Reinstate `application_order SMALLINT` on operations_appearances**
(revert 0163's drop on this column only). Indexer writes the
per-tx 0/1-based position from XDR walk. Endpoint 03 Statement C
adds `ORDER BY oa.application_order`. Cost: schema migration (online,
add column NULLABLE → backfill from current rows is impossible without
re-parsing every ledger's XDR; new rows from now on have it). For
historical rows, accept ordering is best-effort or run a backfill
that re-walks XDR per partition (heavy).

B. **Fix indexer to INSERT in apply order**, so `oa.id` BIGSERIAL is
monotone with apply order. Cheaper if the divergence is from a
single iteration site; costly to backfill historical rows (they
already have the wrong oa.id).

C. **Hybrid**: do both — fix forward ingest to be apply-order
(option B) AND add `application_order` column for explicit ordering
contract (option A). New rows naturally satisfy both; historical
rows correct on `application_order` after backfill, oa.id stays
incorrect for old data but is no longer load-bearing.

Owner's call. **A** has clearest semantics and aligns with 0172's
re-establishment of `transactions.application_order`. **C** is most
robust long-term.

### Step 3: update endpoint 03 SQL header

Drop the false claim about `oa.id` matching apply order. Replace with
a definitive ordering source (whatever the chosen remediation provides).

### Step 4: update 0163 archive note

Add a postscript referencing this task — 0163's premise ("no API
endpoint reads `application_order`") was correct at the time, but the
shape of endpoint 03 Statement C re-introduced an implicit ordering
dependency.

## Acceptance Criteria

- [ ] Root cause identified — pinpoint the indexer site that produces
      alphabetical INSERT order.
- [ ] Audit script confirms divergence is systematic (≥20 multi-op
      sample txs) or local to specific op patterns.
- [ ] Chosen remediation (A / B / C) implemented.
- [ ] Endpoint 03 Statement C returns ops in on-chain apply order
      verified against XDR for at least 5 multi-op test txs.
- [ ] SQL header in `03_get_transactions_by_hash.sql:112-117`
      updated to reflect the actual ordering contract.
- [ ] **Docs updated** — `docs/architecture/database-schema/**`
      ordering contract for `operations_appearances`. Per ADR 0032.

## Notes

- Source DB during discovery: clone of `sbe-audit-postgres-1`
  snapshotted at ledger 62046000 (~10.1M tx, 28 sample ops across 5
  hashes, 24-op PAYMENT bulk as the smoking gun).
- All 11 DB→ field VALUES in Statement C verified MATCH against
  XDR + Horizon; only the ROW ORDERING is wrong.
- This finding does NOT affect endpoint 02 (transactions list) —
  that uses keyset on `(created_at, id)` of transactions, not
  operations_appearances.
- Frontend §6.4 ("operation IDs should display row-position within
  the result set") will produce a misleading numbering until this
  is fixed: row 12 will show as "Operation 12" in the UI even though
  on-chain it was operation 14.
