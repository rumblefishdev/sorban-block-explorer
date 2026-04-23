---
id: '0034'
title: 'soroban_invocations → soroban_invocations_appearances: appearance index with caller_id payload, per-node detail at read time'
status: accepted
deciders: [fmazur]
related_tasks: ['0158']
related_adrs: ['0021', '0027', '0029', '0030', '0033']
tags: [architecture, schema, read-path, s3, db-size]
links: []
history:
  - date: '2026-04-23'
    status: proposed
    who: fmazur
    note: >
      Drafted alongside task 0158 as the table-specific analogue of ADR
      0033. Reshapes soroban_invocations into an appearance index
      (contract, transaction, ledger, caller, amount) and extends ADR
      0029's read-time XDR fetch to cover E11 / E12 / E13.
  - date: '2026-04-23'
    status: accepted
    who: fmazur
    note: >
      Schema + write-path implemented in task 0158. `soroban_invocations`
      replaced in place by `soroban_invocations_appearances`; indexer
      write path rewritten as `(contract, tx, ledger)` aggregate with
      amount=tree-node-count and caller_id preserved as root-caller
      payload. Read-path wire-up (E11 / E12 / E13 handlers) deferred —
      the API crate has no router/state/error infrastructure yet, so
      handler work lands with the API bootstrap that will also pick up
      0157's E3 / E10 / E14 handlers.
---

# ADR 0034: soroban_invocations → soroban_invocations_appearances (read-time per-node detail)

**Related:**

- [ADR 0021: Schema–endpoint–frontend coverage matrix](0021_schema-endpoint-frontend-coverage-matrix.md) — coverage rows for E11 / E12 / E13 shift to "DB appearances + read-time XDR detail"
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md) — `soroban_invocations` section superseded for this table only
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) — pattern extended here from events to invocations
- [ADR 0030: Contracts surrogate BIGINT id](0030_contracts-surrogate-bigint-id.md) — `contract_id BIGINT` FK preserved
- [ADR 0033: soroban_events → soroban_events_appearances](0033_soroban-events-appearances-read-time-detail.md) — direct precedent; this ADR applies the same pattern to the invocation table with one deliberate divergence (caller_id retained as payload)
- [Task 0158: Refactor soroban_invocations → soroban_invocations_appearances](../1-tasks/active/0158_REFACTOR_soroban-invocations-appearances.md)

---

## Context

`soroban_invocations` today stores one row per node in the Soroban
invocation tree. Per ADR 0027 §10 and ADR 0030 its column set is:

```
id, transaction_id, contract_id, caller_id, function_name, successful,
invocation_index, ledger_sequence, created_at
```

indexed by `(contract_id, created_at DESC)` and
`(caller_id, created_at DESC)`, partitioned monthly on `created_at`,
with a replay-safe unique on `(transaction_id, invocation_index,
created_at)`.

Every per-row column except the identity/partitioning columns
(`transaction_id`, `contract_id`, `caller_id`, `ledger_sequence`,
`created_at`) is already produced by `xdr_parser::extract_invocations`
from the transaction envelope plus `SorobanTransactionMeta.return_value`.
The parser output is a strict superset of what the DB currently keeps —
it carries the full tree structure, `function_args`, `return_value`, and
depth — none of which live in the DB today but all of which the
read-path needs for E12 ("expand inner calls") and for per-row E13
rendering (function name + caller per node).

At full Soroban-era coverage this is the second-largest Soroban-domain
table after `soroban_events`. The fixed-width columns (`function_name
VARCHAR(100)`, the SMALLINT/BOOL columns) make the per-row heap cost
smaller than `soroban_events`, but the 1:1 row-per-tree-node cardinality
is the primary growth driver.

This ADR is deliberately shaped as the table-specific analogue of ADR 0033. The overall read-path architecture (DB as a pointer, parser at
read time, S3 as authoritative source of truth) is unchanged; this ADR
documents the two invocation-table-specific trade-offs — `caller_id`
retention and aggregation granularity — and refers to ADR 0033 for the
shared rationale.

---

## Decision

1. **Rename and reshape the table.**

   ```sql
   CREATE TABLE soroban_invocations_appearances (
       contract_id      BIGINT      NOT NULL REFERENCES soroban_contracts(id),
       transaction_id   BIGINT      NOT NULL,
       ledger_sequence  BIGINT      NOT NULL,
       caller_id        BIGINT      REFERENCES accounts(id),
       amount           INTEGER     NOT NULL,
       created_at       TIMESTAMPTZ NOT NULL,
       PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
       FOREIGN KEY (transaction_id, created_at)
           REFERENCES transactions (id, created_at) ON DELETE CASCADE
   ) PARTITION BY RANGE (created_at);

   CREATE INDEX idx_sia_contract_ledger
       ON soroban_invocations_appearances (contract_id, ledger_sequence DESC);
   CREATE INDEX idx_sia_transaction
       ON soroban_invocations_appearances (transaction_id);
   ```

   The natural key is `(contract_id, transaction_id, ledger_sequence)`;
   `created_at` participates in the PK only because partition pruning
   requires the partitioning column in the key. `caller_id` is a payload
   column, **not** part of identity and **not** separately indexed — it
   exists to answer the E11 `unique_callers` stat via
   `COUNT(DISTINCT caller_id)` without extra JOINs.

   `amount` is the count of invocation-tree nodes in that
   `(contract, tx, ledger)` trio, so a single `.xdr.zst` GET for
   `ledger_sequence` yields every node this row stands for — the
   appearance row is a pointer, the per-node detail is in the public
   archive.

2. **Drop the removed columns (function_name, successful,
   invocation_index, surrogate `id`).** Their semantics move to the XDR
   parsed at read time via `xdr_parser::extract_invocations`. The
   `(transaction_id, invocation_index)` replay-safe unique disappears
   with the surrogate; the new PK
   `(contract_id, transaction_id, ledger_sequence, created_at)` is
   itself idempotent under replay.

3. **Keep `caller_id` as payload — deliberate divergence from ADR 0033.**
   The events refactor (ADR 0033) dropped `transfer_from_id` /
   `transfer_to_id` entirely because no endpoint needed a scalar
   aggregate over those columns. For invocations, the spec
   (`docs/architecture/technical-design-general-overview.md:190`,
   `frontend-overview.md:431`, task `0075_FEATURE_frontend-contract-detail`)
   carries `unique_callers` as a visible stat on the contract summary
   card (E11). Without a DB column we would have to either
   (a) drop the stat from the spec, (b) fall back to
   `JOIN transactions.source_id` (semantically different number — counts
   include users who reached the contract only through wrappers,
   because today's staging filter sets `caller_id = NULL` for
   contract-callers), or (c) materialise a separate counter. Keeping
   `caller_id` as a bare payload column preserves the existing
   `COUNT(DISTINCT caller_id)` semantics bit-for-bit and costs one
   nullable BIGINT per appearance row. The column is **not indexed** —
   the contract-scoped scan hits `idx_sia_contract_ledger` and the
   distinct count runs over the already-pruned rows.

   `caller_id` preserves the current staging behaviour: only G-account
   callers (root-level invocations where `caller = tx source`) are
   retained; C-contract callers (sub-invocations) collapse to NULL per
   the existing `is_strkey_account` filter. This matches today's DB
   shape and avoids introducing a second semantic to the stat.

4. **Rewrite migrations in place.** Edit
   `crates/db/migrations/0004_soroban_activity.sql` §10 and drop the
   `soroban_invocations` block from
   `20260421000100_replay_safe_uniques.{up,down}.sql` (the new PK
   covers replay idempotency). No production database exists, so the
   rewrite-in-place convention from ADR 0030, ADR 0031, and ADR 0033
   applies.

5. **Rewrite `insert_invocations` (indexer write path).** Aggregate
   parsed invocations by `(contract_id, transaction_id, ledger_sequence,
created_at)` before the insert, emit one row per trio with
   `amount = tree-node count`, `caller_id = root-level caller of the
trio` (see §6 below), and use `ON CONFLICT
(contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING`.
   Re-processing the same ledger is a no-op because the aggregate is
   deterministic over a given ledger's content.

6. **Aggregation semantics — root-caller per trio.** Each invocation
   tree has one root caller (the tx source for `InvokeHostFunctionOp`);
   sub-invocations carry parent-contract addresses which today's staging
   already filters to NULL. When aggregating tree nodes per
   `(contract, tx, ledger)` trio, `caller_id` is the root caller
   observed for that trio. The edge case "one tx with multiple
   `InvokeHostFunctionOp`s targeting the same contract with different
   root callers" is vanishingly rare (Stellar Protocol 21 allows at
   most one Soroban op per tx in practice); when encountered the
   indexer takes the first-seen root caller and accepts the minor
   semantic drift. This is documented here rather than engineered
   around because the invariant holds in every real-world tx observed
   during task 0157's integration runs.

7. **Route every invocation-bearing endpoint through read-time XDR.**

   | Endpoint                             | Old behaviour                                        | New behaviour                                                              |
   | ------------------------------------ | ---------------------------------------------------- | -------------------------------------------------------------------------- |
   | E11 `GET /contracts/:id`             | DB: `COUNT(*) + COUNT(DISTINCT caller_id)`           | DB: `SUM(amount) + COUNT(DISTINCT caller_id)` over appearances             |
   | E12 `GET /contracts/:id/interface`   | DB: `wasm_interface_metadata` join on contract       | Unchanged — this endpoint never touched `soroban_invocations`              |
   | E13 `GET /contracts/:id/invocations` | DB list of per-node rows (function, caller, success) | DB appearances + S3 XDR fetch → `extract_invocations` → per-node expansion |

   E13 uses the same read pattern ADR 0033 defined for E14:

   ```
   API request → DB: page of appearances by (contract_id, ledger_sequence DESC)
              → for each unique ledger in the page, one public archive GetObject
              → xdr_parser::decompress_zstd + deserialize_batch
              → xdr_parser::extract_invocations, filtered by
                (contract_id, transaction_id)
              → expand each appearance into its `amount` tree nodes in
                response order (depth-first, as the parser emits)
   ```

   E12 is called out explicitly because its row in the ADR 0021 matrix
   references `soroban_contracts`/`wasm_interface_metadata` only — it
   never read `soroban_invocations` and is untouched by this refactor.

8. **Pagination model matches ADR 0033.** Page size applies to
   appearance rows (one per `(contract, tx, ledger)` trio); frontend
   shows Previous/Next with a `(ledger_sequence, transaction_id)`
   descending cursor. Per-node expansion happens inside the page.

9. **Caching stays deferred (ADR 0029 §6).** Same rationale. The
   appearance aggregation concentrates XDR fetches on a small set of
   ledgers per page; the case for "no cache until it hurts" is the same
   as ADR 0033.

10. **Update ADR 0021 coverage matrix.** Rows for E11 (stats side) and
    E13 move from "DB" to "DB appearances + read-time XDR detail". E12
    stays in "DB only" (no change).

---

## Rationale

### Primary: size reduction on the second-largest Soroban table

Per-node rows collapse to per-trio aggregate rows. Conservative
back-of-envelope: typical invocation trees have 1–3 nodes, so each
appearance row absorbs an `amount` on that order. Row count drops by
the mean tree depth per invoking tx; heap cost drops further because
`function_name VARCHAR(100)` disappears from every row. Concrete
numbers measured post-migration in task 0158.

### Completes ADR 0029 for the invocation table

ADR 0029 moved heavy event detail to the public archive. ADR 0033
finished that for events. This ADR closes the same loop for
invocations — the remaining per-node columns (function_name,
successful, invocation_index) were carrying read-path detail the
parser already produces on demand.

### `caller_id` is where this ADR diverges from 0033

ADR 0033 had no analogous constraint: no spec-level stat required a
scalar over `transfer_from_id` / `transfer_to_id`. For invocations,
`unique_callers` is wired into the frontend contract page spec in
multiple places. Removing `caller_id` would either change the number
visibly (JOIN-via-`transactions.source_id` over-counts relative to
today's DB semantics, because today's filter drops contract-callers to
NULL) or require a separate materialisation. A nullable payload column
is the smallest, most reversible way to preserve today's semantics
exactly; the decision is revisitable (`ALTER TABLE DROP COLUMN` is
cheap) if the stat is ever removed from the UI.

### One read pattern across E3/E10/E14 (0157) + E11/E13 (this ADR)

Both refactors converge on the same read-time assembly: DB page →
ledger-grouped archive fetches → parser → filter → expand. The
`crates/xdr-parser` `extract_invocations` function already exists (used
at ingest and available for read-time use with no modifications).

### Per-page archive fetch count is bounded (same as 0033)

Appearance rows are one per `(contract, tx, ledger)` trio; a page of N
rows fetches at most N distinct ledgers, usually fewer when the same
ledger contains multiple invoking transactions.

---

## Alternatives Considered

### Alt 1: Drop `caller_id` entirely (pure analogue of ADR 0033)

**Description:** 4-column appearance table `(contract, tx, ledger,
amount, created_at)` identical to `soroban_events_appearances`.
`unique_callers` resolved via `JOIN transactions` on `source_id` at
read time.

**Pros:** Absolute parity with ADR 0033. Simpler write path. Less
storage.

**Cons:** `COUNT(DISTINCT transactions.source_id)` returns a
semantically different number than today's `COUNT(DISTINCT
caller_id)` — the join counts users reaching the contract through
wrappers (current query drops them because contract-callers are
filtered to NULL). The number visibly changes per-contract. Revisit
cost if "unique callers" needs preservation later: full ledger reindex
or XDR backfill of caller_id column.

**Decision:** REJECTED — the asymmetry of re-adding `caller_id` later
(full reindex) vs. dropping it later (cheap ALTER) favours retention.
Storage cost per row is a single nullable BIGINT, and the column is
unindexed.

### Alt 2: Keep `caller_id` and index it

**Description:** Add `CREATE INDEX idx_sia_caller
ON soroban_invocations_appearances (caller_id, ledger_sequence DESC)`.

**Pros:** Enables hypothetical "list invocations by caller" endpoint.

**Cons:** No such endpoint in ADR 0021. Account page (E6) lists
transactions, not per-contract invocations. Unindexed storage of
`caller_id` is sufficient for the E11 stat because the scan is already
contract-scoped.

**Decision:** REJECTED — speculative. If ever needed, adding the index
later is non-breaking.

### Alt 3: Caller per tree node (PK includes `caller_id`)

**Description:** Aggregate per `(contract, tx, ledger, caller_id)`;
multiple rows per trio when sub-invocations have distinct callers.

**Pros:** Preserves every caller observed across tree depth.

**Cons:** Staging already filters contract-callers to NULL, so the
"distinct sub-callers" information this would capture is already
discarded upstream. Complicates PK (nullable PK component requires
COALESCE-indexed unique). Delivers no new endpoint capability.

**Decision:** REJECTED — over-engineered. The root-caller-per-trio
semantic matches today's filtered-caller behaviour and the E11 stat
question.

---

## Consequences

### Positive

- **DB size drops.** Row count divides by mean tree depth per tx;
  `function_name VARCHAR(100)`, BOOL, SMALLINT columns disappear per
  row. Storage cost of retaining `caller_id` is a nullable BIGINT,
  dominated by the columns removed.
- **`unique_callers` stat preserved bit-for-bit.** E11 query shape
  changes from `FROM soroban_invocations` to `FROM
soroban_invocations_appearances`; the distinct-count semantic stays
  identical because the appearance's `caller_id` matches the filtered
  root-caller the old table stored.
- **Parity with ADR 0033 read path.** E13 uses the same three-step
  assembly as E14 (DB page → ledger-grouped XDR → parser expand).
- **Simpler write path.** `insert_invocations` becomes a pure
  aggregate + UPSERT. No per-row function-name copy, no per-row index
  bookkeeping.
- **No production data to migrate.** Rewrite-in-place pattern from
  ADR 0030 / 0031 / 0033.

### Negative

- **E13 detail depends on public archive.** Same dependency ADR 0033
  introduced for event endpoints. Same mitigations (timeouts, "detail
  unavailable" fallback, observability).
- **Per-row caller rendering latency profile changes.** Old path: DB
  JOIN to accounts, single-digit ms. New path: ledger XDR fetch +
  parse, ms to low seconds depending on archive cache. Matches ADR
  0033's acceptance of this trade-off.
- **Sub-invocation caller info no longer recoverable from DB.** Today's
  staging filter already drops contract-callers to NULL, so this is a
  theoretical loss rather than a regression. The per-node caller chain
  for sub-invocations is available from XDR at read time when E12 /
  E13 render the invocation tree.
- **ADR 0021 coverage matrix needs a revision.** E11 / E12 / E13 rows
  updated; bundled into task 0158.
- **Divergence from ADR 0033 pattern.** Retaining `caller_id` as
  payload breaks the "bare 4-column appearance" shape. Justified above
  and scoped to this table only.

---

## Open Questions

1. **Row count / size estimate.** Back-of-envelope suggests
   factor-of-mean-tree-depth reduction in rows plus per-row shrink
   from dropped VARCHAR / SMALLINT / BOOL. Task 0158 produces the
   measured number post-migration on the same indexer-run sample used
   for ADR 0033's measurement.
2. **Per-ledger parse memoisation.** A ledger with many invocations
   for one contract on one page should parse once, not once per
   appearance row. Task 0158 must memoise the decoded ledger within a
   single request's handler scope — shared concern with ADR 0033's
   Open Question 3.
3. **Interaction with ADR 0033's handler follow-up.** Both ADRs defer
   handler wire-up pending the API crate bootstrap. The follow-up that
   lands E3 / E10 / E14 handlers should also land E11 stats and E13
   list, because all five share the same ledger-XDR-fetch-plus-parse
   machinery.

---

## References

- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md)
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
- [ADR 0033: soroban_events appearances](0033_soroban-events-appearances-read-time-detail.md) — direct precedent
- [Public Stellar ledger archive](https://registry.opendata.aws/stellar-network/)
