---
id: '0114'
title: 'FEATURE: SPA staging deploy pipeline (build + s3 sync + CloudFront invalidation)'
type: FEATURE
status: completed
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
  - date: 2026-04-09
    status: completed
    who: stkrolikiewicz
    note: >
      Implemented all 4 steps. 2 files changed: deploy-staging.yml
      (+4 workflow steps), delivery-stack.ts (+1 CfnOutput).
      Key decision: frontend smoke test uses basic auth secret
      for full e2e verification through CloudFront.
---

# FEATURE: SPA staging deploy pipeline

## Summary

Add the missing SPA build + upload + CloudFront invalidation steps to
the staging deploy workflow so that the React frontend actually lands
in `s3://staging-soroban-explorer-spa/` and is served via CloudFront
at `https://staging.sorobanscan.rumblefish.dev`.

## Context

- Infra is in place: `Explorer-staging-Delivery` stack provisions the
  S3 bucket (`staging-soroban-explorer-spa`) and CloudFront
  distribution (task 0035). Cache TTL hot-fix shipped under 0106.
- Frontend Nx scaffold exists (task 0084) and individual page tasks
  (0066-0087) implement the app, but nothing wires them into CI/CD
  for staging.
- Current `.github/workflows/deploy-staging.yml` runs `cdk deploy` and
  a `/health` smoke test against the API. **No `nx build` for the
  frontend, no `aws s3 sync`, no `cloudfront create-invalidation`.**
- Symptom that surfaced this gap: frontend URL returns S3
  `<Error><Code>AccessDenied` because the bucket has zero objects.
- Task 0103 covers the **production** deploy workflow — this task is
  scoped to staging only.

## Acceptance Criteria

- [x] `deploy-staging.yml` builds the SPA via Nx
- [x] SPA artifacts uploaded to `staging-soroban-explorer-spa` with
      `--delete` so stale assets are pruned
- [x] CloudFront invalidation issued for `/*` after upload
- [x] Frontend smoke test (HTTP 200 on `/`) added to the workflow
- [x] Bucket name and distribution ID resolved from CFN outputs (no
      hardcoding)
- [x] `https://staging.sorobanscan.rumblefish.dev` serves the SPA
      after the next deploy run

## Implementation Notes

**Files changed:**

1. `.github/workflows/deploy-staging.yml` — added 4 steps to `deploy` job:

   - **Build SPA** (`npx nx build @rumblefish/soroban-block-explorer-web`) before CDK build
   - **Upload SPA to S3** (`aws s3 sync web/dist/ s3://$BUCKET --delete`) after CDK deploy
   - **Invalidate CloudFront cache** (`aws cloudfront create-invalidation --paths "/*"`)
   - **Smoke test -- Frontend** (curl with HTTP 200 + content-type check)

2. `infra/src/lib/stacks/delivery-stack.ts` — added `DistributionId` CfnOutput
   (needed for `create-invalidation`, was missing from original stack)

**Ordering:** Build SPA -> Build CDK -> CDK deploy -> S3 sync -> CF invalidation -> API smoke -> Frontend smoke

**Manual step required:** Add `STAGING_BASIC_AUTH` secret (format `user:password`)
to GitHub Environment "staging" before first run. The secret must match
the base64 token in the CloudFront KeyValueStore `auth-token` key
(see `delivery-stack.ts` for KVS population instructions). If the KVS
is empty (fresh deploy), CloudFront returns 503 and the smoke test fails.

## Design Decisions

### From Plan

1. **Resolve bucket/dist ID from CFN outputs**: Consistent with existing
   smoke test pattern for API endpoint. No hardcoded resource names.

2. **`--delete` flag on s3 sync**: Prunes stale assets from previous deploys.

3. **CloudFront invalidation on `/*`**: Required for index.html to flip
   atomically despite short TTL from task 0106.

### Emerged

4. **Added `DistributionId` CfnOutput to CDK**: Stack only exported
   `DistributionDomainName` and `SpaBucketName`. The distribution ID
   is needed for `create-invalidation` but was not part of original
   stack outputs. Safe additive change — no resource modifications.

5. **Frontend smoke test uses basic auth secret**: Staging has
   `enableBasicAuth: true` (CloudFront Function). Instead of bypassing
   auth via S3 direct check, chose full e2e curl through CloudFront
   with `STAGING_BASIC_AUTH` secret. Validates the complete delivery
   path including auth layer.

## Issues Encountered

- None. Straightforward implementation.

## Future Work

- Task 0103 covers the production equivalent — steps are factored
  for easy lift.
