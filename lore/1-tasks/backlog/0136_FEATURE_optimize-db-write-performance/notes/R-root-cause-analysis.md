---
status: mature
spawned_from: '0136'
---

# Root Cause Analysis: DB write latency (450ms/ledger)

Analyzed `persist_ledger` pipeline. 13+ sequential DB roundtrips per ledger,
all within a single transaction. Actual profiling pending (Phase 0 of task).

## Insert ordering (verified safe)

```
1. insert_ledger                          (1 row)
2. insert_transactions_batch              (UNNEST + RETURNING)
3. insert_operations_batch                (UNNEST)
4. ensure_contracts_exist_batch           (UNNEST DO NOTHING)
5. insert_events_batch                    (UNNEST)
6. insert_invocations_batch               (UNNEST)
7. update_operation_trees_batch           (UNNEST UPDATE)
8. upsert_contract_deployments_batch      (UNNEST + COALESCE)
9. upsert_wasm_interface_metadata         (PER-ITEM LOOP)
10. update_contract_interfaces_by_wasm_hash (PER-ITEM LOOP)
11. upsert_account_states_batch            (UNNEST)
12. upsert_liquidity_pools_batch           (UNNEST)
13. insert_liquidity_pool_snapshots_batch   (UNNEST)
14. upsert_tokens_batch                    (UNNEST)
15. upsert_nfts_batch                      (UNNEST)
```

Parent-before-child ordering verified: ledger → transactions → contracts →
operations → events → invocations. FK constraint disabling is safe.

## Bottleneck 1: PostgreSQL config (not tuned for bulk writes)

Default PostgreSQL config prioritizes durability. For re-runnable local backfill:

- `synchronous_commit = off` — don't wait for WAL flush per commit
- `wal_level = minimal` + `max_wal_senders = 0` — reduce WAL volume
- `checkpoint_completion_target = 0.9` — spread checkpoint I/O
- `work_mem = 256MB` — larger sort/hash buffers

**Zero code changes, likely the single biggest win.**

## Bottleneck 2: Single-ledger transactions

1 ledger = 1 transaction = 1 COMMIT. Each COMMIT forces WAL flush (unless
`synchronous_commit = off`). Batching N ledgers per transaction reduces
COMMIT overhead by N×.

Risk: batch failure rolls back all N ledgers. Acceptable for idempotent backfill.

## Bottleneck 3: Transactions no-op UPDATE

`persistence.rs:108`:

```sql
ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash RETURNING hash, id
```

No-op UPDATE triggers WAL writes on every conflict. Fix: split into 2 queries:

```sql
-- Q1: INSERT ON CONFLICT DO NOTHING (no RETURNING)
-- Q2: SELECT hash, id FROM transactions WHERE hash = ANY($1)
```

Q2 returns all ids — both freshly inserted and already existing. No dedup logic
needed. 2 roundtrips in same transaction (~2ms overhead).

## Bottleneck 4: Contract interfaces per-item loop

`persist.rs:179-192` — 2 awaits per interface. 10 interfaces = 20 roundtrips.
Fix: batch into single UNNEST queries.

## Bottleneck 5: GIN index maintenance

3 GIN indexes on JSONB columns:

- `operations.details` (GIN)
- `soroban_events.topics` (GIN)
- `soroban_contracts.search_vector` (TSVECTOR GIN)

Fix for backfill: drop before bulk load, `CREATE INDEX CONCURRENTLY` after.

## Bottleneck 6: Deferred FK constraints

5 FK constraints validated per-INSERT. Fix: `SET CONSTRAINTS ALL DEFERRED` moves
validation to COMMIT time (batch check, not per-row). Safety net preserved.

## Bottleneck 7: Redundant BIGSERIAL + UNIQUE constraints

Events, invocations, operations have `id BIGSERIAL` PK + separate UNIQUE on
business keys = two B-tree indexes per INSERT.

**Verified (2026-04-13):** `id` is never used as FK or queried. Zero references
in API/indexer/domain code. Safe to modify.

**Recommendation for backfill (senior's suggestion):** Keep BIGSERIAL as sole PK
(monotonic, fast append). Drop UNIQUE constraints on business keys during backfill.
Ensure idempotency at application level (check if ledger already processed).
Re-add UNIQUE constraints + validate after backfill.

For long-term schema cleanup (drop BIGSERIAL, promote business keys to PK) —
separate task, not a backfill optimization.

## Bottleneck 8: DEFAULT partition bloat

Events/invocations: only Apr-Jun 2026 partitions. Historical data → DEFAULT
partition → progressively slower unique checks. Depends on task 0130.

## Not explored yet

- **COPY vs INSERT...UNNEST** — COPY is PostgreSQL's fastest bulk load method.
  Would require refactoring persist layer from query-builder to COPY protocol.
  Consider if Phase 1 optimizations are insufficient.
