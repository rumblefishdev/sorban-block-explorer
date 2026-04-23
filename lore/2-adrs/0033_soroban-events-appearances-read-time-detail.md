---
id: '0033'
title: 'soroban_events → soroban_events_appearances: 4-column appearance index, all event detail at read time from S3'
status: accepted
deciders: [fmazur]
related_tasks: ['0157']
related_adrs: ['0021', '0027', '0029', '0030', '0031']
tags: [architecture, schema, read-path, s3, db-size]
links: []
history:
  - date: '2026-04-22'
    status: proposed
    who: fmazur
    note: >
      Drafted alongside task 0157. Reshapes soroban_events into a pure
      appearance index (contract, transaction, ledger, count) and extends
      ADR 0029's read-time XDR fetch to cover every event-bearing endpoint
      (E3, E10, E14).
  - date: '2026-04-23'
    status: accepted
    who: fmazur
    note: >
      Schema + write-path implemented (task 0157 kroki 1-4). `soroban_events`
      replaced in place by `soroban_events_appearances`; indexer write path
      rewritten as `(contract, tx, ledger)` aggregate with amount=count;
      DTO/merge layer stripped of removed columns. Read-path wire-up (E3,
      E10, E14 handlers) deferred — the API crate has no router/state/error
      infrastructure yet, so handler work belongs in a follow-up task once
      the API bootstrap lands. ADR §Open Question 4 (measured row count /
      size) still open pending an indexer run on a representative sample.
---

# ADR 0033: soroban_events → soroban_events_appearances (read-time event detail from S3)

**Related:**

- [ADR 0021: Schema–endpoint–frontend coverage matrix](0021_schema-endpoint-frontend-coverage-matrix.md) — coverage rows for E3/E10/E14 shift to "DB + S3"
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md) — `soroban_events` section superseded for this table only
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) — pattern extended here from E3/E14 to every event-bearing endpoint
- [ADR 0030: Contracts surrogate BIGINT id](0030_contracts-surrogate-bigint-id.md) — `contract_id BIGINT` FK preserved
- [ADR 0031: Enum columns (SMALLINT + Rust enum)](0031_enum-columns-smallint-with-rust-enum.md) — `event_type SMALLINT` removed from this table; ADR still stands for other tables
- [Task 0157: Refactor soroban_events → soroban_events_appearances](../1-tasks/active/0157_REFACTOR_soroban-events-appearances-adr-0033.md)

---

## Context

`soroban_events` today stores one row per contract event. Per ADR 0027,
ADR 0030, and ADR 0031 its current column set is:

```
id, transaction_id, contract_id, event_type, topic0, event_index,
transfer_from_id, transfer_to_id, transfer_amount, ledger_sequence,
created_at
```

indexed by `contract`, `transfer_from`, `transfer_to`, partitioned
monthly on `created_at`, with a replay-safe unique on
`(transaction_id, event_index, created_at)`.

At full Soroban-era coverage (~11.6M ledgers, ~1.5k–3k events/ledger)
this is the single largest Soroban-domain table:

| Metric    | Value      |
| --------- | ---------- |
| Rows      | ~330 M     |
| Heap      | ~36 GB     |
| Indexes   | ~18 GB     |
| **Total** | **~54 GB** |

ADR 0029 already established the principle that heavy event fields
(full topics, value XDR, diagnostic events) are fetched at request time
from the public Stellar archive. It kept `topic0 + transfer_*` in the
DB so that list views and token-transaction queries could stay
DB-resident. This ADR removes that compromise.

The user-facing behaviour we want (matching StellarChain): a per-contract
events panel that paginates by ledger — for each page, the DB tells us
_which ledgers_ contributed events and _how many per transaction in that
ledger_; the API then fetches those ledgers' XDR from the public archive
and expands each appearance into the user-visible list. One GetObject
serves every event a contract emitted in that ledger, regardless of
count.

---

## Decision

1. **Rename and reshape the table.**

   ```sql
   CREATE TABLE soroban_events_appearances (
       contract_id     BIGINT       NOT NULL REFERENCES soroban_contracts(id),
       transaction_id  BIGINT       NOT NULL,
       ledger_sequence BIGINT       NOT NULL,
       amount          BIGINT       NOT NULL,
       created_at      TIMESTAMPTZ  NOT NULL,
       PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
       FOREIGN KEY (transaction_id, created_at)
           REFERENCES transactions (id, created_at) ON DELETE CASCADE
   ) PARTITION BY RANGE (created_at);

   CREATE INDEX idx_sea_contract_ledger
       ON soroban_events_appearances (contract_id, ledger_sequence DESC, created_at DESC);
   CREATE INDEX idx_sea_transaction
       ON soroban_events_appearances (transaction_id, created_at DESC);
   ```

   The natural key is `(contract_id, transaction_id, ledger_sequence)`;
   `created_at` participates in the PK only because partition pruning
   requires the partitioning column in the key. The user-facing column
   set is the four columns shown in the screenshot:
   `contract_id, transaction_id, ledger_sequence, amount`.

   `amount` is the count of events in that `(contract, tx, ledger)`
   trio, so a single `.xdr.zst` GET for `ledger_sequence` yields every
   event this row stands for — the appearance row is a pointer, the
   detail is in S3.

2. **Drop the removed columns (event_type, topic0, event_index,
   transfer_from_id, transfer_to_id, transfer_amount, surrogate `id`).**
   Their semantics move to the XDR parsed at read time. The
   `accounts(id)` FKs from transfer_from/to and the surrogate
   `id BIGSERIAL` disappear entirely.

3. **Rewrite migrations in place.** Edit
   `crates/db/migrations/0004_soroban_activity.sql` and the
   `20260421000100_replay_safe_uniques` migration in place; drop
   ADR 0031's `event_type SMALLINT` converter for this table; drop
   ADR 0030's `contract_id` FK rewrite for this table (FK stays, but
   the surrounding shape is simpler). No production database exists,
   so the rewrite-in-place convention already used for ADR 0030 and
   ADR 0031 applies.

4. **Rewrite `insert_events` (indexer write path).** Aggregate parsed
   events by `(contract_id, transaction_id, ledger_sequence)` before
   the insert, emit one row per trio with `amount = count`, and use
   `ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at)
DO NOTHING`. Re-processing the same ledger is a no-op because the
   aggregate is deterministic over a given ledger's content.

5. **Route every event-bearing endpoint through S3 read-time fetch.**

   | Endpoint                           | Old behaviour                                           | New behaviour                                                            |
   | ---------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------ |
   | E3 `GET /transactions/:hash`       | DB list + S3 heavy fields                               | DB appearances + S3 full expansion                                       |
   | E10 `GET /assets/:id/transactions` | DB `DISTINCT` filtered by `transfer_amount IS NOT NULL` | DB appearances for token's contract + S3 filter for transfer-kind events |
   | E14 `GET /contracts/:id/events`    | DB list + optional S3 heavy                             | DB appearances + S3 full expansion                                       |

   All three use the same pattern:

   ```
   API request → DB: page of appearances by (contract_id, ledger_sequence DESC)
              → for each unique ledger in the page, one public S3 GetObject
              → xdr_parser::decompress_zstd + deserialize_batch
              → xdr_parser::extract_events, filtered by
                contract_id (and transaction_id when E3 / E10 need it)
              → expand each appearance into its `amount` events in
                response order
   ```

   E10's "is this a transfer?" classification moves from a DB column
   to a parser-side check on the decoded event topics.

6. **Pagination model matches StellarChain.** Page size applies to
   appearance rows (typically one row per `(contract, tx, ledger)`),
   not to events. Frontend shows "Page N" with a Previous/Next pair;
   cursor is `(ledger_sequence, transaction_id)` descending. Within a
   page, events are expanded in ledger → tx-within-ledger → index
   order.

7. **Caching stays deferred (ADR 0029 §6).** Same rationale: measure
   first. The appearance aggregation already concentrates S3 reads on
   a small set of ledgers per page, which strengthens the case for
   "no cache until it hurts".

8. **Update ADR 0021 coverage matrix.** Rows for E3, E10, E14 move
   from "DB" (or "DB + S3 heavy") to "DB appearances + S3 detail".
   No schema tables lost — only `soroban_events` is reshaped.

---

## Rationale

### Primary: DB-size reduction on the largest Soroban table

Collapsing per-event rows to per-trio aggregate rows is the direct
win. Conservative back-of-envelope: a typical contract emits
several events per invoking transaction, so each appearance row
absorbs an `amount` on the order of 2–10 events (rare pathological
contracts higher). The ~54 GB figure compresses by an order of
magnitude once `topic0 TEXT`, the transfer triple, and surrogate
`id` disappear, and the row count drops by the mean event count per
trio. Concrete numbers will be measured post-migration in the task.

### Completes ADR 0029

ADR 0029 moved heavy event detail to the public archive but left
`topic0 + transfer_*` in the DB because list views and E10
depended on them. Those columns are the last "parsed event detail"
left in the DB. Removing them makes the DB role consistent: DB is
an index, S3 is the source of truth for event content.

### One read pattern for three endpoints

E3, E10, E14 now share the same assembly: DB page → ledger-grouped
S3 fetches → parser → filter → expand. The `crates/xdr-parser`
`extract_events` function already exists (used at ingest and
at read time for E3/E14 heavy fields). The new call site is a
superset of the existing one.

### Per-page S3 fetch count is bounded

Because the DB row is one per `(contract, tx, ledger)` trio, a page
of N appearance rows fetches at most N distinct ledgers, and in
practice fewer (a contract often appears in multiple transactions of
the same ledger). This is the same property StellarChain relies on
to keep pages snappy.

### Aligns with the rest of the DB model

The DB is already an index over ledger/transaction identity (ADRs
0011/0012/0018/0029). `soroban_events_appearances` fits that pattern
cleanly — it joins the same `(transaction_id, created_at)`
composite everyone else uses, and keeps the monthly partition
strategy from ADR 0027.

---

## Alternatives Considered

### Alt 1: Keep current schema, add a read-through S3 cache

**Description:** Leave the 11-column `soroban_events`; address size
by adding lifecycle/archival of old partitions and an S3 cache for
detail expansion.

**Pros:** No migration; no endpoint rewrites.

**Cons:** Doesn't address the root cause — the DB still stores
parsed event detail that's already on the public archive. Cache
reintroduces the artifact-bucket complexity ADR 0029 walked away
from. Lifecycle-only approach caps growth but doesn't shrink.

**Decision:** REJECTED — treats the symptom.

### Alt 2: Partial reduction — drop `topic0` and `transfer_*` only, keep per-event rows

**Description:** Remove the heavy columns but keep one DB row per
event (so `event_index` and surrogate `id` stay).

**Pros:** Smaller migration surface; existing pagination logic
survives.

**Cons:** Row count is unchanged (~330 M). Indexes on
`(contract, created_at)` stay the dominant cost. Misses the
S3-grouping benefit (one GET per ledger serves every appearance row
from that ledger).

**Decision:** REJECTED — solves half the problem.

### Alt 3: Fully eventless DB — derive appearances from `transactions` + S3

**Description:** Drop `soroban_events` entirely; for per-contract
queries, parse the relevant ledger range's XDR on demand.

**Pros:** Minimum schema.

**Cons:** "Which ledgers did contract X appear in?" becomes an
unbounded S3 scan. Breaks pagination latency. An appearance index
is the minimum DB state that makes per-contract queries feasible.

**Decision:** REJECTED — pagination requires a DB index on contract
appearances.

---

## Consequences

### Positive

- **DB size drops.** `soroban_events_appearances` is a 4-column
  (plus `created_at`) aggregate — narrow rows, narrow indexes, many
  fewer rows.
- **One read pattern across E3/E10/E14.** Event-bearing endpoints
  share `DB appearances → S3 ledger fetch → parser filter → expand`.
- **S3 fetches amortise naturally across a page.** A page's rows
  typically collapse to a handful of distinct ledgers; one GET per
  ledger serves every event in that ledger for the contract.
- **Simpler write path.** `insert_events` becomes a pure aggregate
  - UPSERT. No per-event transfer classification, no `event_index`
    bookkeeping, no `event_type` enum mapping inside the insert.
- **No production data to migrate.** Same rewrite-in-place pattern
  already used for ADR 0030 and ADR 0031.

### Negative

- **Every event-bearing endpoint now depends on the public archive.**
  E3 and E14 already did (ADR 0029). E10 newly does. Availability and
  latency of `aws-public-blockchain` become part of the event read
  path across the board. Same mitigations as ADR 0029 (timeouts,
  "detail unavailable" fallback, observability) apply.
- **Transfer-only filters become parser-side.** E10's "only transfers"
  selection is no longer a DB `WHERE`; it's a decode-and-check on
  each ledger's events. Acceptable because the S3 fetch is per-ledger
  regardless.
- **List pagination is coarser.** Page cursor is per appearance row,
  not per event. Users paging deeply through a very high-count
  contract see ledger-grouped pages (matches StellarChain's UX).
- **ADR 0021 coverage matrix needs a revision.** E3/E10/E14 rows
  change category. Bundled into task 0157.
- **Column-level analytics on events disappear.** Any future query
  that would have wanted "count events of type X across all time"
  now requires parsing the archive. No such query is on the roadmap;
  flagged here as a property.

---

## Open Questions

1. **E10 source-account exposure.** E10 historically joined
   `transactions.source_id → accounts.account_id` to return the
   transfer initiator. The join still works (appearance has
   `transaction_id`). Confirm during task 0157 that the response
   shape is preserved.
2. **Page size tuning.** Default page size for E14 and E10 — start
   at 10 (StellarChain's default) and revisit once measurements
   land.
3. **Per-ledger parse budget.** A ledger with many events for one
   contract on one page should parse once, not once per appearance
   row. Task 0157 must memoise the decoded ledger within a single
   request's handler scope.
4. **Row count / size estimate.** Back-of-envelope suggests an
   order-of-magnitude reduction; task 0157 produces the measured
   number post-migration on a representative sample.

---

## References

- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md)
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)
- [Public Stellar ledger archive](https://registry.opendata.aws/stellar-network/)
- StellarChain contract events UX (ledger-grouped pagination) — reference UI for the new page model
