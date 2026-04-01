---
id: '0007'
title: 'Simplified 2-Lambda architecture — no Event Interpreter'
status: accepted
deciders: [fmazur]
related_tasks: ['0033', '0098']
related_adrs: ['0005']
tags: [architecture, infra, simplicity]
links: []
history:
  - date: 2026-04-01
    status: accepted
    who: stkrolikiewicz
    note: 'ADR created. Decision already implemented in commit 7c961e6.'
---

# ADR 0007: Simplified 2-Lambda architecture — no Event Interpreter

**Related:**

- [Task 0033: CDK Lambda functions + SQS DLQ](../1-tasks/active/0033_FEATURE_cdk-lambda-api-gateway.md)
- [Task 0098: Cleanup Event Interpreter references](../1-tasks/archive/0098_REFACTOR_remove-event-interpreter-refs.md)
- [ADR 0005: Rust-only backend API](0005_rust-only-backend-api.md)

---

## Context

The original architecture specified 3 Lambda functions:

1. **API Lambda** — serves REST endpoints
2. **Indexer Lambda** — processes ledger data from S3
3. **Event Interpreter Lambda** — scheduled every 5 minutes via EventBridge, reads `soroban_events`, writes human-readable interpretations to `event_interpretations` table

The Event Interpreter added significant complexity:

- Separate Lambda with its own deployment, IAM, and monitoring
- EventBridge scheduling rule (5-minute interval)
- Dedicated `event_interpretations` table with CASCADE from `soroban_events`
- LEFT JOINs on every events endpoint (API must handle NULL gracefully)
- Extra columns (`human_readable`, `structured_data`, `interpretation_type`) across response shapes
- Testing and observability for a third Lambda

All of this for a feature that provides "nice to have" human-readable summaries (e.g., "Swapped 100 USDC for 95.2 XLM") — not core explorer functionality.

---

## Decision

Remove the Event Interpreter Lambda entirely. The architecture has exactly 2 Lambdas:

```
Galexie → S3 → Indexer Lambda → PostgreSQL ← API Lambda ← Frontend
```

- No `event_interpretations` table
- No EventBridge scheduling
- No enrichment pipeline
- No `human_readable` fields in API responses

The approach is: **keep infrastructure simple, deliver core value first.**

---

## Rationale

1. **Simplicity over premature enrichment.** The explorer's core value is browsing ledgers, transactions, operations, contracts, invocations, and events. Human-readable interpretations are a presentation layer concern — not a data pipeline concern.

2. **Fewer moving parts = fewer failures.** Each Lambda adds cold start latency, IAM surface area, deployment complexity, and monitoring overhead. Two Lambdas are meaningfully simpler to operate than three.

3. **Enrichment can be added later without architecture changes.** If human-readable summaries are needed in milestone 2, they can be computed:

   - Inline in the Indexer (at ingestion time, zero extra infra)
   - In the API response layer (on-the-fly, no storage needed)
   - As a separate Lambda (if the use case justifies the complexity then)

4. **Reduces scope for milestone 1.** Every table, endpoint, and test that referenced `event_interpretations` was unnecessary work. Removing it shrinks the implementation surface across DB schema, API modules, frontend, and testing.

---

## Consequences

### Positive

- 2 Lambdas instead of 3 — simpler deployment, monitoring, and IAM
- No EventBridge scheduling complexity
- Cleaner API response shapes — no NULL interpretation fields
- Smaller DB schema — no `event_interpretations` table
- Reduced scope across 10+ backlog tasks and 4 architecture docs

### Negative

- No human-readable event summaries in milestone 1
- If enrichment is needed later, the table and pipeline must be built from scratch (though this is straightforward)

---

## References

- Commit 7c961e6: `chore(lore-0033): simplify architecture to 2 Lambdas, remove Event Interpreter`
- Task 0098: Cleanup of all remaining Event Interpreter references
