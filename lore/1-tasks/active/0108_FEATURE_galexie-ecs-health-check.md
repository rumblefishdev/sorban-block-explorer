---
id: '0108'
title: 'Galexie ECS container health check'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0034', '0039']
tags: [priority-high, effort-small, layer-infra]
links: []
history:
  - date: 2026-04-08
    status: backlog
    who: fmazur
    note: 'Task created — spawned from 0039 TODO in ingestion-stack.ts'
  - date: 2026-04-08
    status: active
    who: fmazur
    note: 'Activated task'
---

# Galexie ECS container health check

## Summary

Add container-level health checks to Galexie ECS task definitions (live + backfill) so that ECS detects process crashes and restarts the task automatically. Without a health check, the existing `circuitBreaker: { rollback: true }` on the live service cannot act on container failures.

## Context

TODO at `infra/src/lib/stacks/ingestion-stack.ts:206`. Task 0034 deferred this until the image runs on staging.

Galexie runs `stellar-core` as a child process. A `pgrep stellar-core` check covers the main failure mode — core crash or exit. Hang detection (data freshness) is out of scope here and belongs in CloudWatch alarms (task 0036).

## Implementation Plan

### Step 1: Add health check to live container (ingestion-stack.ts)

Add `healthCheck` to the `liveContainer` definition at line 195:

```typescript
healthCheck: {
  command: ['CMD-SHELL', 'pgrep -x stellar-core || exit 1'],
  interval: cdk.Duration.seconds(30),
  timeout: cdk.Duration.seconds(5),
  retries: 3,
  startPeriod: cdk.Duration.seconds(120),
},
```

- `CMD-SHELL` — runs via container shell, no extra binary needed
- `pgrep -x` — exact match, avoids false positives
- `startPeriod: 120s` — Captive Core needs time to catch up on startup
- `retries: 3` — tolerates transient process restarts during catchup

### Step 2: Add health check to backfill container

Same health check on the `backfillContainer` (line 269). Backfill is one-shot but still benefits from crash detection during long-running historical imports.

### Step 3: Remove the TODO comment

Delete the TODO block at lines 206–210.

## Acceptance Criteria

- [ ] Health check added to live container definition
- [ ] Health check added to backfill container definition
- [ ] TODO comment at ingestion-stack.ts:206 removed
- [ ] `pnpm nx run @rumblefish/soroban-block-explorer-infra:build` passes
