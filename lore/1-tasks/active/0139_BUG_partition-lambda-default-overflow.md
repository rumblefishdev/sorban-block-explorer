---
id: '0139'
title: 'Partition Lambda fails when indexer outpaces range creation (default overflow)'
type: BUG
status: active
related_adr: ['0012']
related_tasks: ['0022', '0131', '0140']
tags: [priority-high, effort-small, layer-infra, pending-adr-0012-review]
milestone: 1
links:
  - crates/db-partition-mgmt/src/main.rs
  - infra/src/lib/stacks/partition-stack.ts
history:
  - date: '2026-04-15'
    status: backlog
    who: stkrolikiewicz
    note: >
      Hit in staging deploy on 2026-04-15. operations_default accumulated 1212 rows
      in range 10M-20M because indexer crossed operations_p0 boundary between
      Lambda triggers. Fixed manually: created operations_p1 standalone, moved rows
      from default, attached as partition. Deploy then succeeded.
  - date: '2026-04-15'
    status: active
    who: stkrolikiewicz
    note: 'Activated. Scope: daily cron + Lambda self-heal + alarm fix, all in one task.'
  - date: '2026-04-17'
    status: active
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 affects referenced schema/flow (see body
      for OLD patterns). This task is NOT hard-blocked by the migration (logic
      is schema-adjacent, not schema-gated). Verify target tables/flow against
      ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (flow, schema, upsert, partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes the schema and ingestion flow but this task is not hard-blocked by the migration — verify target table/column/flow references against ADR 0012 before implementing.

---

# Partition Lambda fails when indexer outpaces range creation

## Summary

`db-partition-mgmt` Lambda fails with "updated partition constraint for default
partition would be violated by some row" when indexer writes `transaction_id`
values beyond the current highest range partition between Lambda triggers.
Rows land in `operations_default`, then subsequent `CREATE TABLE ... PARTITION OF
operations FOR VALUES FROM ... TO ...` is blocked by Postgres because those
rows would belong in the new partition.

## Context

Lambda triggers:

- CDK custom resource (every deploy)
- EventBridge cron (1st of month, 02:00)

Logic in `operations_ranges_to_create` (`crates/db-partition-mgmt/src/main.rs:180`):

```rust
if usage_percent > 80.0 || max_transaction_id >= highest_range_end {
```

The 80% threshold assumes the next trigger fires before indexer reaches 100%.
This assumption breaks when:

- Gap between triggers is long (monthly cron + infrequent deploys)
- Indexer backfills a large batch and crosses the boundary quickly

The fallback `max_id >= highest_range_end` fires only AFTER rows already landed
in default, so the Lambda tries to create the partition and Postgres rejects it.

The CloudWatch alarm `OperationsRangeHigh` (`partition-stack.ts:144`) has the
same blind spot — it only updates when the Lambda runs, and
`treatMissingData: NOT_BREACHING` suppresses it between runs.

## Incident (2026-04-15, staging)

- `operations_p0` covered 0–10M
- `MAX(transactions.id) = 12_162_606`
- `operations_default` contained 1212 rows with `transaction_id` in [10_085_665, 12_162_606]
- Deploy of `Explorer-staging-Partition` failed
- Stack ended in `UPDATE_ROLLBACK_FAILED`; needed `continue-update-rollback`
- Manual fix: `CREATE TABLE operations_p1 (LIKE operations INCLUDING ALL)`,
  moved rows via `DELETE ... RETURNING` + `INSERT`, then
  `ALTER TABLE operations ATTACH PARTITION operations_p1 FOR VALUES FROM (10_000_000) TO (20_000_000)`.

## Implementation

Recommended combo:

1. **Daily cron instead of monthly** — change
   `infra/src/lib/stacks/partition-stack.ts:107-113` to
   `events.Schedule.cron({ minute: '0', hour: '2' })`. Shrinks trigger gap 30x.

2. **Self-heal in Lambda** — when `CREATE TABLE ... PARTITION OF` fails with
   SQLSTATE for "partition constraint violation by default" (check exact code),
   fall back to:

   - `CREATE TABLE <name> (LIKE operations INCLUDING ALL)` (standalone)
   - `WITH moved AS (DELETE FROM operations_default WHERE <range> RETURNING *)
INSERT INTO <name> SELECT * FROM moved`
   - `ALTER TABLE operations ATTACH PARTITION <name> FOR VALUES FROM ... TO ...`
     All in a single transaction.

3. **Alarm fix** — switch `OperationsRangeHigh` to a metric based on actual
   row-level usage queried directly against RDS (CloudWatch custom metric from
   a separate cheap cron), or use `treatMissingData: BREACHING` so missing
   data signals the Lambda itself is broken.

Optional:

- Proactive Lambda invocation from indexer when approaching range end.
- Larger partition size (postpones problem; doesn't fix root cause).

## Acceptance Criteria

- [ ] Cron frequency increased (daily or more)
- [ ] Lambda self-heals when default contains rows belonging to a new range
- [ ] Regression test in `db-partition-mgmt` covering the default-overflow path
- [ ] `OperationsRangeHigh` alarm meaningfully detects overflow between deploys
- [ ] Manual SQL runbook retained in task 0131 or this task for future incidents
