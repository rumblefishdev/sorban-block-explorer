---
status: mature
spawned_from: '0137'
---

# Phase 0: EXPLAIN ANALYZE Results

Benchmark range: 62016000–62016009 (10 ledgers, fresh insert on clean DB).
Run with `EXPLAIN_QUERIES=1` to capture query plans for the first ledger.

## Top 3 queries by cost

### insert_events_batch — 80.8ms (2530 rows)

```
Insert on soroban_events (actual time=43.394..43.395)
  Buffers: shared hit=35522 read=4 dirtied=167 written=165
  ->  Function Scan on unnest (actual time=1.434..3.512 rows=2530)
        Buffers: shared hit=2541

Trigger for constraint soroban_events_contract_id_fkey:      time=8.985  calls=2530
Trigger for constraint soroban_events_transaction_id_fkey:    time=27.714 calls=2530
Execution Time: 80.793 ms
```

| Component                                | Time   | %   |
| ---------------------------------------- | ------ | --- |
| Insert (heap + 5 indexes)                | 43.4ms | 54% |
| FK → transactions (per-row trigger)      | 27.7ms | 34% |
| FK → soroban_contracts (per-row trigger) | 9.0ms  | 11% |
| UNNEST scan                              | 3.5ms  | 4%  |

**5 indexes on soroban_events:**

- PK `(id, created_at)` — B-tree (partitioned)
- UNIQUE `uq_events_tx_index (transaction_id, event_index, created_at)` — B-tree
- GIN `idx_events_topics (topics)` — JSONB
- B-tree `idx_events_contract (contract_id, created_at DESC)`
- B-tree `idx_events_tx (transaction_id)`

### insert_transactions_batch — 48.6ms (307 rows)

```
Insert on transactions (actual time=1.374..44.396 rows=307)
  Buffers: shared hit=4261 read=5 dirtied=140 written=139
  ->  Function Scan on unnest (actual time=1.088..1.502 rows=307)
        Buffers: shared hit=319

Trigger for constraint transactions_ledger_sequence_fkey: time=3.823 calls=307
Execution Time: 48.585 ms
```

| Component                      | Time   | %   |
| ------------------------------ | ------ | --- |
| Insert (heap + indexes)        | 44.4ms | 91% |
| FK → ledgers (per-row trigger) | 3.8ms  | 8%  |
| UNNEST scan                    | 1.5ms  | 3%  |

High per-row cost (0.14ms/row) driven by many columns (14) including
JSONB `operation_tree` and large text fields (`envelope_xdr`, `result_xdr`).

### insert_operations_batch — 26.4ms (774 rows)

```
Insert on operations (actual time=16.375..16.376)
  Buffers: shared hit=9952 read=4 dirtied=90 written=88
  ->  Function Scan on unnest (actual time=0.406..1.052 rows=774)
        Buffers: shared hit=785

Trigger for constraint operations_transaction_id_fkey on operations_p0: time=9.785 calls=774
Execution Time: 26.439 ms
```

| Component                           | Time   | %   |
| ----------------------------------- | ------ | --- |
| Insert (heap + indexes)             | 16.4ms | 62% |
| FK → transactions (per-row trigger) | 9.8ms  | 37% |
| UNNEST scan                         | 1.0ms  | 4%  |

## Key finding: FK triggers are the #1 bottleneck

**Total FK cost per ledger: ~50ms** (36.7 + 9.8 + 3.8).

FK constraints fire per-row triggers that do individual lookups into parent
tables. For 2530 events, that's 5060 lookups (2530 to transactions + 2530 to
soroban_contracts). This is the single largest actionable cost.

| Query               | Total       | FK cost    | FK %    |
| ------------------- | ----------- | ---------- | ------- |
| insert_events       | 80.8ms      | 36.7ms     | 45%     |
| insert_operations   | 26.4ms      | 9.8ms      | 37%     |
| insert_transactions | 48.6ms      | 3.8ms      | 8%      |
| **Total**           | **155.8ms** | **50.3ms** | **32%** |

## Schema constraint issue

All FK constraints are defined as `NOT DEFERRABLE` (the default). This means
`SET CONSTRAINTS ALL DEFERRED` is a no-op — it silently ignores non-deferrable
constraints. To use deferred FK checking, schema must be altered first:

```sql
ALTER TABLE soroban_events ALTER CONSTRAINT soroban_events_transaction_id_fkey DEFERRABLE;
ALTER TABLE soroban_events ALTER CONSTRAINT soroban_events_contract_id_fkey DEFERRABLE;
ALTER TABLE operations ALTER CONSTRAINT operations_transaction_id_fkey DEFERRABLE;
ALTER TABLE transactions ALTER CONSTRAINT transactions_ledger_sequence_fkey DEFERRABLE;
```

These ALTERs are metadata-only (no table rewrite, no index rebuild). Instant
even on large tables.

## Remaining cost breakdown (after FK elimination)

If FK cost is eliminated entirely (~50ms saved), remaining per-ledger cost:

| Query               | Time (no FK) | Driver                        |
| ------------------- | ------------ | ----------------------------- |
| insert_events       | ~44ms        | 5 indexes × 2530 rows         |
| insert_transactions | ~45ms        | 14 columns, JSONB, large text |
| insert_operations   | ~17ms        | 3 indexes × 774 rows          |
| other queries       | ~30ms        | accounts, pools, etc.         |
| commit              | ~10ms        | WAL flush                     |
| **Total**           | **~146ms**   |                               |

To reach <100ms target, would additionally need to address index maintenance
(drop GIN/redundant B-tree indexes during backfill) or reduce data volume
(e.g., skip storing `envelope_xdr`/`result_xdr` during backfill).

## Indexes that could be dropped during backfill

Only needed for API queries, not for insert integrity:

- `idx_events_topics` (GIN on JSONB) — heaviest per-row cost
- `idx_events_contract` (B-tree)
- `idx_events_tx` (B-tree) — overlaps with `uq_events_tx_index`
- `idx_source` on transactions (B-tree)
- `idx_ledger` on transactions (B-tree)

Must keep: PK indexes, UNIQUE constraints (needed for ON CONFLICT DO NOTHING).
