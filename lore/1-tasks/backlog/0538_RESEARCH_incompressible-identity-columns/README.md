---
id: '0538'
title: 'RESEARCH: identity columns that do not compress — measure the real cost of the surrogate transaction id and the duplicated hash'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0393', '0417']
tags:
  [
    'clickhouse',
    'storage',
    'performance',
    'research',
    'phase-future',
    'effort-medium',
    'priority-medium',
  ]
links:
  - crates/db-clickhouse/schema/init.sql
history:
  - date: 2026-09-03
    status: backlog
    who: karolkow
    note: >
      Filed after a measurement made while shipping the net-settled column
      (0411/0393). The surrogate `transaction_id` was found to occupy ~267 GiB
      across seven tables at a compression ratio of 1.0, and the 32-byte
      transaction hash is stored twice for a further ~273 GiB. Together that is
      ~45% of a 1.16 TiB database spent on identity alone. The natural key
      `(ledger_sequence, application_order)` was verified unique and costs
      0.135 B/row against 8.03 B/row for the surrogate. Filed as RESEARCH, not
      REFACTOR: the saving is real but every candidate touches a sort key, so
      the question to answer first is which subset is worth the rewrite.
---

# RESEARCH: identity columns that do not compress

## Summary

One class of column dominates this database and carries no information beyond
identity: **hash-derived identifiers**. They are indistinguishable from random
noise, so they compress at ratio 1.0 — every row pays the full 8 or 32 bytes.
Measured on production, they account for roughly **45% of the 1.16 TiB**.

Stellar gives every transaction a natural, dense key —
`(ledger_sequence, application_order)` — that is verifiably unique and ~59×
cheaper. This task measures which tables would actually benefit from switching,
what the migration costs, and whether the same reasoning applies to other
columns.

## Context

The `net_settled` column (0393, shipped to the frontend in 0411) needed a
per-row cost model, which exposed the pattern by accident:

- adding the value column itself cost **292 MiB**;
- storing the same value _per account_ would have cost ~8 B/row **just to
  repeat the transaction id**, dwarfing the payload.

That prompted a database-wide measurement, and the surrogate turned out to be
the single largest structural cost in the schema.

## Measurements (production, 2026-09-03)

Full per-column figures in [notes/R-column-costs.md](notes/R-column-costs.md).
Headline numbers, all **measured**, not estimated:

| Column                                 | Size          | Ratio | B/row     |
| -------------------------------------- | ------------- | ----- | --------- |
| `transaction_id` × 7 tables            | **267 GiB**   | 1.0   | 8.03      |
| `transaction_hash_index.hash`          | **149.8 GiB** | 1.0   | 32.13     |
| `transactions.hash`                    | **123.4 GiB** | 1.0   | 32.13     |
| `transactions.id`                      | 30.9 GiB      | 1.0   | 8.03      |
| `ledger_sequence` (leading a sort key) | 0.25 GiB      | 130.5 | **0.061** |
| `application_order`                    | 0.31 GiB      | 26.9  | **0.074** |
| `account_id` (leading a sort key)      | 0.99 GiB      | 86.6  | **0.092** |

Two facts follow directly:

1. **Position in the sort key decides compression, not the column.**
   `ledger_sequence` costs 0.061 B/row when it leads the key and **2.82 B/row**
   when it sits second behind `account_id` — a 46× swing for the same data.
   Any redesign trades one column's compression for another's.
2. **The natural key is unique.** Verified on a 322,240-transaction sample:
   322,240 distinct `(ledger_sequence, application_order)` pairs, zero
   collisions. `application_order` maxes at 100, so it fits in two bytes.
   `ledger_sequence` is already present in every candidate table because it is
   the partition key — so the marginal cost is one narrow column, not two.

## Open questions this task must answer

1. **Which tables actually pay off?** The surrogate compresses at 1.0 in
   `operation_asset_appearances` and `transaction_participants`, but at ~1.57 in
   `soroban_events` and `operations_appearances`, where it repeats within a
   sort key. The saving is not uniform and the migration cost is.
2. **What does the read path lose?** Every join moves from one column to a
   pair. Measure whether two-column joins on the hot list endpoints stay within
   budget, or whether the CPU cost eats the storage win.
3. **Is the hash genuinely stored twice?** `transactions.hash` and
   `transaction_hash_index.hash` total ~273 GiB. Confirm the index is a
   deliberate reverse lookup and establish whether a cheaper structure (or a
   prefix) serves the same query.
4. **What else is identity-shaped?** `inner_tx_hash` (28.1 GiB),
   `source_id`/`destination_id` (23.5 + 21.4 GiB) — do they share the pattern,
   and does `LowCardinality` or a dictionary help where the value repeats?
5. **Non-schema quick wins.** ClickHouse's own `text_log.message` (24.5 GiB)
   and `query_log.ProfileEvents` (8.1 GiB) are server-side logs. A TTL is a
   configuration change, not a migration — the cheapest ~32 GiB available, and
   worth confirming separately because it needs no code at all.
6. **What is the migration actually worth?** Every candidate sits in a sort
   key, so this is a full rebuild per table plus a re-ingest, not an `ALTER`.
   Quantify against the alternative of applying the natural key **only to new
   tables** (the per-account delta table under discussion is the first
   candidate) and leaving the existing stock alone.

## Constraints

- **Free space is the live constraint.** Production has 458.87 GiB free of
  1.72 TiB, and `/backups/` shares that volume — every GiB counts twice.
- The engine is version-less `ReplacingMergeTree`; a sort-key change cannot be
  `ALTER`ed and forces a rebuild (the same wall 0393 hit; see its notes).
- No migration lands without a read-path benchmark first — the 0243/0386 quota
  outages were both read-shape regressions.

## Acceptance Criteria

- [ ] Per-table verdict: migrate / leave / new-tables-only, each with its
      measured saving and its measured read-path cost
- [ ] Two-column join benchmarked on the hot tx-list endpoints against today's
      single-column join
- [ ] Duplicate-hash question settled: what `transaction_hash_index` is for and
      whether a narrower structure serves it
- [ ] Log TTL quantified and handed over as a standalone config change
- [ ] Recommendation written as an ADR if a schema-wide convention is adopted
      (identity columns use the natural key; surrogates only where measured)
