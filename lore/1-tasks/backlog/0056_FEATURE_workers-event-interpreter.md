---
id: '0056'
title: 'Workers: Event Interpreter Lambda'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0008', '0018']
tags: [priority-medium, effort-medium, layer-indexing]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Workers: Event Interpreter Lambda

## Summary

Implement a scheduled Lambda worker that runs every 5 minutes via EventBridge, reads recently stored Soroban events from the database, identifies known patterns (swap, transfer, mint, burn), and writes human-readable interpretations to the event_interpretations table. This worker is completely independent from the Ledger Processor and operates as a post-processing enrichment step.

## Status: Backlog

**Current state:** Not started. Research task 0008 (event interpreter patterns) provides foundational knowledge. Database schema task 0018 defines the soroban_events and event_interpretations tables.

## Context

The Event Interpreter is a secondary worker, not part of the primary ingestion pipeline. It runs on a timer, reads from the database (not S3 or XDR), and produces human-readable enrichment that improves the explorer experience.

Key architectural boundaries:

- **Completely independent from the Ledger Processor**: does not share code paths, triggers, or data flow with the ingestion pipeline
- **Reads from DB, not S3/XDR**: queries the soroban_events table for recently stored events
- **Deployed separately**: lives in `apps/workers`, not `apps/indexer`
- **Runs on timer, not ledger trigger**: EventBridge rate(5 minutes), not S3 event

### Source Code Location

- `apps/workers/src/event-interpreter/`

## Implementation Plan

### Step 1: High-Water Mark Management

Maintain a persisted high-water mark to track which events have already been processed:

- Store the high-water mark as ledger_sequence or event_id in a dedicated state table or configuration row
- On each invocation, query soroban_events with id > last_processed_id (or ledger_sequence > last_processed_sequence)
- Support a configurable lookback window (e.g., 10 minutes) as a safety net for events that might have been missed
- Update the high-water mark after successful processing

### Step 2: Event Pattern Recognition

Query soroban_events since the last run and identify known patterns based on event topics and contract conventions:

**Swap events:**

- Detect token swap patterns from DEX contract events
- Extract: source token, destination token, amounts, accounts involved

**Transfer events:**

- Detect token transfer patterns (SEP-41 standard)
- Extract: from account, to account, token, amount

**Mint events:**

- Detect token minting patterns
- Extract: to account, token, amount, minter

**Burn events:**

- Detect token burning patterns
- Extract: from account, token, amount

Pattern recognition uses the event's contractId, topics (ScVal array), and data (ScVal) to match known signatures.

### Step 3: Interpretation Writing

For each identified pattern, write to the `event_interpretations` table:

- `event_id` FK: references the source soroban_events row (ON DELETE CASCADE)
- `interpretation_type`: one of 'swap', 'transfer', 'mint', 'burn' (extensible)
- `human_readable` TEXT: a human-readable description (e.g., "Transferred 100 USDC from GABC... to GDEF...")
- `structured_data` JSONB: normalized interpretation payload with typed fields for programmatic consumption

### Step 4: Idempotency

The interpreter must be idempotent:

- Skip events that already have an existing interpretation for the same (event_id, interpretation_type)
- Or use UPSERT on (event_id, interpretation_type) to safely reprocess
- Multiple invocations over the same time window must not create duplicate interpretations

### Step 5: CASCADE Awareness

`event_interpretations` has ON DELETE CASCADE from `soroban_events`. When a soroban_event is deleted (e.g., via partition drop or parent transaction cleanup), associated interpretations are automatically cleaned up. The interpreter does not need to manage deletion.

## Acceptance Criteria

- [ ] Lambda is triggered by EventBridge every 5 minutes
- [ ] High-water mark tracks last processed event, persisted across invocations
- [ ] Configurable lookback window provides safety net for missed events
- [ ] Swap events are correctly identified and interpreted
- [ ] Transfer events are correctly identified and interpreted
- [ ] Mint events are correctly identified and interpreted
- [ ] Burn events are correctly identified and interpreted
- [ ] event_interpretations rows include event_id FK, interpretation_type, human_readable text, and structured_data JSONB
- [ ] Processing is idempotent: reprocessing the same events does not create duplicates
- [ ] ON DELETE CASCADE from soroban_events cleans up interpretations automatically
- [ ] Worker runs independently from the Ledger Processor with no shared state
- [ ] Unit tests cover each pattern recognition path and idempotency
- [ ] Integration test verifies end-to-end: events in DB -> interpreter run -> interpretations in DB

## Notes

- The Event Interpreter is deployed as a separate Lambda in `apps/workers`, not in `apps/indexer`. This separation keeps enrichment logic decoupled from core ingestion.
- EventBridge trigger configuration and retry/DLQ setup are defined in CDK task 0037.
- New event patterns can be added incrementally without changing the core pipeline. The interpretation_type column and structured_data JSONB are designed for extensibility.
- The 5-minute cadence means interpretations are not real-time. They appear shortly after events are ingested. This is acceptable for the explorer's enrichment layer.
- If the interpreter falls behind (e.g., during initial backfill when many events arrive), the lookback window and high-water mark ensure it catches up over subsequent runs.
