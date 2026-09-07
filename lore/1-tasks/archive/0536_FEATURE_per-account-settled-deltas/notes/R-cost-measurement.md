---
prefix: R
title: What per-account deltas cost — measured across three epochs
status: mature
---

# R — Cost measurement (2026-09-03)

## Why three epochs

Four earlier estimates in the same session gave **250 GB, then 45 GB, then
26 GB, then 80–110 GB**. All four multiplied an average taken from a single
epoch, and all four were wrong, because the multiplier is not stable:

| Epoch (200-ledger sample) | Triples from operations | From Soroban events | After removing zeros | Pairs   | Multiplier |
| ------------------------- | ----------------------- | ------------------- | -------------------- | ------- | ---------- |
| 60,000,000 — older        | 120,759                 | 223,295             | 314,465              | 196,724 | **1.60**   |
| 63,000,000 — middle       | 103,925                 | 301,400             | 370,467              | 123,059 | **3.01**   |
| 64,249,000 — today        | 33,701                  | 143,613             | 162,065              | 188,817 | **0.86**   |

**A 3.5× swing between epochs.** Today's traffic is diluted by KALE farming
(contract calls that move no token); the middle epoch was dominated by Soroban
transfers. Any single-epoch sample is worthless for this question.

Weighted multiplier across the three: **1.67** → 11.6 bn pairs × 1.67 =
**19.3 bn rows**.

## What was filtered out, and how

A "triple" is (transaction, asset, account). Not every touched triple needs a
stored row — a row is only worth writing when the delta is non-zero.

**Step 1 — in ClickHouse.** Dropped failed transactions and payments to self:

| Epoch      | Before  | After   | Drop     |
| ---------- | ------- | ------- | -------- |
| 60,000,000 | 194,969 | 120,759 | −38%     |
| 63,000,000 | 137,995 | 103,925 | −25%     |
| 64,249,000 | 80,848  | 33,701  | **−58%** |

**Step 2 — pass-through accounts, from raw XDR.** These cannot be filtered in
the database, because `operations_appearances.amount` is **not an amount** — it
holds `agg.count`, the number of aggregated operations of the same shape
(`stage.rs`, the `OperationAppearanceRow` construction). The column name is
misleading and cost an hour of this analysis.

So 24 value-moving transactions were fetched from Soroban RPC, decoded with
`stellar xdr decode`, and their balance deltas computed independently:

```
touched triples          : 362
triples with non-zero    : 331
share genuinely moving   : 91.4%
```

Pass-throughs and other zeros are only **8.6%** — less than feared.

## Per-column costs

Taken from `lp_operation_amounts`, an existing production table with the same
shape (subject + location + asset + signed `Int128`), 948 M rows:

| Column                         | B/row    |
| ------------------------------ | -------- |
| `transaction_id`               | **4.98** |
| `amount` (signed `Int128`)     | 4.82     |
| `ledger_sequence`              | 2.58     |
| `application_order`            | 0.30     |
| leading key column (`pool_id`) | 0.17     |
| `asset_id`                     | 0.06     |

`account_id` leading a key measures **0.092 B/row** in
`transaction_participants` — the same order as `pool_id` above.

## Result

|                       | Rows    | Size       |
| --------------------- | ------- | ---------- |
| Natural key           | 19.3 bn | **153 GB** |
| With `transaction_id` | 19.3 bn | 249 GB     |

Difference: **96 GB**, which is what [[0538]] is worth on this one table alone.

## The one remaining assumption

Sides per Soroban token event was set to **1.7** (a transfer has two, a mint or
burn has one) rather than measured, because it would require decoding event
payloads rather than counting them.

| Sides   | Rows        | Size       |
| ------- | ----------- | ---------- |
| 1.0     | 13.6 bn     | 108 GB     |
| 1.5     | 17.7 bn     | 140 GB     |
| **1.7** | **19.3 bn** | **153 GB** |
| 2.0     | 21.8 bn     | 173 GB     |

Honest range: **110–175 GB**. Decoding the events would close it, but it does
not change the order of magnitude or the decision.

## Why the two-column alternative was rejected

Storing `Σ+` and `Σ−` per (transaction, asset) costs ~300 MB — 500× less. It
was rejected on measurement: across the same 24 decoded transactions,

- `Σ+` **equals** `Σ−` in **42 of 64** asset-transactions (65.6%);
- they differ in 22 (34.4%), and there the pattern is mint/burn
  (`KALE +6,984,583 / −0`), not transfers between accounts.

An asset that moves has to land somewhere, so for ordinary transfers both sums
are the same number and the second column carries no information. It would
answer "was this issuance or destruction", not "did this account receive".

**Caveat on that 34.4%:** part of it is a blind spot in the reducer rather than
genuine one-sidedness — when an asset enters a Soroban AMM pool, the outflow is
visible but the pool's own contract-storage balance may not be. This does not
change the verdict on the two-column option, but it is a defect to fix before
any backfill of the per-account table.
