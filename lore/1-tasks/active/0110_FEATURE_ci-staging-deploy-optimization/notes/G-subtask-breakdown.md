---
title: 'Subtask breakdown — PRs 0-3'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [ci, cd, github-actions, plan]
links:
  - ../../../.github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: developing
    who: stkrolikiewicz
    note: 'Extracted from README during directory conversion.'
---

# Subtask breakdown

Four PRs. Strict scope limits. Independent branches from `develop`.

## Branch strategy

Each PR ships as a short-lived branch from `develop`. The parasol branch
`feat/0110_ci-staging-deploy-optimization` is **not** a PR base — it holds
scratch work only (baseline measurements, ADR drafts).

- `feat/0110-pr0-workflow-dispatch`
- `feat/0110-pr1-region-var`
- `feat/0110-pr2-caching`
- `feat/0110-pr3-tag-gating`

## PR 0 — `workflow_dispatch` trigger (prerequisite)

**Goal:** Make staging deploy triggerable manually without pushing to develop. Required for Phase 0 baseline (PR 2) and for safely testing workflow changes pre-merge.

**Steps:**

1. Add `workflow_dispatch:` to `on:` in `deploy-staging.yml`.
2. Add an early step `echo "Region: ${{ vars.AWS_REGION || 'us-east-1' }}"` as a sanity log — will light up once PR 1 lands, harmless before.
3. Test: trigger manually from GitHub UI on `develop` — deploy should run.

**Scope:** ~3 lines changed in one file. Tiny PR.

**Acceptance:**

- Manual trigger works from Actions tab on any branch containing the updated workflow file.
- Push-to-develop trigger still works (unchanged).

---

## PR 1 — Region as GitHub variable

**Goal:** Remove all literal `us-east-1` from `deploy-staging.yml`; source from `vars.AWS_REGION` in the `staging` GitHub Environment.

**⚠️ Coordination with 0038:** task 0038 introduces a central CDK config
module (`infra/aws-cdk/lib/config/`) with per-environment values. This PR
is **scoped to the workflow file only**: region goes into `vars.AWS_REGION`
(GitHub), workflow reads from there. CDK-internal region stays untouched
and can be revisited when 0038 lands. See hand-off note in `G-quality-gates.md`.

**Steps:**

1. Grep **the whole repo** for `us-east-1` and **document results in the PR description** — for each occurrence, decide: "this PR" / "out of scope (spawned task)". Target locations to check:
   - `.github/workflows/*.yml`
   - `infra/bin/staging.ts` and any CDK env config
   - SSM parameter paths, ARN strings, scripts, docs
2. **Before editing the workflow:** ask repo admin to create `AWS_REGION` variable in GitHub `staging` environment (default `us-east-1`). Must exist before merge.
3. Replace in `.github/workflows/deploy-staging.yml` only:
   - Line 26: `aws-region: ${{ vars.AWS_REGION }}`
   - Line 42: `aws ecr get-login-password --region ${{ vars.AWS_REGION }}`
   - Line 101: `aws-region: ${{ vars.AWS_REGION }}`
4. Add the regression guard (see `G-quality-gates.md`).
5. Test via `workflow_dispatch` on PR branch (enabled by PR 0).
6. Verify end-to-end staging deploy passes after merge.

**Scope limit (hard):** one file touched — `.github/workflows/deploy-staging.yml`. All other `us-east-1` occurrences → spawned backlog task. Do not expand scope mid-PR.

**Acceptance:**

- `rg 'us-east-1' .github/workflows/deploy-staging.yml` → no matches.
- Full repo grep documented in PR description with disposition per match.
- Regression guard in CI (see quality gates).
- `workflow_dispatch` pre-merge run successful.
- Staging deploy green after merge.

**Risks:**

- Missing `AWS_REGION` variable at merge time → workflow breaks. Mitigation: confirm variable exists via GH UI before clicking merge.
- CDK-internal hardcoded region may surface in deploy even though workflow is clean. If deploy fails with region-related error → revert, spawn follow-up task, do not try to fix in this PR.

---

## PR 2 — Deploy caching

**Goal:** Reduce deploy time for no-op / small-change deploys by caching synthesis _inputs_, not outputs.

### Phase 0 — Baseline (mandatory, before any code change)

Steps:

1. Trigger staging deploy 3× via `workflow_dispatch` (enabled by PR 0) against an unchanged `develop` (or with trivial unrelated changes).
2. From each run collect per-step timings via GitHub Actions UI or `gh run view --log`.
3. Record in `worklog/baseline-YYYY-MM-DD.md` using the template below.
4. Also record current staging deploy frequency via `gh api 'repos/:owner/:repo/actions/workflows/deploy-staging.yml/runs?per_page=100'` — count runs in last 30 days. Gates PR 2 and PR 3 ROI.

Template table to fill in the worklog:

| step                           | run1 | run2 | run3 | avg |
| ------------------------------ | ---- | ---- | ---- | --- |
| mirror-image (total)           |      |      |      |     |
| setup-node + npm ci            |      |      |      |     |
| Swatinem/rust-cache restore    |      |      |      |     |
| rust build (cargo check, etc.) |      |      |      |     |
| pip3 install cargo-lambda      |      |      |      |     |
| nx build CDK                   |      |      |      |     |
| cdk deploy --all               |      |      |      |     |
| smoke test                     |      |      |      |     |
| **total**                      |      |      |      |     |

**Go/no-go gate:** if total avg <5 min and deploys <2×/day, **close PR 2 as `canceled: obsolete`** and move on.

### Phase 1 — Implementation (only after baseline)

Apply caching in ROI order. Full strategy and Rust/Node/Nx details →
**[G-caching-strategy.md](G-caching-strategy.md)**

Summary (detailed in the other note):

1. Nx task cache (`.nx/cache`) — biggest expected win.
2. cargo-lambda prebuilt binary — certain ~30-60s win.
3. Rust build cache tuning (`Swatinem/rust-cache` with explicit key + `target/lambda/`).
4. `node_modules/` cache — only if still a bottleneck after above.

**Pre-flight:** verify `nx.json` `inputs`/`outputs` are correctly declared for the CDK `build` target. If not → **spawn a separate PR to fix Nx config first**, block PR 2 until it lands. Caching broken Nx config just replicates breakage.

### Phase 2 — Validation (before merge)

- Run cache correctness test matrix (see G-caching-strategy.md). Every row must produce correct hit/miss pattern.
- Add **post-deploy Lambda SHA256 verification step** to workflow — compute hash of built zip, compare to deployed `CodeSha256` from `aws lambda get-function`. This is mandatory for PR 2 merge regardless of which caches end up in scope, because stale-binary is the #1 caching failure mode.
- Observe at least one deploy with cache hit and one with cache miss (forced).

**Scope limit (hard):** at most 2 files — `deploy-staging.yml` + possibly one minor CDK/Nx config tweak if absolutely required. If CDK source code needs to change for caching to work, stop and reconsider.

**Stop-loss:** 2 working days from start of Phase 1. If improvement <20% total deploy time after 2 days → ship what's working, spawn follow-up backlog task, close PR 2.

**Acceptance:**

- Baseline documented in worklog (3 runs + deploy frequency).
- Each caching layer added justified by baseline data (not speculation).
- Cache validation test matrix passed (all rows correct).
- SHA256 Lambda verification step in place and green.
- Measured deploy time improvement documented in PR description (sample size: ≥3 no-op deploys post-merge).
- No correctness regressions (smoke test green, no cross-account cache pollution).

**Risks:**

- Cache key too loose → stale artifacts → broken deploy. Keys must include lockfile hashes, toolchain versions, arch.
- Nx cache masks bugs if declared inputs are incomplete.
- Native module platform mismatch on `node_modules/` cache restore.

---

## PR 3 — Tag-gated deploy

**Goal:** Deploy to staging only on git tag push (and `workflow_dispatch` safety valve), not on every merge to `develop`.

### Pre-requisite: ADR

**Before any workflow code change**, create `lore/2-adrs/NNNN_staging-deploy-trigger-strategy.md` (status: proposed). Answer concretely (not as a list of options):

1. **What is staging for?** Release candidate env (tag-gated) vs continuous develop mirror (current).
2. **Who tags and when?** Manual by dev post-merge, auto-tag on merge, nightly, on-demand before demo?
3. **Tag naming scheme?** Proposal: `staging-YYYY.MM.DD-N` (date-based, easy) vs `staging-vX.Y.Z` (semver).
4. **Hotfix flow?** Tag from hotfix branch or only from develop?
5. **What replaces "continuous staging"?** If develop no longer auto-deploys, how do devs test integration? Preview envs? Manual `workflow_dispatch`? Accepted staleness?

**ADR process:**

- Propose concretely (pick your answer, justify).
- Share with Filip. **Deadline: 5 working days.**
- No response → merge with note "no objections within review window; revisit if concerns arise".
- Objections → discuss, update, restart deadline **once**.
- Persistent disagreement → move PR 3 to blocked, continue 1 and 2. Do not let PR 3 stall the task.

### Implementation (only after ADR accepted)

1. Replace `on.push.branches: [develop]` with `on.push.tags: ['<agreed-pattern>']`. Keep `workflow_dispatch`.
2. Protect tag pattern in GitHub repo settings (prevent force-push / deletion). Document this as manual step in PR description.
3. Document tagging procedure in repo `README.md` or a new `docs/staging-deploy.md`.
4. Document rollback procedure: what to do if tagged deploy fails mid-flight. CloudFormation auto-rollback per stack is baseline; retag + manual redeploy is escalation path.
5. Optional (spawned task if valuable): scheduled `cdk diff` workflow to detect drift between staging and latest tag.

**Pre-merge test:** create a test tag (`staging-test-<sha>`), verify deploy triggers. Delete test tag after verification.

**Scope limit (hard):** 2 files + 1 ADR — `deploy-staging.yml` + docs file + `lore/2-adrs/NNNN_*.md`.

**Acceptance:**

- ADR merged (status: accepted).
- Push to `develop` does NOT trigger deploy.
- Push of tag matching agreed pattern DOES trigger deploy.
- `workflow_dispatch` still works.
- Tagging procedure documented in repo.
- Rollback procedure documented.
- Tag protection rules configured in repo settings.

**Risks:**

- Team disagreement → PR 3 blocks socially. Mitigated by ADR-first + deadline + blocked-status fallback.
- Devs forgetting to tag → staging becomes stale. Mitigation: clear team ritual or auto-tag automation (spawn follow-up task).
- Tag mutability → unreliable deploy history. Mitigation: protected tags in repo settings.
