---
id: '0088'
title: 'Unit and integration tests: XDR parsing, API endpoints, event interpretation'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-large, layer-testing]
milestone: 3
links:
  - docs/architecture/technical-design-general-overview.md
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — D3 scope coverage (task 0085)'
---

# Unit and integration tests: XDR parsing, API endpoints, event interpretation

## Summary

Write unit tests for XDR parsing correctness, API endpoint responses, and event interpretation logic. Write integration tests covering the end-to-end pipeline: ingestion → database → API → frontend data. D3 acceptance criteria require test coverage across all three layers.

## Status: Backlog

**Current state:** Not started.

## Context

D3 (§7.4) requires "Unit and integration tests covering XDR parsing correctness, API endpoint responses, and event interpretation logic." The effort breakdown (§7.1F) allocates: unit tests API — 8 days, unit tests XDR/ingestion — 7 days, integration tests e2e — 5 days (20 days total).

## Implementation Plan

### Step 1: Unit tests — XDR parsing + ingestion correctness (7 days)

Test XDR deserialization for LedgerCloseMeta, operations, soroban events/invocations, and ledger entry changes against known mainnet transaction hashes. Verify field extraction, edge cases, and error handling.

### Step 2: Unit tests — API endpoints (8 days)

Test all NestJS controller/service layers: request validation, response shape, pagination, error envelopes, filter parameters. Mock database layer.

### Step 3: Integration tests — end-to-end (5 days)

Test ingestion → API → response pipeline with real database. Ingest known testnet ledgers, query API endpoints, verify data consistency.

## Acceptance Criteria

- [ ] XDR parsing tests cover all 4 parsing tasks (0024–0027)
- [ ] API endpoint tests cover all feature modules (0045–0053)
- [ ] Event interpretation tests verify human-readable summaries for known patterns
- [ ] Integration tests verify ingestion → API data consistency
- [ ] All tests pass in CI pipeline
