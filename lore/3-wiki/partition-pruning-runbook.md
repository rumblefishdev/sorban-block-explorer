# Partition Pruning Runbook

Partitions are created automatically. **Dropping partitions is a manual operation** that requires operator review.

## When to Consider Pruning

- Storage pressure on RDS instance (>80% capacity)
- Explicit retention policy change
- Data archived to cold storage

## Pre-Pruning Checklist

- [ ] Confirm no active backfill is running (check the backfill ECS service/task family for running tasks and verify there are no recent backfill log entries in CloudWatch)
- [ ] Confirm no active queries span the partition being dropped (check `pg_stat_activity`)
- [ ] Verify the partition contains only data older than the retention threshold
- [ ] Verify a backup exists (RDS automated backup or manual snapshot)
- [ ] Notify team via Slack before proceeding
- [ ] Confirm you are connected to the correct environment (staging vs production)

## Procedure

### 1. Identify the partition to drop

```sql
-- List all partitions for a table
SELECT c.relname, pg_size_pretty(pg_total_relation_size(c.oid)) AS size
FROM pg_inherits i
JOIN pg_class c ON c.oid = i.inhrelid
JOIN pg_class p ON p.oid = i.inhparent
WHERE p.relname = 'soroban_events'
ORDER BY c.relname;
```

### 2. Detach the partition

```sql
ALTER TABLE soroban_events
    DETACH PARTITION soroban_events_y2024m02 CONCURRENTLY;
```

`CONCURRENTLY` reduces lock duration but does **not** fully eliminate locking — a brief `ACCESS SHARE` lock is still acquired on the parent table. This is normally sub-second but may block momentarily under heavy write load. Schedule during low-traffic periods. The detached table still exists as a standalone table.

### 3. Verify detachment

```sql
-- Should NOT appear in partition list
SELECT c.relname
FROM pg_inherits i
JOIN pg_class c ON c.oid = i.inhrelid
JOIN pg_class p ON p.oid = i.inhparent
WHERE p.relname = 'soroban_events'
  AND c.relname = 'soroban_events_y2024m02';
```

### 4. Drop the detached table

```sql
DROP TABLE soroban_events_y2024m02;
```

### 5. Repeat for other tables in the same month

If pruning a month, drop from all three time-based tables:

- `soroban_invocations_y{YYYY}m{MM}`
- `soroban_events_y{YYYY}m{MM}`
- `liquidity_pool_snapshots_y{YYYY}m{MM}`

## Post-Pruning Verification

- [ ] `SELECT COUNT(*) FROM {parent_table}` — sanity check row count
- [ ] Check CloudWatch `FuturePartitionCount` metric — ensure alarm is not triggered
- [ ] Run a sample query against the parent table — confirm no errors
- [ ] Check application logs for any partition-related errors

## Rollback

If detach was done with `CONCURRENTLY`, you can reattach:

```sql
ALTER TABLE soroban_events
    ATTACH PARTITION soroban_events_y2024m02
    FOR VALUES FROM ('2024-02-01 00:00:00+00') TO ('2024-03-01 00:00:00+00');
```

**After `DROP TABLE`, data is gone.** Restore from RDS backup if needed.

## Operations Table (transaction_id range)

Same procedure but with range-based partition names (`operations_p0`, `operations_p1`, etc.):

```sql
ALTER TABLE operations DETACH PARTITION operations_p0 CONCURRENTLY;
DROP TABLE operations_p0;
```

Only prune ranges that contain exclusively old transactions with no active references.
