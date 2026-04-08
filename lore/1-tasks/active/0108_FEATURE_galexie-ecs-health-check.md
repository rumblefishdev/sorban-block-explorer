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

Add a container-level health check to the Galexie live ECS service so that ECS can detect hangs and crashes and automatically restart the task. Currently there is a TODO at `infra/src/lib/stacks/ingestion-stack.ts:206` — without a health check, ECS only detects full process exits, not silent hangs where Galexie stops producing data.

## Context

Task 0034 deployed the Galexie ECS/Fargate service but deferred the health check until the image is running on staging and we can investigate what's available inside `stellar/stellar-galexie`.

A naive `pgrep` check only detects crashes, not hangs. A meaningful check should verify that Galexie is actually producing data (e.g. checking the age of the last exported file on the local data volume or an HTTP endpoint if one exists).

## Implementation Plan

### Step 1: Investigate stellar/stellar-galexie image

Run the image locally or via ECS Exec on staging to determine:

- Whether it exposes an HTTP health endpoint
- What processes run inside the container
- Whether last-exported-file age on `/data` can be checked via a shell command

### Step 2: Implement health check in CDK

Add `healthCheck` property to the container definition in `ingestion-stack.ts`:

```typescript
healthCheck: {
  command: [/* determined in Step 1 */],
  interval: cdk.Duration.seconds(30),
  timeout: cdk.Duration.seconds(5),
  retries: 3,
  startPeriod: cdk.Duration.seconds(120),
},
```

### Step 3: Validate on staging

Deploy to staging, confirm health check passes during normal operation and correctly marks the task unhealthy when Galexie stalls.

## Acceptance Criteria

- [ ] Health check added to Galexie live container definition
- [ ] Health check detects actual data production stall (not just process liveness)
- [ ] `startPeriod` accounts for Galexie cold-start time
- [ ] Validated on staging — task restarts on simulated hang
- [ ] TODO at ingestion-stack.ts:206 removed
