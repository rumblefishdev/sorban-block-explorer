---
id: '0035'
title: 'Drop `account_balance_history` — unused denormalisation, defer chart feature design'
status: accepted
deciders: [fmazur]
related_tasks: ['0159']
related_adrs: ['0012', '0020', '0021', '0027', '0029']
tags: [schema, size-reduction, write-path, drop-unused, denormalisation]
links: []
history:
  - date: '2026-04-23'
    status: proposed
    who: fmazur
    note: >
      Drafted alongside task 0159. Decision made jointly with senior review
      after balance-stage audit during task 0158: `account_balance_history`
      has zero production consumers and is pure write-amplification for an
      unscheduled "balance over time chart" feature. Collapse to
      `account_balances_current` only; defer historical-snapshot design
      to feature launch time.
  - date: '2026-04-23'
    status: accepted
    who: fmazur
    note: >
      Implemented under task 0159. Re-benched on the same 100-ledger
      sample (62016000..62016099) used as task 0158 baseline:
      `balances_ms` mean dropped from ~38 ms to 15.47 ms per ledger
      (−22.5 ms, median 15 ms, p95 25 ms, min/max 9/31 ms). Exceeds the
      task 0159 target of −10 to −20 ms. Total persist mean moved from
      ~200 ms to 192 ms (bench noise absorbs part of the headline delta).
      Table, indexes, partitions, and write-path helpers removed;
      `account_balances_current` unchanged, 22,600 rows populated on the
      bench ledger. ADR 0021 no longer references the table; ADR 0027 §18
      carries a superseded-by-0035 marker.
---

# ADR 0035: Drop `account_balance_history` — unused denormalisation, defer chart feature design

**Related:**

- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md) — originally introduced `account_balance_history` as a partitioned audit log
- [ADR 0020: transaction_participants cut + soroban_contracts index cut](0020_tp-drop-role-and-soroban-contracts-index-cut.md) — projected `account_balance_history` at ~90 GB at 11M-ledger scale (5th-largest table)
- [ADR 0021: Schema ↔ endpoint ↔ frontend coverage matrix](0021_schema-endpoint-frontend-coverage-matrix.md) — coverage matrix; this ADR removes row 18
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md) — `account_balance_history` section (§18) superseded
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) — pattern precedent for "move heavy derivable data off DB"
- [Task 0159: Drop `account_balance_history`](../1-tasks/archive/0159_REFACTOR_drop-account-balance-history.md)

---

## Context

`account_balance_history` was added in ADR 0012 as a partitioned audit log
of every balance snapshot per (account, asset, ledger). ADR 0027 §18
confirmed its shape in the post-surrogate schema. ADR 0020 projected it at
~90 GB at 11M-ledger scale (5th-largest table after `transactions`,
`operations`, `soroban_events`, `transaction_participants`).

Audit during task 0158 surfaced three findings:

### 1. Zero production consumers

Grep across `crates/api`, the ADR 0021 matrix, and all backend code
confirms:

- No endpoint reads from this table.
- Only non-write references are: integration-test count assertions and
  the partition-management table list.
- ADR 0021 line 391 notes the table "supports a future 'balance over
  time' chart" — an explicit future-tense marker. That chart feature
  is not in spec, not in backlog as an active task, and has no wired
  query pattern.

### 2. Write-path cost is real

Per task 0158 benchmark (100 recent ledgers):

- `balances_ms` averages ~38 ms/ledger — 19% of total persist time.
- Breakdown: 5 sub-queries per ledger (trustline DELETE, current-native
  UPSERT, current-credit UPSERT, history-native INSERT, history-credit
  INSERT).
- History-append (14c-N + 14c-C) accounts for an estimated 10–20 ms of
  the 38 ms, with 2 index updates per row (partial uniques).
- At ~724 history rows/ledger average in the sample, scaling to ~8
  billion rows at 11M ledgers (linear) or ~2–4 billion at more realistic
  blended rates.

### 3. `account_balances_current` is the authoritative projection

Empirical verification during task 0158:

```
current.last_updated_ledger ≡ MAX(history.ledger_sequence) for same (account, asset)
```

Confirmed across 10 sampled accounts with ≥7 history rows each: all 10
match exactly. The relation holds modulo two documented edge cases
(trustline removals where `current` DELETEs but `history` retains the
last pre-removal snapshot; out-of-order replay where `current` blocks on
watermark while `history` upserts a lower-ledger row). Both are contained
— `current` remains correct for live-state queries regardless of
`history`'s shape.

The `current` table handles E6 (account balances display) and E8 (token
`total_supply` + `holder_count` aggregates) with PK / partial-unique
lookups. No read-path needs `history` today.

### 4. The chart feature design is premature

Sketching a "balance over time" chart rightly needs chosen design
parameters (granularity, zoom range, retention, expected query
volume). Without those, picking a storage shape now is guessing:

- Per-ledger full snapshots (status quo) — max precision, max cost
- Appearance index + values from XDR replay — cheap storage, expensive reads
- S3 snapshot artifacts — reintroduces ADR 0029-rejected parsed-artifact pattern
- Daily/hourly bucketed snapshots — matches typical UI granularity

Freezing one choice now without load data biases the future decision.
Dropping the table closes the current footprint and reopens the design
space clean.

---

## Decision

1. **Drop `account_balance_history` entirely.** Remove the table from
   migration `0007_account_balances.sql` (rewrite-in-place per project
   convention), along with its two partial unique indexes
   `uidx_abh_native` and `uidx_abh_credit`.

2. **Keep `account_balances_current` unchanged.** Live balance state
   continues to be served by this table exactly as before. Write-path
   stages 14a (trustline DELETE) and 14b (current UPSERT) are untouched.

3. **Remove all write-path code** for balance history:

   - `append_balance_history*` functions in `crates/indexer/src/handler/persist/write.rs`
   - `balance_history_rows` field in `Staged` (staging.rs) + its
     clone-derivation from `balance_rows`
   - `AccountBalanceHistory` domain type in `crates/domain/src/balance.rs`

4. **Remove `"account_balance_history"`** from
   `crates/db-partition-mgmt/src/lib.rs::TIME_PARTITIONED_TABLES` and
   `crates/backfill-bench/src/main.rs` default-partition list.

5. **Update related ADRs:**

   - ADR 0021: remove schema-table row 18; remove the "future chart"
     reference at line 391; consolidate balance coverage to
     `account_balances_current` only.
   - ADR 0027: add superseded-by-0035 marker on §18.

6. **Defer chart-feature design.** If a "balance over time" endpoint
   enters backlog, its design task re-examines the storage shape
   against measured query patterns, picking among:

   - Re-introducing a snapshot table (potentially narrower columns,
     sparser cadence)
   - Appearance index (analogous to ADR 0033/0034) + read-time
     value derivation from XDR
   - S3 snapshot artifacts (only if the ADR 0029 read-time-XDR path
     proves too expensive for the specific chart UX)
   - Cumulative delta log + periodic anchors

   Historical backfill when feature ships: re-ingest the target
   ledger range (idempotent via `ON CONFLICT DO NOTHING` on all
   ingest tables) or targeted XDR replay.

---

## Rationale

### Primary: write-only dead load

The table writes ~10–20 ms/ledger for ~724 rows, scaling to ~2–8
billion rows and 90 GB–1.1 TB at full 11M-ledger scale. Nobody reads
it. Every byte and every index update is waste.

### `account_balances_current` is already the authoritative projection

Empirical proof above: `current` equals the latest-per-trio projection
of `history` for every tested case. Keeping both is a denormalisation
whose read-side benefit is zero (no consumer uses history); the
write-side cost is the full 14c stage.

### Collapsing avoids divergence risk

Two-tables-for-same-data designs always carry drift risk on edge paths
(trustline removals, out-of-order replay, schema evolution). Single
source of truth is structurally safer.

### Chart feature design deserves real data

Committing to per-ledger full snapshots locks us into the most
expensive shape. The feature doesn't exist; projecting storage against
unknown query patterns guesses wrong.

### Reversible at feature launch

`account_balance_history` can be rebuilt from the ingest pipeline by
re-processing any ledger range (idempotent). The schema can be added
back with targeted design. No data is lost that can't be reconstructed.

---

## Alternatives Considered

### Alt 1: Keep populating, drop only `balance` column (appearance pattern)

**Description:** Analog to ADR 0033/0034. Convert history to appearance
index: `(account, asset, ledger, amount)` where `amount` = balance-change
event count for the trio. Drop `balance NUMERIC(28,7)` column; per-ledger
balance values materialise from XDR replay at read time.

**Pros:** Reuses the ADR 0033/0034 pattern. Cheap storage. Retains
"kiedy się ruszał" activity index.

**Cons:**

- Per-row storage reduction is ~6% (empirical: balance avg 8 B, replaced
  by INTEGER amount 4 B, net −4 B). Not the 70–80% of ADR 0033/0034
  because balance rows are already narrow.
- Chart feature still needs VALUES, which require XDR replay — not
  trivially derivable from Stellar XDR (ledger deltas, not balance
  snapshots per account).
- Keeps the write path alive (~5-8 ms/ledger) for an activity-only
  index that no consumer requests.

**Decision:** REJECTED. Marginal savings for feature-incomplete solution;
drop-entirely is 18× more disk savings at similar effort.

### Alt 2: Move to S3 parsed artifacts

**Description:** Write `parsed_ledger_{N}_balances.json.zst` to S3 at
indexing time; DB keeps only appearances (or nothing).

**Pros:** Decouples hot DB path from write-only history.

**Cons:** Contradicts ADR 0029's explicit rejection of custom parsed
artifacts for events/invocations. Reintroduces artifact lifecycle +
consistency + invalidation complexity. Similar total byte cost just
moved to S3. Chart feature still needs a read-time pipeline to materialise
from snapshots.

**Decision:** REJECTED. Architecture regression.

### Alt 3: Daily-bucket snapshots instead of per-ledger

**Description:** Populate only every ~86,400 ledgers (1 day). Cuts rows
~6000×.

**Pros:** Matches typical chart granularity. Storage within reason.

**Cons:** Designs a specific cadence against unknown UX requirements.
Introduces cadence-selection logic at staging time for a feature that
may want different granularity. Any feature-launch-time redesign
discards or reshapes this work.

**Decision:** REJECTED. Premature commitment; drop-and-redesign-later is
cleaner.

### Alt 4: Status quo

**Description:** Keep writing snapshots; absorb the ~15 ms/ledger + TB
of disk cost as a future-feature investment.

**Pros:** Zero code change today. Chart feature "just works" when wired.

**Cons:** Wastes resources indefinitely. "Future chart" has no timeline
in backlog — could be years. Opportunity cost grows with ledger count.
A less-expensive shape will almost certainly be chosen at feature-design
time anyway.

**Decision:** REJECTED. Status-quo cost compounds with network growth.

---

## Consequences

### Positive

- **Write time: −10 to −20 ms/ledger** (the entire 14c stage disappears).
  Target: ~5–10% throughput improvement on the 100-ledger bench.
- **Disk: −90 GB to −1.1 TB at 11M-ledger scale** (entire table + 2
  partial unique indexes).
- **Code simplified:**
  - `Staged.balance_history_rows` field removed
  - `clone_balance_row` helper removed
  - Three history-append functions removed (~150 LOC)
  - Integration test simplified
- **No functional regression for today's API.** E6 and E8 read from
  `account_balances_current` exclusively, which is unchanged.
- **Design option preserved.** Rebuild is trivial (re-ingest is
  idempotent; ADR 0029's XDR read-time path is also available).

### Negative

- **Historical balance values for pre-feature ledgers are not recoverable
  from DB alone.** Reconstruction options when the chart feature lands:
  (a) replay public-archive XDR (expensive; Stellar XDR is effects-based,
  requires state-machine simulation for exact balance); (b) re-index
  target ledger range from archive if write-path is re-enabled with
  snapshot storage.
- **"When did balance change" queries** are no longer served from a
  dedicated table; reconstruct from `operations` + `transactions` +
  `soroban_events_appearances` per the earlier audit. Acceptable
  because no endpoint uses this today.
- **ADR 0012 historical decision partially reversed.** ADR 0012 added
  the table as "full history for future auditability"; the audit never
  materialised as a read-path. Documented here to avoid confusion in
  future schema reviews.

---

## Open Questions

1. **Measured write-time improvement.** _Resolved._ `balances_ms` dropped
   from ~38 ms to 15.47 ms mean on the task-0158 100-ledger baseline
   (−22.5 ms; median 15 ms, p95 25 ms). Total persist mean moved from
   ~200 ms to 192 ms — some of the headline balances-stage delta is
   absorbed by run-to-run noise in other stages (operations_ms,
   participants_ms), but the `balances_ms` component is unambiguously
   cleaner of the 14c append-history cost.
2. **Measured disk improvement.** _Resolved for sample._ 100-ledger
   sample: `account_balance_history` was ~10 MB (5.4 MB heap + 4.9 MB
   indexes); after drop: 0. ADR 0019 projected it at ~90 GB at 11M-ledger
   scale — that projection falls out of the total-DB footprint (ADR 0019
   midpoint ~1.25 TB → ~1.16 TB after this ADR).
3. **Feature-launch design plan.** When "balance over time" enters
   backlog, the design task draws from:
   - Chart UX granularity (daily / hourly / per-ledger?)
   - Zoom range support (1Y / all-time?)
   - Expected QPS
   - Preferred read latency
     Then picks storage shape accordingly.

---

## References

- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md) — original introduction
- [ADR 0020: transaction_participants cut + soroban_contracts index cut](0020_tp-drop-role-and-soroban-contracts-index-cut.md) — size projection
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md) — §18 superseded
- [ADR 0029: Abandon parsed artifacts, read-time XDR fetch](0029_abandon-parsed-artifacts-read-time-xdr-fetch.md) — read-time XDR pattern precedent
