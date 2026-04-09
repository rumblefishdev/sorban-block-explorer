---
id: '0115'
title: 'CI: cdk diff early exit for staging deploy (galexieImageTag blocker)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0110']
tags: [ci, cd, cdk, staging, priority-medium, effort-medium]
links:
  - .github/workflows/deploy-staging.yml
history:
  - date: '2026-04-09'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from lore-0110 PR 2 Phase 0 baseline. CDK deploy is 76% of staging deploy wall-clock (6m 7s avg). cdk diff early exit could save ~5 min on no-op deploys but is blocked by galexieImageTag changing every commit.'
---

# CI: cdk diff early exit for staging deploy

## Summary

Phase 0 baseline for lore-0110 showed CDK deploy (CloudFormation) is 76%
of staging deploy wall-clock (6m 7s out of 8m 1s). Running `cdk diff --all`
before `cdk deploy` and skipping deploy when no stacks changed could save
~5 min on no-op deploys.

## Blocker

`deploy-staging.yml` passes `-c galexieImageTag=${GITHUB_SHA}` to CDK.
Every commit has a different SHA, so `cdk diff` always reports a change
in the ECS task definition even for pure docs/lore commits. Until this
is resolved, `cdk diff` can never return "no differences".

## Options to investigate

1. Don't pass `galexieImageTag` to `cdk diff` — diff structural template only, deploy always passes tag.
2. Pin `galexieImageTag` to currently deployed value for diff (read from `aws ecs describe-task-definition`).
3. Restructure how image tag is passed — separate from CDK context, use SSM parameter or env var.
4. Accept that CDK diff always reports changes → this task is not viable → cancel.

## Acceptance criteria

- [ ] Design decision on galexieImageTag handling documented
- [ ] If viable: no-op staging deploy (no code/infra changes) completes in ≤ 2 min
- [ ] If not viable: task canceled with documented rationale

## Context

See `lore/1-tasks/active/0110_FEATURE_ci-staging-deploy-optimization/worklog/2026-04-09-phase0-baseline.md`
and `lore/1-tasks/active/0110_FEATURE_ci-staging-deploy-optimization/notes/G-caching-strategy.md` for full Phase 0 data and analysis.
