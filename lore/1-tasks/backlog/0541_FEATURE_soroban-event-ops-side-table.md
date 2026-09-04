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
---

# soroban_event_ops

## Summary

Store which operation emitted each Soroban event, in a **narrow side table**
keyed like `soroban_events` — never as a column added to `soroban_events`
itself.

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

## Why a side table, not a column

The obvious fix — `ALTER TABLE soroban_events ADD COLUMN op_index` — is wrong
here, and the reason is the one [[0540]] hit first:

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
    ledger_sequence  Int64,
    transaction_id   Int64,
    event_index      Int16,
    op_index         Int16,      -- envelope position of the emitting operation
    event_pos_in_op  Int16       -- position within that operation's container
)
```

Same key as `soroban_events`, so the join is a seek. Projected **~10.4 bn rows
at roughly 1 B/row ≈ 10 GB** — an estimate from neighbouring narrow columns,
not a measurement; size it properly before the run.

Together, `(op_index, event_pos_in_op)` is Stellar's **official** event identity
(TOID plus position within the operation), which 0540 measured as total for
token verbs. It is **not** total for `soroban_events` as a whole — tx-level (fee)
and diagnostic events have no operation — so those rows are simply absent from
this table rather than carrying a null. Absence is the honest encoding: the
question "which operation emitted the fee charge" has no answer.

## Why now

0540's S3 re-parse decodes every event of every ledger anyway. Writing this
table on the same pass costs the extra ~10 GB of inserts and nothing else. Done
separately it costs a second ~1-day pass over ~1 TB of XDR.

Note `event_pos_in_op` needs a one-line parser change first: the per-operation
loop in `event.rs` records `op_index` but does not `enumerate()` the events
within the operation, so the position is not currently captured.

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
- [ ] `soroban_events` itself is **not** rewritten, and carries no new column
- [ ] Coverage proven against the source: for a sampled range, every per-op
      event in the archive meta has a row, and no row exists for a tx-level or
      diagnostic event
- [ ] 0453's transaction-detail render reads the table instead of decoding XDR,
      and the micro-backend is removed
- [ ] **Docs updated** — `docs/architecture/database-schema/**` and
      `xdr-parsing/**` per ADR 0032
