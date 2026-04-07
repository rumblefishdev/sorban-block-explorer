---
id: '0039'
title: 'CI/CD pipeline: GitHub Actions workflows'
type: FEATURE
status: completed
related_adr: ['0001', '0004', '0005']
related_tasks: ['0006', '0021', '0034', '0040', '0092', '0103']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: added Rust CI job (dtolnay/rust-toolchain, cargo-lambda, SQLX_OFFLINE)'
  - date: 2026-04-03
    status: active
    who: fmazur
    note: 'Activated task'
  - date: 2026-04-03
    status: active
    who: fmazur
    note: >
      Review: updated plan to reflect current infra state. Fixed package
      manager (npm not pnpm), branch strategy (master/develop not main),
      extend existing ci.yml instead of rewriting, removed redundant
      migration ordering step (CDK handles it), merged OIDC scope from
      task 0040, updated ECR references to task 0034/IngestionStack,
      use official stellar/stellar-galexie image (no custom Dockerfile).
  - date: 2026-04-07
    status: completed
    who: fmazur
    note: >
      Implemented CI + OIDC + staging deploy. 4 new files, 3 modified.
      Rust CI parallel to TS CI, CicdStack with OIDC + deploy roles,
      staging auto-deploy from develop with ECR mirror + smoke test.
      Production deploy deferred to task 0103 (milestone 3).
---

# CI/CD pipeline: GitHub Actions workflows

## Summary

Extended CI with Rust job, added OIDC deploy roles in CDK, and created staging deployment workflow with Galexie image mirroring to ECR.

## Acceptance Criteria

**CI:**

- [x] Existing ci.yml extended with parallel Rust job
- [x] Rust CI: cargo fmt, clippy, test, cargo lambda build with SQLX_OFFLINE=true
- [x] Both TypeScript and Rust jobs must pass before merge
- [ ] Branch protection rules on master and develop (manual GitHub config post-merge)

**OIDC & IAM (merged from task 0040):**

- [x] GitHub Actions OIDC identity provider defined in CDK
- [x] Separate staging and production deploy roles with environment-scoped trust policies
- [x] No long-lived AWS access keys in GitHub secrets
- [x] Deploy roles follow least-privilege

**Staging:**

- [x] Auto-deploys after CI passes on develop
- [x] Uses OIDC to assume staging deploy role
- [x] Runs `cdk deploy --all` with `-c galexieImageTag` context
- [x] Mirrors Galexie image to ECR with git SHA tag

**Production:** Deferred to task 0103 (milestone 3).

**ECR Image:**

- [x] Official `stellar/stellar-galexie` image pulled by digest with format validation
- [x] Pushed to ECR with git SHA tag for traceability
- [x] ECR repo URI fetched from SSM (not hardcoded)
- [x] galexieImageTag passed via CDK context
- [x] Skip mirror if digest already in ECR (saves time + Docker Hub rate limit)

**Operational:**

- [x] Concurrency group on staging deploy workflow
- [x] Post-deploy smoke test (health endpoint)
- [x] Rollback by re-running previous workflow
- [ ] galexieDesiredCount set to 1 after first image push (manual step)

## Implementation Notes

**New files:**

- `.github/workflows/ci.yml` — extended with parallel Rust CI job
- `.github/workflows/deploy-staging.yml` — auto-deploy from develop
- `infra/src/lib/stacks/cicd-stack.ts` — OIDC provider + deploy roles (~140 lines)
- `infra/src/lib/cicd-app.ts` — factory function (consistent with bastion-app pattern)
- `infra/src/bin/cicd.ts` — entry point for CicdStack
- `infra/envs/cicd.json` — shared CICD config (region, repo name)
- `lore/1-tasks/backlog/0103_FEATURE_deploy-production-workflow.md` — spawned task

**Modified files:**

- `infra/src/lib/types.ts` — added CicdConfig interface
- `infra/src/lib/stacks/ingestion-stack.ts` — galexieImageTag reads CDK context with config fallback
- `infra/src/index.ts` — exported CicdStack + CicdConfig
- `infra/Makefile` — deploy-cicd, diff-cicd targets

## Issues Encountered

- **`StringEquals` duplicate key:** OIDC trust policy had two `StringEquals` blocks in one object — TypeScript error. Merged `:aud` and `:sub` conditions into single `StringEquals`.
- **`workflow_dispatch.branches` doesn't exist:** GitHub Actions ignores `branches` on `workflow_dispatch`. Used `if: github.ref == 'refs/heads/master'` instead for production (deferred to 0103).
- **Staging deploy not gated on CI:** Initial `on: push` trigger ran deploy independently of CI. Changed to `workflow_run` trigger that waits for CI to complete on develop.
- **galexieImageTag passthrough:** Makefile targets don't support CDK context args. Workflows build CDK via Nx and run `npx cdk` directly with `-c galexieImageTag`.

## Design Decisions

### From Plan

1. **OIDC-only auth:** No static AWS access keys. OIDC roles scoped to GitHub Environment name per ADR 0001.
2. **Official Galexie image:** Pull `stellar/stellar-galexie` from Docker Hub, re-tag, push to ECR. No custom Dockerfile — official image bundles stellar-core + galexie. Apache 2.0 license allows redistribution.
3. **CDK context for image tag:** `-c galexieImageTag=${GITHUB_SHA}` passed at deploy time. IngestionStack reads context with fallback to config JSON.

### Emerged

4. **Separate CicdStack (account-level):** OIDC provider is singleton per AWS account. Created dedicated stack with own entry point (`cicd.ts`) and config (`cicd.json`), following bastion-app pattern. Not per-environment.
5. **ECR mirror skip when digest unchanged:** Added check against existing ECR tags — if same digest is already in ECR, skip Docker Hub pull. Saves ~2 min per deploy and avoids Docker Hub rate limits.
6. **Digest format validation:** `[[ "$GALEXIE_DIGEST" =~ ^sha256:[a-f0-9]{64}$ ]]` — prevents shell injection from misconfigured GitHub Environment variable.
7. **Staging deploys from develop, not master:** Changed trigger from `master` to `develop` branch per team's branch strategy (develop = integration, master = release).
8. **Production deferred to task 0103:** Not needed until production environment is ready. Keeps PR scope focused.

## Future Work

- Production deployment workflow (task 0103, milestone 3)
- Pin `cargo-lambda` and third-party GitHub Actions to specific versions/SHAs
- Task 0040 retains: Lambda IAM role refinements
