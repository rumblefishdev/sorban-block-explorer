---
id: '0008'
title: 'Research: Event Interpreter pattern matching and enrichment approach'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0059']
tags: [priority-medium, effort-medium, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: Event Interpreter pattern matching and enrichment approach

## Summary

Investigate the Event Interpreter's pattern matching strategy, known Soroban event signatures for DEX and token operations, idempotent reprocessing semantics, watermark-based event windowing, and the extensibility model for adding new pattern types. This research must produce a concrete pattern registry design and confirmed event signatures for the initial set of known patterns (swap, transfer, mint, burn).

## Status: Backlog

## Context

The Event Interpreter is a secondary worker in the indexing pipeline. It runs separately from the Ledger Processor and does not perform primary chain ingestion. Instead, it reads recently stored event data from the `soroban_events` table, identifies known patterns, and writes human-readable summaries to the `event_interpretations` table. This keeps enrichment logic separate from the core ledger parse/write path.

### Execution Model

The Event Interpreter runs as a Lambda function triggered every 5 minutes by EventBridge. It is not triggered by individual events -- it processes batches of recent events on a schedule. This means it operates on events that have already been persisted by the Ledger Processor.

### Target Table

The Event Interpreter writes to the `event_interpretations` table:

```sql
CREATE TABLE event_interpretations (
    id                   BIGSERIAL PRIMARY KEY,
    event_id             BIGINT REFERENCES soroban_events(id) ON DELETE CASCADE,
    interpretation_type  VARCHAR(50) NOT NULL,  -- 'swap', 'transfer', 'mint', 'burn'
    human_readable       TEXT NOT NULL,
    structured_data      JSONB NOT NULL
);
```

Each interpretation links to a specific soroban_event via `event_id` FK. The `human_readable` field contains a display-ready text summary (e.g., "Swapped 100 USDC for 95.2 XLM on Soroswap"). The `structured_data` JSONB contains normalized interpretation payloads that are queryable and extensible.

### Known Pattern Types

The initial set of known patterns to support:

- **swap** -- DEX trade events (token A exchanged for token B)
- **transfer** -- token transfer events (from account to account)
- **mint** -- new token creation events
- **burn** -- token destruction events

### Source Event Data

The Event Interpreter reads from `soroban_events`, which contains:

- `event_type` (VARCHAR 20) -- 'contract', 'system', 'diagnostic'
- `topics` (JSONB) -- decoded ScVal array, GIN-indexed
- `data` (JSONB) -- decoded ScVal payload
- `contract_id` (VARCHAR 56) -- the emitting contract
- `created_at` (TIMESTAMPTZ) -- event timestamp, used for partitioning and windowing

### Idempotent Reprocessing

The Event Interpreter must handle reprocessing safely. If it runs again over the same time window, it should either skip events that already have interpretations or upsert to replace existing interpretations. The architecture does not specify which approach, so the research must recommend one.

### Watermark Strategy

The "recent events" window needs a watermark strategy to determine which events to process on each run. Options include: tracking the last processed event ID, tracking the last processed timestamp, or using the `created_at` window relative to the current time. The strategy must handle gaps, retries, and the 5-minute schedule cadence.

## Research Questions

- What are the specific Soroban event signatures (topic structures) for swap events on known DEXes? Research: Soroswap, Phoenix Protocol, Aqua DEX event formats.
- What are the standard token transfer event signatures on Soroban? Is there a common pattern across SAC contracts and custom token contracts?
- What are the mint and burn event signatures? Do they follow a consistent pattern, or do they vary by contract implementation?
- How should the pattern registry be structured in code? A static map of topic patterns to interpretation handlers, a plugin system, or a configuration-driven approach?
- What watermark strategy should be used for determining "recent" events? Event ID watermark, timestamp watermark, or sliding time window?
- Should the Event Interpreter use upsert semantics (replace existing interpretations) or skip-if-exists semantics? What are the implications for correcting interpretations when patterns are updated?
- How should the `human_readable` text be constructed? Template strings with extracted values, or a more sophisticated formatting system?
- How should the `structured_data` JSONB be shaped for each pattern type? What fields are needed for swap (amounts, tokens, DEX), transfer (from, to, amount, token), mint (amount, token, recipient), burn (amount, token)?
- How should the Event Interpreter handle events from unknown contracts that match known topic patterns? Should it attempt interpretation or skip?
- What is the expected volume of events per 5-minute window, and what are the performance implications for the Lambda execution?

## Acceptance Criteria

- [ ] Documented event signatures for swap events on at least one known Soroban DEX
- [ ] Documented event signatures for token transfer, mint, and burn
- [ ] Pattern registry design documented with extensibility model for adding new patterns
- [ ] Watermark strategy selected and documented
- [ ] Idempotent reprocessing approach selected (upsert vs skip-if-exists)
- [ ] `human_readable` text template approach documented with examples for each pattern type
- [ ] `structured_data` JSONB structure defined for each pattern type
- [ ] Volume estimate per 5-minute window and Lambda performance assessment
- [ ] Unknown contract handling strategy documented

## Notes

- The Event Interpreter is decoupled from the Ledger Processor intentionally. It should never block or delay primary ingestion. If the Event Interpreter fails, the core explorer functionality (transactions, events, contracts) continues to work -- only human-readable summaries are missing.
- The `event_interpretations` table has an index on `interpretation_type`, suggesting the API may filter or group by pattern type.
- The frontend transaction detail page in normal mode shows human-readable summaries (e.g., "Swapped 100 USDC for 95.2 XLM on Soroswap"). These summaries come from event_interpretations, so the `human_readable` text format must be display-ready.
- The `soroban_events.topics` GIN index is available for the Event Interpreter to query events by topic pattern efficiently.
