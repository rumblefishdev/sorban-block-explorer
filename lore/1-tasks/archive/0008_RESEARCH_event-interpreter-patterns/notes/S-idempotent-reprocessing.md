---
prefix: S
title: Idempotent Reprocessing Decision
status: mature
spawned_from: null
spawns: []
---

# Idempotent Reprocessing: Upsert vs Skip

Synthesis of research into how the Event Interpreter should handle re-encountering events that already have interpretations.

## Sources

- [PostgreSQL INSERT ... ON CONFLICT docs](../sources/postgresql-insert-on-conflict.md) — DO UPDATE vs DO NOTHING semantics, EXCLUDED pseudo-table
- [AWS: Making retries safe with idempotent APIs](../sources/aws-idempotent-apis.md) — ClientToken pattern, ACID idempotency
- [Brandur Leach: Building Robust Systems with Idempotency Keys](../sources/brandur-idempotency-keys.md) — atomic phases, recovery points

## Options

| Aspect            | `ON CONFLICT DO UPDATE` (Upsert)                    | `ON CONFLICT DO NOTHING` (Skip)     |
| ----------------- | --------------------------------------------------- | ----------------------------------- |
| Behavior          | Overwrites existing interpretation                  | Silently skips if exists            |
| Pattern evolution | Reprocessing produces updated results               | Old interpretations never corrected |
| Performance       | Slightly more expensive (row lock + new MVCC tuple) | Cheaper (no lock, no write)         |
| Dead tuples       | Generates dead tuples on re-upsert (needs VACUUM)   | No dead tuples for skipped rows     |
| Idempotency       | Fully idempotent                                    | Fully idempotent                    |
| Use case          | Interpretations may need correction                 | Interpretations are immutable       |

## Decision: Upsert (ON CONFLICT DO UPDATE)

**Selected: Upsert semantics**, because:

1. **Pattern handlers will evolve.** When a bug is fixed in the swap detector or new fields are added, reprocessing should correct old interpretations.
2. **Volume is low enough** (~7,000-10,000 events per 5-min window at normal load) that MVCC overhead is negligible.
3. **Forward-compatible** — no manual cleanup needed when patterns improve.

## Implementation

### Schema addition: `pattern_version` column

Add a `pattern_version` column to track which version of the pattern handler produced the interpretation:

```sql
ALTER TABLE event_interpretations ADD COLUMN
  pattern_version INTEGER NOT NULL DEFAULT 1;

-- Unique constraint for conflict target
ALTER TABLE event_interpretations ADD CONSTRAINT
  uq_event_interpretations_event_id UNIQUE (event_id);
```

### Upsert query

```sql
INSERT INTO event_interpretations
  (event_id, interpretation_type, human_readable, structured_data, pattern_version)
SELECT * FROM unnest($1::bigint[], $2::text[], $3::text[], $4::jsonb[], $5::int[])
ON CONFLICT (event_id) DO UPDATE SET
  interpretation_type = EXCLUDED.interpretation_type,
  human_readable = EXCLUDED.human_readable,
  structured_data = EXCLUDED.structured_data,
  pattern_version = EXCLUDED.pattern_version,
  updated_at = NOW();
```

### Selective reprocessing

When patterns are updated, selectively reprocess only events interpreted by an older version:

```sql
-- Find events that need reprocessing
SELECT e.* FROM soroban_events e
JOIN event_interpretations ei ON ei.event_id = e.id
WHERE ei.pattern_version < 2;  -- current version
```

This avoids reprocessing the entire event history — only events affected by pattern changes.

### Batch processing with upserts

Use `unnest` arrays or multi-row `VALUES` for efficient batched upserts. A single multi-row `INSERT ... ON CONFLICT` with 6,000 rows completes in well under a second on PostgreSQL.

### Transaction atomicity

Upserts are wrapped in the **same transaction** as the watermark update (see S-watermark-strategy.md). If anything fails, everything rolls back.
