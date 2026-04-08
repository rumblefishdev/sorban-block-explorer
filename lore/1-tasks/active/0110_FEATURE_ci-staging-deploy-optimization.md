---
id: '0110'
title: 'CI: staging deploy optimization — region var, caching, tag-gating'
type: FEATURE
status: active
related_adr: []
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
---

# CI: staging deploy optimization

**Scope: staging only.** Three independent improvements to
`.github/workflows/deploy-staging.yml`, bundled as one task but implemented
as **3 separate PRs** (ordered by risk, low → high).

A parallel task **0103** covers the same three improvements for the production
deploy workflow. Staging acts as pilot; production reuses patterns validated here.

## Related tasks

- **0039** — parent task, created `deploy-staging.yml` (archived).
- **0038** — CDK environment config module. **Coordination needed for subtask 1**: region could live in CDK config instead of (or alongside) GitHub variable. See subtask 1 notes.
- **0103** — production deploy workflow. Scope was extended to mirror this task's three improvements. 0110 should land first, 0103 reuses patterns.

## Motivation

Current staging deploy has three pain points:

1. `us-east-1` hardcoded in multiple places → brittle to region change.
2. Every push to `develop` triggers full CDK deploy of all stacks, even when nothing changed → slow, noisy, wastes CI minutes.
3. No way to control _when_ staging gets updated — staging drifts with every merge.

## Ground rules (set by discussion before task creation)

- **Measure before optimizing.** Phase 0 baseline is mandatory — no caching work without numbers.
- **Do NOT cache `cdk.out`.** It contains account-specific synthesized templates and asset hashes; cache hits across accounts/contexts are a correctness risk. Cache _inputs_ to synthesis (node_modules, cargo target, rust artifacts), not outputs.
- **Each subtask = one PR.** Revertable independently.
- **Tag-gating is a process decision, not just a workflow change** — requires ADR + team agreement before implementation.

---

## Subtask 1 — Region as GitHub variable (low risk, do first)

**Goal:** Remove all literal `us-east-1` from staging deploy pipeline; source from `vars.AWS_REGION` in the `staging` GitHub Environment.

**⚠️ Coordination with 0038:** task 0038 introduces a central CDK config module
(`infra/aws-cdk/lib/config/`) with per-environment values. This subtask is
scoped to the workflow file only: region goes into `vars.AWS_REGION` (GitHub),
workflow reads from there. CDK-internal region stays untouched and can be
revisited when 0038 lands.

**Steps:**

1. Grep the **entire repo** for `us-east-1` — not just `deploy-staging.yml`. Check:
   - `.github/workflows/*.yml`
   - `infra/bin/staging.ts` and any CDK env config
   - SSM parameter paths, ARN strings, scripts
2. Create `AWS_REGION` variable in GitHub `staging` environment (default `us-east-1`) — requires repo admin. **Must exist before merging PR**, otherwise workflow breaks.
3. Replace in `.github/workflows/deploy-staging.yml`:
   - Line 26: `aws-region: ${{ vars.AWS_REGION }}`
   - Line 42: `aws ecr get-login-password --region ${{ vars.AWS_REGION }}`
   - Line 101: `aws-region: ${{ vars.AWS_REGION }}`
4. If CDK code also hardcodes region, decide: env var (`AWS_REGION` / `CDK_DEFAULT_REGION`) vs context parameter. Document choice.
5. Verify staging deploy still passes end-to-end.

**Acceptance:**

- `rg 'us-east-1' .github/workflows/deploy-staging.yml` → no matches.
- Full repo grep for `us-east-1` documented; any remaining occurrences justified in PR description.
- Staging deploy green.

**Risks:**

- Forgetting to create the GH variable → workflow fails on first run. Mitigation: create variable first, merge second.
- CDK synthesis may also need region → check `infra/` for hardcoded values.

---

## Subtask 2 — Deploy caching (measurement-driven)

**Goal:** Reduce deploy time for no-op / small-change deploys by caching synthesis _inputs_, not outputs.

### Phase 0: Baseline (mandatory)

Before any optimization, run staging deploy 3× and record per-step timings:

- `mirror-image` job total
- `setup-node` + `npm ci`
- `rust-toolchain` + `rust-cache` restore
- `cargo-lambda` install
- `nx build` CDK
- `cdk deploy --all` (per-stack if possible)
- `smoke test`

Document in task worklog. **This tells us where the actual bottleneck is** — optimization without this is guessing.

### Phase 1: Cache inputs (only after baseline shows they matter)

Candidate caches (apply only if Phase 0 shows they're bottlenecks):

- **`node_modules`** — already via `cache: npm`. Verify cache hit rate.
- **`target/` (Rust)** — `Swatinem/rust-cache` already present. Verify hit rate; may need cache key tuning.
- **cargo-lambda binary** — `pip3 install cargo-lambda` on every run. Cache the pip install or pin to a prebuilt binary.
- **Nx cache** — `.nx/cache` can be cached with `actions/cache` keyed on source hash. Benefit: skip `nx build` when CDK sources unchanged.
- **Mirror-image skip logic** — already implemented (line 51). Verify it works and short-circuits correctly.

### Phase 2: Deploy-time optimization

- **DO NOT cache `cdk.out`.** (See ground rules.)
- Consider `cdk diff --all` as a pre-step; if diff is empty for all stacks, skip `cdk deploy` entirely. CloudFormation already no-ops unchanged stacks, but `cdk deploy` still waits on each — skipping saves the roundtrip.
- Do **not** introduce `nx affected` here — single CDK project, no benefit, adds complexity with tag-based triggers.

**Acceptance:**

- Baseline documented in worklog with 3 runs.
- Each caching change justified by baseline data (not speculation).
- Second no-op deploy measurably faster than baseline (target: ≥30% reduction, but final target set AFTER baseline).
- No correctness regressions (smoke test still green, no cross-account cache pollution).

**Risks:**

- Cache key too loose → stale artifacts used → broken deploy. Keys must include lockfile hashes, toolchain versions, `Cargo.lock`.
- Nx cache can mask bugs if sources aren't captured in hash.

---

## Subtask 3 — Tag-gated deploy (process decision + ADR required)

**Goal:** Deploy to staging only on git tag push, not on every merge to `develop`.

### Pre-requisite: ADR

**Before writing any workflow code**, create ADR answering:

1. **What is staging for now?**
   - (a) Continuous mirror of develop (current behavior)
   - (b) Release candidate environment (tag-gated)
   - These are fundamentally different — team must agree.
2. **Who tags and when?**
   - Manual by developer after merge?
   - Auto-tag on merge via workflow?
   - Nightly auto-tag?
   - Only on demand before demo/release?
3. **Tag naming scheme?**
   - Proposals: `staging-YYYY.MM.DD-N` (date-based, easy to read) vs `staging-vX.Y.Z` (semver, needs version bumps).
4. **Hotfix flow?**
   - Can we tag from a hotfix branch, or only from develop?
5. **What replaces "continuous staging"?**
   - If develop no longer auto-deploys, how do devs test integration? Preview envs? Manual `workflow_dispatch`?

**ADR location:** `lore/2-adrs/NNNN_staging-deploy-trigger-strategy.md`
**Status:** proposed → accepted (needs team sign-off, at minimum stkrolikiewicz + fmazur).

### Implementation (only after ADR accepted)

1. Replace `on.push.branches: [develop]` with `on.push.tags: ['<agreed-pattern>']`.
2. Add `workflow_dispatch` for manual redeploy (safety valve).
3. Protect tags in GitHub repo settings (prevent force-push / deletion).
4. Document tagging procedure in wiki / README.
5. Consider drift detection: since deploys become rarer, config drift grows. Optional: scheduled `cdk diff` workflow that alerts if staging != tagged commit.
6. Rollback strategy: document what to do if tagged deploy fails mid-flight. CloudFormation auto-rollback per stack is probably enough, but confirm and write it down.

**Acceptance:**

- ADR merged and accepted.
- Push to `develop` does NOT trigger deploy.
- Push of tag matching agreed pattern DOES trigger deploy.
- `workflow_dispatch` works for manual runs.
- Tagging procedure documented.
- Rollback procedure documented.

**Risks:**

- Team disagreement on staging purpose → task blocked until resolved. This is why ADR is upfront.
- Devs forgetting to tag → staging becomes stale. Mitigation: auto-tag on merge, or clear team ritual.
- Broken tag immutability → deploy history becomes unreliable. Mitigation: protect tags.

---

## Ordering & ROI

1. **Subtask 1 (region var)** — low risk, low effort, unblocks future multi-region work. Do first.
2. **Subtask 2 (caching)** — measurement first, optimization second. ROI depends on deploy frequency; document in Phase 0.
3. **Subtask 3 (tag-gating)** — requires team alignment via ADR. Cannot be parallelized with 1 or 2 safely (don't change trigger while also changing caching — too many variables if something breaks).

**ROI sanity check before starting subtask 2:** If staging deploys <2×/day, caching work may not pay off vs. other priorities. Document current deploy frequency in worklog and decide whether to proceed.

## Out of scope

- Production deploy workflow (separate task if needed — grep results from subtask 1 may surface it).
- Preview environments per PR.
- Multi-region deploy.
- Replacing CDK with something else.

## Open questions (to resolve during work)

- [ ] Does `infra/bin/staging.ts` hardcode region? (answers subtask 1 scope)
- [ ] Current mean deploy time and frequency? (answers subtask 2 ROI)
- [ ] Team consensus on staging purpose? (answers subtask 3 ADR)
