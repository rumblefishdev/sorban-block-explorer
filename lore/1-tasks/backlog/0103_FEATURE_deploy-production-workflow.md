---
id: '0103'
title: 'CI/CD: Production deployment workflow'
type: FEATURE
status: backlog
related_adr: ['0001']
related_tasks: ['0039']
tags: [priority-medium, effort-small, layer-infra]
milestone: 3
links: []
history:
  - date: 2026-04-07
    status: backlog
    who: fmazur
    note: 'Spawned from task 0039. Production deploy workflow deferred to milestone 3.'
---

# CI/CD: Production deployment workflow

## Summary

Add GitHub Actions workflow for manual production deployment with approval gate. Uses OIDC for AWS auth, shows CDK diff before approval, mirrors Galexie image to ECR, runs CDK deploy, and verifies with a smoke test.

## Context

Task 0039 defined the CI workflow (Rust + TypeScript CI jobs), CDK OIDC/deploy roles, and staging deployment workflow. The production deployment workflow was designed but deferred — not needed until production environment is ready.

The workflow file (`deploy-production.yml`) was drafted in task 0039 and can be used as starting point.

## Acceptance Criteria

- [ ] Manual trigger via workflow_dispatch, restricted to master branch
- [ ] CDK diff job runs before approval for changeset review
- [ ] Required reviewers via GitHub Environment "production" protection rules
- [ ] Uses OIDC to assume production deploy role
- [ ] Mirrors Galexie image to ECR with git SHA tag (digest-pinned pull)
- [ ] Runs `cdk deploy --all` with `-c galexieImageTag=${GITHUB_SHA}`
- [ ] Concurrency group prevents parallel deploys
- [ ] Post-deploy smoke test on /health endpoint
