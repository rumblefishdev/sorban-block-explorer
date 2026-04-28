---
id: '0039'
title: 'Add five read-path indexes for E02 / E15 / E18 / E02 Statement B endpoint queries'
status: accepted
deciders: [stkrolikiewicz]
related_tasks: ['0132', '0167']
related_adrs: ['0027', '0032', '0037']
tags: [database, schema, indexes, performance, read-path]
links:
  - crates/db/migrations/20260428000100_add_endpoint_query_indexes.up.sql
  - crates/db/migrations/20260428000100_add_endpoint_query_indexes.down.sql
  - docs/architecture/database-schema/endpoint-queries/02_get_transactions_list.sql
  - docs/architecture/database-schema/endpoint-queries/15_get_nfts_list.sql
  - docs/architecture/database-schema/endpoint-queries/18_get_liquidity_pools_list.sql
history:
  - date: '2026-04-28'
    status: accepted
    who: stkrolikiewicz
    note: >
      ADR drafted post-implementation. Five indexes shipped in branch
      `feat/0132_missing-db-indexes` (PR #137, task 0132) — migration
      `20260428000100_add_endpoint_query_indexes`. Snapshot ADR 0037
      §5/§9/§10/§12/§14 records the pre-0039 index sets; this ADR is
      the "thin follow-up" 0037 §533 explicitly invites for small
      schema deltas, matching the 0038 pattern.
---

# ADR 0039: Add five read-path indexes for E02 / E15 / E18 / E02 Statement B endpoint queries

**Related:**

- [ADR 0027: Hybrid partitioned schema](0027_hybrid-partitioned-schema.md) — partition strategy that shapes how `CONCURRENTLY` interacts with `transactions` / `*_appearances`
- [ADR 0032: Docs architecture evergreen maintenance](0032_docs-architecture-evergreen-maintenance.md) — required updates to `docs/architecture/**` follow this ADR
- [ADR 0037: Current schema snapshot](0037_current-schema-snapshot.md) — schema snapshot prior to this addition (anchor migration `20260424000000`); §5/§9/§10/§12/§14 record the pre-0039 index sets
- [Task 0132: DB — add missing indexes for planned API query patterns](../1-tasks/active/0132_FEATURE_missing-db-indexes.md)
- [Task 0167: API — hand-tuned SQL query reference set](../1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md) — surfaced the gaps via per-endpoint `EXPLAIN` audit

---

## Context

[Task 0167](../1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md) produced one hand-tuned SQL script per public REST endpoint and ran each under `EXPLAIN` against the live schema in [ADR 0037](0037_current-schema-snapshot.md). Five queries surfaced inline `INDEX GAP:` comments — the planner falls back to seq-scan-then-sort or to an index whose leading column doesn't match the keyset cursor — that the existing index set in 0037 doesn't cover. A subsequent review on PR #136 (task 0172, E02 Statement B variant 2) raised two more contract-leading keyset gaps on the appearance tables.

The original 0132 task body called for `CREATE INDEX CONCURRENTLY` so the migration could land on a populated staging RDS without taking an `AccessExclusiveLock`. Implementation surfaced two operational facts:

1. Postgres forbids `CREATE INDEX CONCURRENTLY` on partitioned parent tables (`transactions`, `soroban_invocations_appearances`, `soroban_events_appearances`). The DDL fails with `cannot create index on partitioned table "..." concurrently`.
2. sqlx's migration runner sends multi-statement scripts via the simple-query protocol, which Postgres wraps in an implicit transaction — and `CREATE INDEX CONCURRENTLY` is forbidden inside any transaction block.

The migration is intended to run **post-restore on staging** before any live traffic is pointed at the DB (see [`lore/3-wiki/backfill-execution-plan.md`](../3-wiki/backfill-execution-plan.md) phase T6). At that point there are no concurrent writers, so the brief `AccessExclusiveLock` on each child partition during plain `CREATE INDEX` is moot. This ADR records the trade-off so a later operator looking at the migration doesn't try to "fix" it back to `CONCURRENTLY`.

---

## Decision

Add five indexes via a single forward-only migration `20260428000100_add_endpoint_query_indexes`:

```sql
-- §5 transactions (partitioned)
CREATE INDEX idx_tx_keyset
    ON transactions (created_at DESC, id DESC);

-- §12 nfts
CREATE INDEX idx_nfts_collection_trgm
    ON nfts USING GIN (collection_name gin_trgm_ops);

-- §14 liquidity_pools
CREATE INDEX idx_pools_created_at_ledger
    ON liquidity_pools (created_at_ledger DESC, pool_id DESC);

-- §10 soroban_invocations_appearances (partitioned)
CREATE INDEX idx_sia_contract_keyset
    ON soroban_invocations_appearances
       (contract_id, created_at DESC, transaction_id DESC);

-- §9 soroban_events_appearances (partitioned)
CREATE INDEX idx_sea_contract_keyset
    ON soroban_events_appearances
       (contract_id, created_at DESC, transaction_id DESC);
```

Each index targets a specific endpoint plan that the `EXPLAIN` audit flagged:

| Index                         | Endpoint                                   | Why the existing set was insufficient                                                                                                                                                                           |
| ----------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `idx_tx_keyset`               | E02 `GET /transactions` no-filter keyset   | Without `(created_at DESC, id DESC)` the planner falls back to per-partition seq-scan + sort.                                                                                                                   |
| `idx_nfts_collection_trgm`    | E15 `GET /nfts` ILIKE on `collection_name` | The existing `idx_nfts_collection` btree only handles exact `=`; the endpoint contract wants ILIKE. The trigram GIN unblocks ILIKE; the existing btree stays as the equality path, complementary not redundant. |
| `idx_pools_created_at_ledger` | E18 `GET /liquidity-pools` keyset          | Pool ordering is `(created_at_ledger DESC, pool_id DESC)`; `liquidity_pools` had no index leading with `created_at_ledger`.                                                                                     |
| `idx_sia_contract_keyset`     | E02 Statement B (variant 2) UNION branch   | Existing `idx_sia_contract_ledger` leads with `ledger_sequence`, mismatched against the `(created_at DESC, transaction_id DESC)` cursor.                                                                        |
| `idx_sea_contract_keyset`     | E02 Statement B (variant 2) UNION branch   | Same shape as above, sibling table.                                                                                                                                                                             |

Plain `CREATE INDEX` (not `CONCURRENTLY`) is used uniformly because three of the five target partitioned parents and Postgres forbids `CONCURRENTLY` there. Mixing `CONCURRENTLY` for the two non-partitioned indexes only would have made the migration file inconsistent without buying anything in the deployment scenario this ADR actually targets (post-restore staging, no live traffic).

The `.up.sql` uses `IF NOT EXISTS` per index so a partial-failure retry is safe; the matching `.down.sql` drops each index in reverse order.

---

## Rationale

The schema already routes deduplication and partition pruning through their respective mechanisms; what's missing is the keyset-shape coverage that the API endpoints actually request:

- **Cursor pagination is keyset-on-`(created_at, id)` for E02** (per [ADR 0025](0025_final-schema-and-endpoint-realizability.md) and the [endpoint-queries reference](../../docs/architecture/database-schema/endpoint-queries/02_get_transactions_list.sql)). Without an index whose leading columns match that cursor, every cold-cursor request from page 2 onwards forces a sort step.
- **Trigram GIN on `nfts.collection_name`** mirrors the existing `idx_nfts_name_trgm` pattern, satisfying the ILIKE requirement E15 inherited from frontend-overview §6 without changing the endpoint contract or removing the equality index.
- **`liquidity_pools.created_at_ledger` is small enough today** that a heap scan + sort is tolerable, but pool count is monotonic and the cost grows linearly. Pre-emptive indexing avoids a future "pool list got slow overnight" diagnostic.
- **Statement B variant 2 of E02** fans out across three appearance tables and keyset-orders the union by `(created_at, transaction_id)`. The existing contract-leading indexes (`idx_sia_contract_ledger`, `idx_sea_contract_ledger`) carry `(contract_id, ledger_sequence)` — semantically close but cursor-shape-wrong. On a popular contract with millions of rows, the sort step is the dominant cost.

Scope is intentionally narrow: only `CREATE INDEX`. No constraint changes, no column additions, no FK changes. Compatible with every code path that wrote to these tables before this ADR (no rows are rewritten; PG simply backfills the new B-tree / GIN at index-build time). Migration is reversible via `.down.sql`.

---

## Alternatives Considered

### Alternative 1: `CREATE INDEX CONCURRENTLY` on every index, including partitioned tables

**Description:** Use `CONCURRENTLY` uniformly so the migration could in principle run against a live DB without long locks.

**Pros:**

- Avoids `AccessExclusiveLock` even if the migration ever runs in front of live traffic.
- Matches a common Postgres ops idiom.

**Cons:**

- Postgres forbids `CONCURRENTLY` on partitioned parent tables — three of five indexes simply can't use it.
- The proper "concurrent-on-partitioned" workflow (`CREATE INDEX … ON ONLY parent`, then per-child `CREATE INDEX CONCURRENTLY` + `ALTER INDEX … ATTACH PARTITION`) requires `1 + N + N` statements per partitioned table per index, where N is the partition count. With ~30 children × 3 partitioned indexes the migration becomes ~180 lines of brittle hand-rolled DDL that must match partition naming exactly.
- The migration target is post-restore staging with zero live traffic. The `AccessExclusiveLock` window the workflow exists to avoid does not exist in the deployment scenario.

**Decision:** REJECTED — operational complexity not justified by a benefit that doesn't materialise in the target scenario. Plain `CREATE INDEX` is simpler, atomic per-statement, and equally safe in the no-traffic window.

### Alternative 2: `CREATE INDEX CONCURRENTLY` only on the two non-partitioned indexes

**Description:** Mixed approach — use `CONCURRENTLY` for `idx_nfts_collection_trgm` and `idx_pools_created_at_ledger`, plain `CREATE INDEX` for the three on partitioned parents.

**Pros:**

- Captures the marginal benefit of `CONCURRENTLY` where Postgres permits it.

**Cons:**

- Forces the migration into multiple files (sqlx wraps multi-statement scripts in an implicit tx, and `CONCURRENTLY` rejects implicit tx). Five separate migrations + `-- no-transaction` directives + a cross-file ordering rule, vs. one self-contained file.
- Inconsistent rationale across the same migration: an operator reading two of the five indexes through `CONCURRENTLY` would reasonably ask "why not the others?" — the answer is "PG forbids it here", but that's not visible at the call site.
- Buys nothing in the no-traffic scenario.

**Decision:** REJECTED — uniformity is worth more than the marginal lock-window saving on two of five indexes.

### Alternative 3: Skip `idx_sia_contract_keyset` / `idx_sea_contract_keyset`; rewrite Statement B to keyset on `ledger_sequence`

**Description:** Two of the five indexes serve a UNION branch in E02 Statement B. The branch could instead keyset on `(contract_id, ledger_sequence DESC)` — which the existing `idx_sia_contract_ledger` / `idx_sea_contract_ledger` already cover — at the cost of introducing a second cursor flavor in the API layer (one for the simple keyset path, one for the `ledger_sequence`-keyed UNION path).

**Pros:**

- Two fewer indexes to maintain.

**Cons:**

- Two API cursor flavors is worse than two indexes. Cursors leak to clients via opaque base64 tokens; a second flavor means a parser branch on every E02 read, doubled test surface, and a future migration trap if the cursor format ever changes.
- The existing indexes' `(contract_id, ledger_sequence)` shape was right for the appearance-as-index access pattern (ADR 0033 / ADR 0034); changing the API cursor to match would have been a workaround for a missing index.

**Decision:** REJECTED — schema absorbs the cost (two indexes), API cursor stays uniform.

---

## Consequences

### Positive

- Every endpoint flagged with an `INDEX GAP:` comment in [`docs/architecture/database-schema/endpoint-queries/`](../../docs/architecture/database-schema/endpoint-queries/) now has a covering index. `EXPLAIN` against the local DB shows the planner walking the new indexes instead of falling back to per-partition seq + sort.
- ADR 0037's snapshot index sets in §5/§9/§10/§12/§14 stay valid as the historical "schema before 0039" reference; this ADR is the official delta.
- ILIKE on `nfts.collection_name` becomes a real plan rather than a sequential scan, satisfying the E15 endpoint contract without changing the endpoint shape.
- Reversible: `.down.sql` drops each index in reverse order. Safe to revert if a future plan regression surfaces.

### Negative

- Each new index costs ~few µs per write on its host table. Five indexes spread across five tables — write throughput impact is small but non-zero. Backfill perf is marginal vs. RDS network I/O, well under a percent of total backfill time.
- Migration takes a brief `AccessExclusiveLock` on each partitioned child during index build. **This ADR's deployment scenario is post-restore staging with no live traffic**, so the lock is invisible to consumers; running this migration in front of live writers without the no-traffic precondition would block writes for the build duration. Operators must respect the precondition.
- ADR 0037's snapshot DDL is now stale wrt the live schema for these five tables. Per the open question carried over from ADR 0038, refreshing 0037 inline vs. keeping it frozen as a fixed-anchor snapshot is still pending @fmazur's call.

---

## Open questions

- **Coordination with ADR 0037**: same question as ADR 0038. 0037 is "current schema snapshot", anchored on migration `20260424000000`. This ADR adds a 14th migration (`20260428000100`) and adds five entries to the index sets recorded at 0037 §5/§9/§10/§12/§14. Per 0037's own §533, "a thin follow-up ADR referencing this one is an acceptable substitute for small deltas" — this ADR is exactly that. Decision deferred to @fmazur whether 0037's body should be refreshed inline (re-anchored to migration `20260428000100`, DDL blocks updated) or remain frozen with this ADR (and 0038) serving as the official deltas. The `related_adrs` list in 0037's frontmatter is updated to include this ADR either way. No code or runtime impact.

---

## Delivery Checklist

Per [ADR 0032](./0032_docs-architecture-evergreen-maintenance.md), any ADR that changes the shape of the system MUST be landed together with the corresponding updates to `docs/architecture/**`. Tick each that applies before marking the ADR `accepted`:

- [ ] `docs/architecture/technical-design-general-overview.md` updated (or N/A) — **N/A — the technical-design overview lists indexes only at the level of "monthly range partitioning on `created_at`" / index categories, not per-index. No drift introduced.**
- [x] `docs/architecture/database-schema/database-schema-overview.md` updated — five indexes added inline next to the existing ones in the corresponding §sections with `-- task 0132 / ...` provenance comments
- [ ] `docs/architecture/backend/backend-overview.md` updated (or N/A) — **N/A — backend reads indexes indirectly via SQL plans; no per-index documentation in this file**
- [ ] `docs/architecture/frontend/frontend-overview.md` updated (or N/A) — **N/A — indexes are DB-internal**
- [ ] `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` updated (or N/A) — **N/A — indexing pipeline writes through the persist layer; index changes don't alter the write path or its idempotency contract**
- [ ] `docs/architecture/infrastructure/infrastructure-overview.md` updated (or N/A) — **N/A — no infrastructure changes**
- [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` updated (or N/A) — **N/A — XDR parsing is upstream of the DB**
- [x] This ADR is linked from each updated doc at the relevant section — link added to `database-schema-overview.md` next to each new index

Additional updates outside `docs/architecture/**`:

- [x] `lore/3-wiki/backfill-execution-plan.md` updated — prerequisite gate row reflects the five indexes and the plain `CREATE INDEX` trade-off
- [x] [ADR 0037](0037_current-schema-snapshot.md) `related_adrs` updated to include `0039` (delta-ADR pattern matching 0038)

---

## References

- Migration: [`20260428000100_add_endpoint_query_indexes.up.sql`](../../crates/db/migrations/20260428000100_add_endpoint_query_indexes.up.sql)
- Endpoint-queries reference set (origin of the `INDEX GAP:` comments): [`docs/architecture/database-schema/endpoint-queries/`](../../docs/architecture/database-schema/endpoint-queries/)
- Task 0167 (per-endpoint EXPLAIN audit): [archive/0167](../1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md)
- PR #137 (this implementation): https://github.com/rumblefishdev/soroban-block-explorer/pull/137
