---
id: '0116'
title: 'BUG: indexer Lambda concurrency exhausts RDS connections — blocks deploy migrations'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0113', '0034']
tags: [bug, indexer, rds, lambda, staging, priority-high, effort-small]
links:
  - infra/src/lib/stacks/compute-stack.ts
  - crates/db/src/pool.rs
history:
  - date: '2026-04-09'
    status: backlog
    who: stkrolikiewicz
    note: 'Discovered during staging deploy after indexer fix (0113) went live. Unrestricted indexer concurrency saturated t4g.micro max_connections (74/87), migration Lambda got pool timeout.'
---

# BUG: indexer Lambda concurrency exhausts RDS connections

## Symptom

Staging deploy fails at `Explorer-staging-Migration` stack:

```
UPDATE_FAILED | RunMigrations | pool timed out while waiting for an open connection
```

CloudFormation enters `UPDATE_ROLLBACK_FAILED` state requiring manual
`continue-update-rollback` to recover.

## Root cause

After indexer fix (task 0113) landed, the indexer Lambda started
processing S3 events for real. With no reserved concurrency limit,
AWS spun up ~74 concurrent Lambda instances (one per S3 PutObject event
from Galexie). Each instance holds 1 DB connection (per `crates/db/src/pool.rs:13`
`max_connections(1)`).

Staging RDS is `db.t4g.micro` with ~87 `max_connections`. 74 indexer
connections + RDS Proxy overhead left no room for the migration Lambda.

## Verified data (2026-04-09)

CloudWatch `DatabaseConnections` metric:

| Time (UTC)  | Connections | Context                              |
| ----------- | ----------- | ------------------------------------ |
| 10:29-10:54 | 2-9         | Before indexer ramp-up               |
| 10:59       | 68          | Indexer processing S3 events         |
| 11:04-11:19 | 74          | Saturated — migration fails at 10:32 |
| 11:24       | 23          | Connections draining                 |

## Temporary workaround (applied via CLI)

```bash
aws lambda put-function-concurrency \
  --function-name staging-soroban-explorer-indexer \
  --reserved-concurrent-executions 20 \
  --region us-east-1
```

This is a CLI override, not persisted in CDK. Next `cdk deploy` will
remove it unless codified in CDK.

## Fix scope

1. **Add `reservedConcurrentExecutions: 20` to indexer Lambda in CDK**
   (`infra/src/lib/stacks/compute-stack.ts`). Codifies the workaround
   so `cdk deploy` preserves it.

2. **Rationale for 20:** Galexie produces ~12 files/min on mainnet.
   Each Lambda invocation processes a file in <1s. 20 concurrent is
   more than sufficient throughput. Leaves ~64 connection slots for
   API Lambda, migration Lambda, RDS Proxy, and headroom.

3. **Consider also:** making concurrency configurable via
   `EnvironmentConfig` (`infra/envs/staging.json`) so it can be tuned
   per environment without code change. Production may need a different
   value.

## Acceptance criteria

- [ ] `reservedConcurrentExecutions` set in CDK for indexer Lambda
- [ ] Value configurable via `EnvironmentConfig` (or hardcoded with comment explaining why 20)
- [ ] `cdk deploy` passes without migration timeout
- [ ] CloudWatch `DatabaseConnections` stays below 50 during deploy
- [ ] CLI workaround removed (CDK manages it)

## Risks

- **Throttled indexer events** — with concurrency = 20, excess S3 events
  queue in Lambda's async invoke queue. If Galexie writes faster than
  20 Lambda instances can process, events queue up. At 12 files/min and
  <1s per file, 20 concurrent is more than enough. But during historical
  backfill (task 0030) with high file throughput, may need higher limit.
- **DLQ overflow** — throttled events retry 2x then go to DLQ. At steady
  state (12/min, 20 concurrent) no throttling expected. Monitor DLQ
  after fix.
