# 2026-04-08 — PR 1 pivot from `vars.AWS_REGION` to comments

## Context

Original PR 1 plan (per `G-subtask-breakdown.md` revision 1):

1. Replace `us-east-1` literals in `deploy-staging.yml` with `${{ vars.AWS_REGION }}`.
2. Create `AWS_REGION` GH variable in `staging` environment.
3. Add CI regression guard.

## What changed during research

Grep of `us-east-1` across the whole repo (PR 1 pre-flight) revealed:

- **3 literals** in `.github/workflows/deploy-staging.yml` (in scope of original PR 1).
- **`infra/envs/staging.json:3`** `awsRegion: "us-east-1"` — canonical source consumed by `infra/src/bin/staging.ts` → `infra/src/lib/app.ts:30` (`region: config.awsRegion`).
- **`infra/envs/staging.json:5`** `availabilityZones: ["us-east-1a", "us-east-1b"]` — region-specific.
- **`infra/envs/staging.json:45`** ACM cert ARN: `arn:aws:acm:us-east-1:750702271865:certificate/...`.
- ~5 occurrences in CDK code/comments referencing region semantics.
- ~30 occurrences in docs and lore (out of scope, no action needed).

## Emerged finding

The ACM certificate referenced in `staging.json:45` is for CloudFront. **CloudFront requires its certificate in `us-east-1` regardless of where the rest of the stack runs.** This creates a hard architectural lock — the staging stack region cannot change to anything other than `us-east-1` while CloudFront is part of the architecture.

## Decision

PR 1 pivoted from "introduce GH variable" to "document the constraint via inline comments".

### Rationale

1. **No real value in `vars.AWS_REGION` if region never changes.** The abstraction would be dead code with no second-value, only setup ceremony (admin request to create variable).
2. **Two sources of truth would emerge.** Workflow change alone leaves CDK reading region from `staging.json` and workflow reading from GH var. They must agree but nothing enforces it. Worse than the current (single hardcoded literal in two places).
3. **Region consolidation belongs to task 0038.** That task introduces a CDK environment config module and is the right place to make region a first-class abstraction across both CDK and CI layers consistently.
4. **Comments capture intent at zero cost.** 3 lines, no infrastructure, no admin involvement, no future maintenance.

### What we lost

- CI-enforced regression guard. The comments are convention, not enforcement. Accepted because:
  - The canonical source file path is referenced in the comment, making the link discoverable on next region change.
  - `infra/envs/staging.json` itself would be reviewed in any region change.
  - The region is architecturally locked anyway.

## Implementation

Branch: `feat/0110-pr1-region-comments`
PR: https://github.com/rumblefishdev/soroban-block-explorer/pull/79
Diff: 1 file, 5 lines added (3 comments + 2 blank-line context)

```diff
+          # Must match infra/envs/staging.json -> awsRegion (single source of truth).
+          # Region is locked to us-east-1 by ACM cert requirement for CloudFront.
           aws-region: us-east-1
```

(Same pattern in 3 places: mirror-image job, ECR login, deploy job.)

## Side effects on other PRs in 0110

- **PR 2 (caching)** — unaffected. Caching plan does not depend on region abstraction.
- **PR 3 (tag-gating)** — unaffected.
- **0038 hand-off** — strengthened. The hand-off note in `G-quality-gates.md` should now explicitly say that region consolidation is fully delegated to 0038, not partially started in 0110.
- **0103 (prod workflow)** — should adopt the same comments-only approach when implementing analogous changes for `deploy-production.yml`. Update 0103 README.

## Why this is senior-like

- Honest about ROI: refused to ship a refactor whose only purpose was to look like progress.
- Avoided creating dead abstraction.
- Respected scope boundaries between tasks (0110 vs 0038).
- Documented the reasoning so future me (or any reader) understands why PR 1 looks small.
