---
id: '0541'
title: 'FEATURE: soroban_event_ops — operation attribution for events, as a narrow side table'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0453', '0457', '0540', '0182']
tags:
  [
    'clickhouse',
    'indexer',
    'xdr-parsing',
    'phase-future',
    'effort-medium',
    'priority-medium',
  ]
links:
  - crates/db-clickhouse/schema/init.sql
  - crates/xdr-parser/src/event.rs
history:
  - date: 2026-09-04
    status: backlog
    who: karolkow
    note: >
      Filed from [[0540]]. `soroban_events` does not store which operation
      emitted an event, although the data exists across the whole ingested
      range — measured, not assumed: 1 265 of 1 265 archive transactions carry
      `TransactionMeta::V4` at protocols 20, 22 and 27, and all 1 770
      non-diagnostic token events carry an operation index. Three tasks already
      pay for the gap. Rides 0540's S3 pass.
  - date: 2026-09-06
    status: backlog
    who: karolkow
    note: >
      Parser (`event_pos_in_op`), `SorobanEventOpRow`, staging
      (`persist/value_flow.rs`), the `soroban_event_ops` DDL and the `--only`
      targeted write all landed on 0540's branch. Left here: the backfill run
      itself (0540 rollout step 6), coverage proof, and retiring 0453's
      read-time decode.
---

# soroban_event_ops

## Summary

Store which operation emitted each Soroban event. First as a **narrow side
table** written by 0540's S3 pass (the only additive write available), then
folded into two columns on `soroban_events` itself — see "Target shape" below
(decided 2026-09-07, reversing the "never as a column" stance this task was
filed with).

## Context

`xdr_parser` already computes the operation index: `extract_events` sets
`ExtractedEvent.op_index` from the CAP-67 V4 per-operation container.
`stage.rs` then drops it when building `SorobanEventRow`. The column simply
does not exist in ClickHouse.

Three consumers pay for that today:

| Task               | What it does instead                                                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| **0453**           | Built a read-time "micro-backend": decodes `op_index` from archive XDR on **every** transaction-detail render, exposed as `XdrEventDto.op_index` |
| **0457** (Effects) | Will need the same attribution, for every event and not only token verbs                                                                         |
| **0540**           | Needs an S3 pass rather than a ClickHouse-local transform                                                                                        |

## Why a side table, not a column (original reasoning — superseded by "Target shape")

The obvious fix — `ALTER TABLE soroban_events ADD COLUMN op_index` — looked
wrong here, for the reason [[0540]] hit first:

- `soroban_events` is **10.4 bn rows / 223 GiB**, a version-less
  `ReplacingMergeTree`. Filling a new column means re-inserting **whole rows**,
  which then compete with the existing ones on the same sort key. Which row
  survives a merge is not controlled. This is the documented failure that made
  the 0383 backfill unsafe to re-run after the `net_settled` column landed.
- Rebuilding the table and swapping it (`EXCHANGE TABLES`, the house pattern)
  needs **both copies on disk**: ~446 GiB against 459 GiB free.

A side table avoids both: nothing competes, nothing is rewritten.

```
soroban_event_ops(
    ledger_sequence    Int64,
    application_order  Int16,    -- tx position in the ledger → transactions.id → soroban_events
    event_index        Int16,
    op_index           Int16,    -- envelope position of the emitting operation
    event_pos_in_op    Int16     -- position within that operation's container
)
```

Keyed by the transaction's position, not its id (see "Why now" for the
measurement: the id was 92% of the row). The join to `soroban_events` goes
through `transactions` on `(ledger_sequence, application_order)`, the same hop
`asset_transfers` makes.

Together, `(op_index, event_pos_in_op)` is Stellar's **official** event identity
(TOID plus position within the operation), which 0540 measured as total for
token verbs. It is **not** total for `soroban_events` as a whole — tx-level (fee)
and diagnostic events have no operation — so those rows are simply absent from
this table rather than carrying a null. Absence is the honest encoding: the
question "which operation emitted the fee charge" has no answer.

## Why now

0540's S3 re-parse decodes every event of every ledger anyway. Writing this
table on the same pass costs the extra inserts and nothing else. Done
separately it costs a second ~1-day pass over ~1 TB of XDR.

**Size, measured 2026-09-07** on 39.5 M local events with the final DDL: the
first DDL keyed the table like `soroban_events` (`ledger_sequence,
transaction_id, event_index`) and cost **5.07 B/row — 4.66 of them the
`transaction_id`**, a random hash that does not compress; the two payload
columns cost 0.24. Re-keyed by the transaction's position
(`ledger_sequence, application_order, event_index`, the join going through
`transactions` as `asset_transfers` does) it is **0.63 B/row → ~3.6 GB** on
~5.7 bn rows, instead of ~29 GB. The same two columns inside `soroban_events`
would cost 0.24 B/row (ZSTD) — ~2.5 GB on 10.4 bn rows — which is the target
shape above.

`event_pos_in_op` needed a one-line parser change (the per-operation loop in
`event.rs` did not `enumerate()` the events within the operation) — landed on
0540's branch.

## Target shape — a column on `soroban_events`, the side table as the vehicle (decided 2026-09-07)

The deep review of 0540 asked the principled question: where does the
operation index of an event belong? Canonically **on the event**. stellar-rpc
identifies an event by the cursor `(ledger, tx, op, event)` and, since
Protocol 23, returns the operation index as an attribute of each event in
`getEvents` (per the RPC docs — verify the field name when implementing). A
separate table keyed like `soroban_events` is the right data in the wrong
place; it exists only because filling a column on a 10.4 bn-row table looked
like a rewrite.

It is not. The reasoning above ("re-inserting whole rows", "`EXCHANGE TABLES`
needs both copies") misses ClickHouse mutations: `ALTER TABLE … ADD COLUMN` is
metadata-only and instant, and `ALTER TABLE … UPDATE col = …` rewrites **only
the mutated column's files** — every other column of the part is hard-linked
into the new part. Filling two `Int16` columns over 10.4 bn rows costs one
narrow column-write on the box, no S3, no second copy of the table.

So the side table is kept for 0540's pass — it is what the S3 pass can write
additively today, and it is the **source** for the fold — and the target is:

1. `ALTER TABLE soroban_events ADD COLUMN op_index Nullable(Int16), ADD COLUMN
event_pos_in_op Nullable(Int16)` — instant; NULL = "not yet folded, or a
   tx-level / diagnostic event" (the two must be told apart by the fold's
   completion, not by the value).
2. Live indexer writes both from the deploy on (`SorobanEventRow` gains two
   fields — same deploy-window rule as any struct change, driver validates
   against `DESCRIBE`).
3. History: one mutation **per partition**, sourced from `soroban_event_ops`
   joined to `transactions` on `(ledger_sequence, application_order)` to
   recover `transaction_id`, loaded into a `Join`-engine table for that
   partition (~200 M rows, ~5 GB in memory — fits; 125 GB box), `WHERE` on
   the partition key so each mutation touches one partition's parts.
4. Coverage gate: per partition, `countIf(op_index IS NULL)` on
   `soroban_events` equals the partition's tx-level + diagnostic event count.
5. `DROP TABLE soroban_event_ops`; 0453/0457 read the columns.

Why not do the column now: the live path change and the ALTER need their own
deploy window, and 0540's window is already carrying `DROP COLUMN
net_settled` plus three new tables. One schema change per window (0310).

## Implementation Plan

1. **Parser** — capture the position within the operation (one `enumerate()`).
2. **Row + staging** — new row type; extend the targeted-write mode 0540 adds so
   one pass writes both new tables and touches nothing else.
3. **Table** — create by hand on prod (`init.sql` is fresh-install only).
4. **Backfill** — rides 0540's pass.
5. **Consumers** — retire 0453's read-time decode; hand 0457 the join.

## Acceptance Criteria

- [ ] `soroban_event_ops` created, keyed like `soroban_events`
- [ ] Written by the same pass as 0540 — no second re-parse
- [ ] `soroban_events` is **not** re-inserted; the two columns are added by
      `ALTER` and filled by per-partition mutations from the side table (see
      "Target shape"), then the side table is dropped
- [ ] Coverage proven against the source: for a sampled range, every per-op
      event in the archive meta has a row, and no row exists for a tx-level or
      diagnostic event
- [ ] 0453's transaction-detail render reads the table instead of decoding XDR,
      and the micro-backend is removed
- [ ] **Docs updated** — `docs/architecture/database-schema/**` and
      `xdr-parsing/**` per ADR 0032
