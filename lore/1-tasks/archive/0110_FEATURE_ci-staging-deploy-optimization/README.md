---
id: '0110'
title: 'CI: staging deploy optimization — region var, caching, tag-gating'
type: FEATURE
status: completed
related_adr: ['0009']
related_tasks: ['0039', '0038', '0103']
tags: [ci, cd, cdk, github-actions, staging, priority-medium, effort-medium]
links:
  - .github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Task created — bundles 3 independent improvements to staging deploy workflow'
  - date: '2026-04-08'
    status: active
    who: stkrolikiewicz
    note: 'Promoted from backlog to active.'
  - date: '2026-04-08'
    status: active
    who: stkrolikiewicz
    note: 'Converted to directory format; detailed plan split into notes/.'
  - date: '2026-04-09'
    status: completed
    who: stkrolikiewicz
    note: >
      All 4 PRs merged (#76, #79, #82, #84) + IAM fix (#88).
      Tag-based deploy verified (staging-2026.04.09-1). ADR 0009 accepted.
      Phase 0 baseline documented. 3 follow-up tasks spawned (0112, 0115, 0116).
      Key emerged decisions: PR 1 pivoted from vars.AWS_REGION to comments
      (region locked by ACM cert), PR 2 pivoted from Rust/Nx caching to
      node_modules cache (CDK deploy=76% of wall-clock, not build steps).
---

# CI: staging deploy optimization

## Summary

Optimized staging deploy workflow with 4 PRs: workflow_dispatch trigger,
region documentation, node_modules caching (~42s saving), and tag-gated
deploys (staging-YYYY.MM.DD-N). Helper script `scripts/staging-deploy.sh`
automates tagging.

## Acceptance criteria

- [x] PR 0 merged — `workflow_dispatch` available (PR #76)
- [x] PR 1 merged — `deploy-staging.yml` documents `infra/envs/staging.json` as canonical source for region (PR #79, pivoted to comments — see worklog/2026-04-08-pr1-pivot-to-comments.md)
- [x] PR 2 merged — node_modules cache, ~42s saving (PR #82, Phase 0 baseline: CDK deploy=76%, see worklog/2026-04-09)
- [x] PR 3 merged — tag-gated deploy with `staging-*` pattern (PR #84, ADR 0009 accepted)
- [x] ~~Regression guard~~ (dropped — region locked by ACM cert)
- [x] Worklog entries for each PR
- [x] Post-merge validation — tag deploy `staging-2026.04.09-1` succeeded
- [x] IAM fix for SPA deploy + CloudFront invalidation (PR #88)

## Implementation notes

| PR  | File(s)                                                     | What                                                                  |
| --- | ----------------------------------------------------------- | --------------------------------------------------------------------- |
| #76 | `deploy-staging.yml`                                        | Added `workflow_dispatch:` trigger (+1 line)                          |
| #79 | `deploy-staging.yml`                                        | Inline comments documenting region source of truth (+5 lines)         |
| #82 | `deploy-staging.yml`                                        | `actions/cache@v4` for node_modules, conditional `npm ci` (+12 lines) |
| #84 | `deploy-staging.yml`, `scripts/staging-deploy.sh`, ADR 0009 | Tag trigger `staging-*`, helper script with validation                |
| #88 | `infra/src/lib/stacks/cicd-stack.ts`                        | S3 + CloudFront IAM grants for deploy role                            |

## Design decisions

### From Plan

1. **Each subtask = one PR.** Independently revertable, clean review.
2. **Measure before optimizing.** Phase 0 baseline mandatory before PR 2.
3. **ADR before tag-gating.** Process decision documented formally.
4. **Date-based tags `staging-YYYY.MM.DD-N`.** Zero ceremony, sufficient for 3-dev team.

### Emerged

5. **PR 1 pivoted from `vars.AWS_REGION` to comments.** Research showed region is locked to us-east-1 by ACM cert requirement for CloudFront. GH variable would be dead abstraction. Region consolidation delegated to task 0038.
6. **PR 2 pivoted from Rust/Nx/cargo-lambda caching to node_modules only.** Phase 0 baseline showed CDK deploy (CloudFormation) = 76% of wall-clock. Rust cache tuning (<9s), Nx cache (<6s), cargo-lambda (<8s) — all dropped as below noise floor. `cdk diff` early exit blocked by galexieImageTag changing every commit → spawned task 0115.
7. **Pre-merge testing via workflow_dispatch from feature branch abandoned.** Staging environment branch policy restricts deploys to `develop` only. Testing would require widening policy → unnecessary risk. Replaced with actionlint + code review + post-merge observation.
8. **Required Reviewers as temporary gate.** Enabled on staging environment during task, removed after tag-gating landed. Tag = explicit deploy decision, no additional gate needed.
9. **Helper script `scripts/staging-deploy.sh` added.** Not in original plan. Automates tag creation with branch validation, remote tag fetch, MAX N computation. Prevents typos/collisions.
10. **IAM fix for SPA deploy.** Not in original scope. SPA sync + CloudFront invalidation steps were added to workflow by another dev during task. Deploy role missing S3/CloudFront permissions. Fixed in cicd-stack.ts + CLI workaround for immediate unblock.

## Issues encountered

- **Pre-commit hook re-staged `package-lock.json`** — PR 0 first push included accidental package-lock changes. Fixed by stashing before commit.
- **`workflow_dispatch` UI button not visible** — default branch is `master`, not `develop`. Buttons show for default branch only. CLI `gh workflow run --ref develop` works.
- **`staging-deploy.sh` failed on first run** — `set -euo pipefail` + `grep` returning exit 1 on empty input. Fixed with `|| true`.
- **CICD stack deploy failed** — `Explorer-Cicd` stack existed but CDK tried to recreate OIDC provider. Worked around with `aws iam put-role-policy` CLI. CDK code has correct policy; needs CICD stack redeploy to make permanent.
- **Indexer Lambda exhausted RDS connections** — discovered during deploy testing. 74 concurrent Lambdas saturated t4g.micro max_connections. Spawned task 0116.

## Future work (spawned as backlog tasks)

- **0112** — CI workflow optimization (arm64 runner, path filter, Nx cache for ci.yml)
- **0115** — CDK diff early exit for staging deploy (blocked by galexieImageTag)
- **0116** — Indexer Lambda concurrency limit in CDK (completed separately)

## Notes

Detailed notes preserved in `notes/`:

- `G-subtask-breakdown.md` — PR 0-3 detailed plans
- `G-caching-strategy.md` — Phase 0 baseline + revised optimization strategy
- `G-quality-gates.md` — Process, rollback, scope limits

Worklog in `worklog/`:

- `2026-04-08-pr1-pivot-to-comments.md`
- `2026-04-09-phase0-baseline.md`
