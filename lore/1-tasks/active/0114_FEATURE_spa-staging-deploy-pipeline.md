---
id: '0114'
title: 'FEATURE: SPA staging deploy pipeline (build + s3 sync + CloudFront invalidation)'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0035', '0106', '0084', '0103']
tags: [layer-infra, layer-frontend, priority-high, effort-small]
milestone: 2
links: []
history:
  - date: 2026-04-08
    status: backlog
    who: stkrolikiewicz
    note: >
      Discovered while debugging "AccessDenied" XML response on
      https://staging.sorobanscan.rumblefish.dev — bucket
      `staging-soroban-explorer-spa` is empty. CDK provisions the
      CloudFront + bucket (task 0035) and a follow-up hot-fix (0106)
      already adjusted the CloudFront cache policy in anticipation of
      "task 0039 frontend pipeline going live", but no actual SPA
      deploy step exists in `.github/workflows/deploy-staging.yml`.
      The workflow only builds CDK + smokes the API. Result: SPA has
      never been published to staging.
  - date: 2026-04-09
    status: active
    who: stkrolikiewicz
    note: Activated task for implementation.
---

# FEATURE: SPA staging deploy pipeline

## Summary

Add the missing SPA build + upload + CloudFront invalidation steps to
the staging deploy workflow so that the React frontend actually lands
in `s3://staging-soroban-explorer-spa/` and is served via CloudFront
at `https://staging.sorobanscan.rumblefish.dev`.

## Context

- Infra is in place: `Explorer-staging-Frontend` stack provisions the
  S3 bucket (`staging-soroban-explorer-spa`) and CloudFront
  distribution (task 0035). Cache TTL hot-fix shipped under 0106.
- Frontend Nx scaffold exists (task 0084) and individual page tasks
  (0066–0087) implement the app, but nothing wires them into CI/CD
  for staging.
- Current `.github/workflows/deploy-staging.yml` runs `cdk deploy` and
  a `/health` smoke test against the API. **No `nx build` for the
  frontend, no `aws s3 sync`, no `cloudfront create-invalidation`.**
- Symptom that surfaced this gap: frontend URL returns S3
  `<Error><Code>AccessDenied` because the bucket has zero objects.
- Task 0103 covers the **production** deploy workflow — this task is
  scoped to staging only.

## Implementation Plan

### Step 1: Build SPA in CI

Add an `nx build` step for the frontend project (resolve exact target
name when picking up the task — likely `@rumblefish/...-spa` or
similar) before the existing `Build CDK` step. Reuse the existing
`actions/setup-node` + `npm ci` setup.

### Step 2: Upload to S3

After CDK deploy succeeds (so the bucket is guaranteed to exist), run
`aws s3 sync dist/<spa>/ s3://staging-soroban-explorer-spa/ --delete`.
Resolve bucket name via `aws cloudformation describe-stacks` output
rather than hardcoding, to stay consistent with the smoke-test
pattern already in the workflow.

### Step 3: Invalidate CloudFront

Run `aws cloudfront create-invalidation --paths "/*"` against the
staging distribution ID (also resolved via CFN output). Cheap given
the short-TTL policy from 0106, but still required for `index.html`
to flip atomically.

### Step 4: Smoke test the frontend

Add a `curl -fI https://staging.sorobanscan.rumblefish.dev/` check
expecting 200 + `content-type: text/html` so future regressions
(empty bucket, broken upload) fail the deploy instead of silently
shipping AccessDenied.

## Acceptance Criteria

- [ ] `deploy-staging.yml` builds the SPA via Nx
- [ ] SPA artifacts uploaded to `staging-soroban-explorer-spa` with
      `--delete` so stale assets are pruned
- [ ] CloudFront invalidation issued for `/*` after upload
- [ ] Frontend smoke test (HTTP 200 on `/`) added to the workflow
- [ ] Bucket name and distribution ID resolved from CFN outputs (no
      hardcoding)
- [ ] `https://staging.sorobanscan.rumblefish.dev` serves the SPA
      after the next deploy run

## Notes

- Watch out for ordering: SPA upload **must** run after `cdk deploy`
  (bucket might be created on first run) and **before** the
  invalidation.
- Production equivalent will be picked up by task 0103; keep the
  staging step factored so 0103 can lift it with minimal changes.
- Related: 0106 already tuned CloudFront cache TTL for `index.html`,
  so no extra cache headers are needed on the upload side.
