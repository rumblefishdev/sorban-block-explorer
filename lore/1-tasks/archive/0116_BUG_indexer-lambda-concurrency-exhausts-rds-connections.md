---
id: '0116'
title: 'BUG: indexer Lambda concurrency exhausts RDS connections — blocks deploy migrations'
type: BUG
status: completed
related_adr: []
related_tasks: ['0113', '0034', '0110']
tags: [bug, indexer, rds, lambda, staging, priority-high, effort-small]
milestone: 1
links:
  - infra/src/lib/stacks/compute-stack.ts
  - crates/db/src/pool.rs
history:
  - date: '2026-04-09'
    status: backlog
    who: stkrolikiewicz
    note: 'Discovered during staging deploy after indexer fix (0113) went live. Unrestricted indexer concurrency saturated t4g.micro max_connections (74/87), migration Lambda got pool timeout.'
  - date: '2026-04-09'
    status: active
    who: stkrolikiewicz
    note: 'Promoted from backlog to active.'
  - date: '2026-04-09'
    status: completed
    who: stkrolikiewicz
    note: >
      PR #87 merged. reservedConcurrentExecutions=20 added to indexer Lambda
      in CDK, configurable via indexerLambdaConcurrency in envs/*.json.
      CLI workaround applied for immediate unblock. Staging deploy passes.
---

# BUG: indexer Lambda concurrency exhausts RDS connections

## Summary

After indexer fix (0113), unrestricted Lambda concurrency (74 instances)
saturated RDS t4g.micro max_connections (~87), blocking migration Lambda
during deploy with `pool timed out`.

## Acceptance criteria

- [x] `reservedConcurrentExecutions` set in CDK for indexer Lambda (PR #87)
- [x] Value configurable via `EnvironmentConfig` — `indexerLambdaConcurrency` in envs/\*.json
- [x] `cdk deploy` passes without migration timeout — staging deploy succeeded
- [x] CloudWatch connections within budget — concurrency=20 limits to max 20 indexer connections
- [x] CLI workaround applied for immediate unblock — `SpaDeployAndCloudfrontAccess` inline policy (redundant once CICD stack redeployed)

## Implementation notes

- `infra/src/lib/stacks/compute-stack.ts` — added `reservedConcurrentExecutions: config.indexerLambdaConcurrency`
- `infra/src/lib/types.ts` — added `indexerLambdaConcurrency: number` to `EnvironmentConfig`
- `infra/envs/staging.json` — `indexerLambdaConcurrency: 20`
- `infra/envs/production.json` — `indexerLambdaConcurrency: 20`

## Design decisions

### From Plan

1. **Concurrency = 20.** Galexie produces ~12 files/min. Each Lambda processes in <1s. 20 concurrent is sufficient throughput. Leaves ~64 connection slots for API + migration + proxy overhead.

### Emerged

2. **CLI workaround first, CDK fix second.** Deploy was blocked; applied `aws lambda put-function-concurrency` immediately, then codified in CDK as permanent fix.
3. **Same value for production.** 20 concurrent is conservative enough for both environments. Can be tuned independently via envs/\*.json.
