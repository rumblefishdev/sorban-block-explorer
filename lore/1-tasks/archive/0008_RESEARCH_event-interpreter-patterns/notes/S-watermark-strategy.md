---
prefix: S
title: Watermark Strategy Decision
status: mature
spawned_from: null
spawns: []
---

# Watermark Strategy for Event Processing

Synthesis of research into how the Event Interpreter should determine which events to process on each Lambda invocation.

## Sources

- [PostgreSQL INSERT ... ON CONFLICT docs](../sources/postgresql-insert-on-conflict.md) — transaction semantics for atomic watermark + upsert
- [AWS: Making retries safe with idempotent APIs](../sources/aws-idempotent-apis.md) — idempotency patterns for Lambda
- Martin Kleppmann, _Designing Data-Intensive Applications_ (O'Reilly, 2017), Ch. 11: Stream Processing — watermark concepts (book, not archived)
- Tyler Akidau, ["The World Beyond Batch: Streaming 101"](https://www.oreilly.com/radar/the-world-beyond-batch-streaming-101/) — foundational article on watermarks (external link)

## Options Analyzed

### A. Event ID Watermark (track last processed `id`)

Query: `WHERE id > last_processed_id ORDER BY id ASC LIMIT N`

| Pros                                        | Cons                                |
| ------------------------------------------- | ----------------------------------- |
| Deterministic, gap-free with sequential IDs | Out-of-order inserts can cause gaps |
| No clock skew — IDs from database           | Requires single watermark store     |
| Simple to implement                         |                                     |
| Naturally at-least-once on failure          |                                     |

### B. Timestamp Watermark (track last processed `created_at`)

Query: `WHERE created_at > last_watermark`

| Pros                            | Cons                                   |
| ------------------------------- | -------------------------------------- |
| Works across partitioned tables | Clock skew can miss events             |
| Natural time-based alignment    | Late-arriving events silently skipped  |
|                                 | Ties with same timestamp need handling |

### C. Sliding Time Window (relative to `NOW()`)

Query: `WHERE created_at BETWEEN NOW() - '10 min' AND NOW() - '5 min'`

| Pros                                      | Cons                                |
| ----------------------------------------- | ----------------------------------- |
| Self-healing (overlap catches stragglers) | Requires idempotent writes          |
| No persistent state                       | Lambda delays can miss events       |
|                                           | Harder to reason about exactly-once |

### D. Hybrid: Event ID + Lookback Buffer

Query: `WHERE id > (last_processed_id - buffer)`

| Pros                       | Cons                     |
| -------------------------- | ------------------------ |
| Determinism + self-healing | More complex             |
|                            | Buffer size needs tuning |

## Decision: Event ID Watermark

**Selected: Option A — Event ID Watermark**, for these reasons:

1. The `soroban_events` table is populated by a single ingestion pipeline (Ledger Processor) that we control. Monotonic ID assignment is guaranteed by PostgreSQL `BIGSERIAL` (project schema uses `id BIGSERIAL PRIMARY KEY`).
2. Lambda running every 5 minutes is a simple batch loop — ID-based pagination is the textbook pattern.
3. No clock skew concerns — IDs are database-assigned.
4. Natural at-least-once semantics: if Lambda fails mid-batch, watermark is not advanced, next invocation reprocesses from the same point.

### Implementation

Store watermark in a dedicated table:

```sql
CREATE TABLE processing_watermarks (
  processor_name  TEXT PRIMARY KEY,
  last_processed_id BIGINT NOT NULL,
  last_processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Initial row
INSERT INTO processing_watermarks (processor_name, last_processed_id)
VALUES ('event_interpreter', 0);
```

### Processing loop (pseudo-code)

```typescript
async function processEvents() {
  const BATCH_SIZE = 1000;

  // 1. Read current watermark
  const { lastProcessedId } = await db.query(
    `SELECT last_processed_id FROM processing_watermarks
     WHERE processor_name = 'event_interpreter'`
  );

  // 2. Fetch next batch
  const events = await db.query(
    `SELECT * FROM soroban_events
     WHERE id > $1 ORDER BY id ASC LIMIT $2`,
    [lastProcessedId, BATCH_SIZE]
  );

  if (events.length === 0) return;

  // 3. Interpret events
  const interpretations = events
    .map((e) => registry.interpret(e))
    .filter(Boolean);

  // 4. Upsert interpretations + advance watermark (SINGLE TRANSACTION)
  await db.transaction(async (tx) => {
    await tx.query(upsertInterpretations(interpretations));
    await tx.query(
      `UPDATE processing_watermarks
       SET last_processed_id = $1, last_processed_at = NOW()
       WHERE processor_name = 'event_interpreter'`,
      [events[events.length - 1].id]
    );
  });
}
```

### Critical correctness property

The watermark update and interpretation upserts are in **the same transaction**. If the transaction rolls back, the watermark is not advanced — the next invocation reprocesses from the same point. This gives exactly-once semantics at the database level.
