---
prefix: R
title: Per-column storage cost, measured on production
status: mature
---

# R — Per-column cost on production (2026-09-03)

Source: `system.parts_columns` and `system.parts`, active parts only, via `chq`.
Database total at the time: **1.16 TiB / 61.01 billion rows**. Disk:
**458.87 GiB free of 1.72 TiB**, with `/backups/` on the same volume.

## Every column above 8 GiB

| Table                       | Column          | GiB       | Ratio | B/row |
| --------------------------- | --------------- | --------- | ----- | ----- |
| soroban_events              | topics_xdr      | 154.2     | 13.01 | 15.93 |
| transaction_hash_index      | hash            | **149.8** | 1.00  | 32.13 |
| transactions                | hash            | **123.4** | 1.00  | 32.13 |
| operation_asset_appearances | transaction_id  | **86.9**  | 1.00  | 8.03  |
| transaction_participants    | transaction_id  | **80.4**  | 1.00  | 8.03  |
| soroban_events              | transaction_id  | 49.8      | 1.56  | 5.14  |
| operations_appearances      | transaction_id  | 32.5      | 1.57  | 5.08  |
| transactions                | id              | 30.9      | 1.00  | 8.03  |
| transaction_participants    | ledger_sequence | 28.2      | 2.84  | 2.82  |
| transactions                | inner_tx_hash   | 28.1      | 4.51  | 7.31  |
| text_log                    | message         | 24.5      | 5.23  | 20.30 |
| transactions                | source_id       | 23.5      | 1.31  | 6.12  |
| operations_appearances      | destination_id  | 21.4      | 2.69  | 3.35  |
| transaction_hash_index      | ledger_sequence | 20.5      | 1.82  | 4.40  |
| soroban_events              | data_xdr        | 16.7      | 35.14 | 1.73  |
| operations_appearances      | source_id       | 14.7      | 3.92  | 2.30  |
| operation_asset_appearances | ledger_sequence | 12.1      | 7.16  | 1.12  |
| query_log                   | ProfileEvents   | 8.1       | 3.15  | 73.95 |

`topics_xdr` and `data_xdr` are already `CODEC(ZSTD(3))` and compress 13× and
35× — they are large because the payload is large, not because of waste. They
are **not** candidates.

## `transaction_id` across the schema

| Table                           | GiB      | Ratio | Rows        |
| ------------------------------- | -------- | ----- | ----------- |
| operation_asset_appearances     | 86.9     | 1.00  | 11.6 bn     |
| transaction_participants        | 80.4     | 1.00  | 10.7 bn     |
| soroban_events                  | 49.8     | 1.56  | 10.4 bn     |
| operations_appearances          | 32.5     | 1.57  | 6.9 bn      |
| soroban_invocations_appearances | 8.0      | 1.00  | 1.1 bn      |
| operation_pools                 | 4.7      | 1.00  | 0.6 bn      |
| lp_operation_amounts            | 4.4      | 1.61  | 0.9 bn      |
| **total**                       | **~267** |       | **42.3 bn** |

## Why the ratio differs

Sort keys, read from `system.tables`:

```
operation_asset_appearances   asset_id, ledger_sequence, transaction_id
transaction_participants      account_id, ledger_sequence, transaction_id
operations_appearances        ledger_sequence, transaction_id, application_order
soroban_events                contract_id, ledger_sequence, transaction_id, event_index
```

Where the surrogate is effectively unique per row it compresses at **1.00**.
Where several rows share a transaction (`operations_appearances`,
`soroban_events` — many operations/events per transaction) run-length coding
gets ~1.57. The ceiling is low either way: a hash has no exploitable structure.

The same effect governs the cheap columns. `ledger_sequence` costs
**0.061 B/row** leading `transactions`' key and **2.82 B/row** sitting second
behind `account_id` in `transaction_participants` — identical data, 46×
difference. **Compression is a property of position, not of the column.**

## The natural key

`(ledger_sequence, application_order)` on a 322,240-transaction sample
(ledgers 64,249,000–64,250,000):

```
rows 322240 | distinct (ledger, app_order) 322240 | distinct id 322240
```

Unique, no collisions. `max(application_order) = 100` (also the 99.9th
percentile), so two bytes suffice; it is `Int16` today.

Combined cost **0.135 B/row** (0.061 + 0.074) versus **8.03 B/row** for the
surrogate — **59× cheaper**. And `ledger_sequence` already exists in every
candidate table as the partition key, so the marginal addition is
`application_order` alone. Two tables (`operations_appearances`,
`lp_operation_amounts`) already carry both.

## Caveat on any projected saving

These per-column costs are measured; a projected saving is not. Moving an
identifier changes the sort key, which changes every other column's
compression in that table — the 46× swing above is the warning. Any number
for "what we would save" has to come from a trial rebuild of one table, not
from multiplying these figures.
