---
id: '0103'
title: 'CI/CD: Production deployment workflow'
type: FEATURE
status: backlog
related_adr: ['0001']
related_tasks: ['0039', '0110', '0038']
tags: [priority-medium, effort-medium, layer-infra, ci, cd]
milestone: 3
links: []
history:
  - date: '2026-04-07'
    status: backlog
    who: fmazur
    note: 'Spawned from task 0039. Production deploy workflow deferred to milestone 3.'
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Scope extended: apply region-var + caching + tag-gating improvements from 0110 (staging pilot).'
---

# CI/CD: Production deployment workflow

## Summary

Add GitHub Actions workflow for manual production deployment with approval gate. Uses OIDC for AWS auth, shows CDK diff before approval, mirrors Galexie image to ECR, runs CDK deploy, and verifies with a smoke test.

## Context

Task 0039 defined the CI workflow (Rust + TypeScript CI jobs), CDK OIDC/deploy roles, and staging deployment workflow. The production deployment workflow was designed but deferred — not needed until production environment is ready.

The workflow file (`deploy-production.yml`) was drafted in task 0039 and can be used as starting point.

## Acceptance Criteria

### Core workflow (from 0039)

- [ ] Manual trigger via workflow_dispatch, restricted to master branch
- [ ] CDK diff job runs before approval for changeset review
- [ ] Required reviewers via GitHub Environment "production" protection rules
- [ ] Uses OIDC to assume production deploy role
- [ ] Mirrors Galexie image to ECR with git SHA tag (digest-pinned pull)
- [ ] Runs `cdk deploy --all` with `-c galexieImageTag=${GITHUB_SHA}`
- [ ] Concurrency group prevents parallel deploys
- [ ] Post-deploy smoke test on /health endpoint

### Extended scope — mirror improvements from 0110 (staging pilot)

Apply the three improvements validated on staging in 0110:

- [ ] **Region as GitHub variable** — no literal `us-east-1` in `deploy-production.yml`; sourced from `vars.AWS_REGION` in the `production` GitHub Environment.
- [ ] **Deploy caching** — reuse caching strategy from 0110 (inputs only: node_modules, cargo target, Nx cache; **do NOT cache `cdk.out`**). Baseline measurements taken before optimization.
- [ ] **Tag-gated trigger** — align with tagging scheme established by 0110's ADR. Production typically triggers on release tags (e.g. `prod-v*` or `v*`); exact pattern decided in the ADR.

**Dependency:** 0110 should land first so production reuses the validated
patterns (caching keys, tag scheme, ADR, region-var convention). If 0110 is
blocked, production workflow can still be built with the core workflow
criteria, and extended scope applied as a follow-up.
