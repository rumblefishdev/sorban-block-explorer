---
title: 'Quality gates — testing, rollback, scope, discipline'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [process, quality, testing, rollback]
links: []
history:
  - date: '2026-04-08'
    status: developing
    who: stkrolikiewicz
    note: 'Extracted from README during directory conversion; expanded with monitoring, cost, regression guards, post-merge validation.'
---

# Quality gates and process

Cross-cutting concerns that apply to all PRs. Not code — but ignoring them
is how "small CI change" turns into "broken staging for 3 hours".

## Pre-merge testing of workflow changes

**Problem:** `deploy-staging.yml` runs only on push-to-develop. Broken
workflow = broken staging immediately after merge.

**⚠️ Revised approach (emerged decision, see below).** Original plan was
to use `workflow_dispatch` from a feature branch ref to test changes
before merging. This was abandoned for two reasons:

1. The `staging` GitHub Environment has an existing branch policy
   restricting deploys to `develop` only. Allowing feature branches would
   require widening that policy — extra admin overhead, weakens the
   existing safety net, and risks staging being polluted by experimental
   feature-branch deploys.
2. For most PRs in this task, the change is small enough that
   careful code review + post-merge observation + revert-on-fail is
   strictly cheaper and not meaningfully less safe.

**Replacement strategy per PR:**

| PR   | Pre-merge check                                                                                 | Post-merge check                                                                                              |
| ---- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| PR 0 | `actionlint` + code review (1 line YAML)                                                        | Observe next deploy on develop is green                                                                       |
| PR 1 | `actionlint` + code review + grep for missed `us-east-1`                                        | Observe next deploy is green; SHA256 of any Lambda unchanged                                                  |
| PR 2 | `actionlint` + code review + cache validation test matrix simulated as much as possible locally | Watch next 3 deploys (Phase 0 baseline + organic) for cache correctness, SHA256 verification step, total time |
| PR 3 | `actionlint` + code review                                                                      | Throwaway test tag (`staging-test-<sha>`) on develop after merge to verify trigger; delete tag after          |

**Common across all PRs:**

- Run `actionlint .github/workflows/deploy-staging.yml` locally before commit. Catches ~90% of YAML/action mistakes for free, no infra needed.
- Self-review the diff line by line in PR view before requesting review.
- Have rollback command ready (in PR description).

**Why this is safe enough:**

- The Required Reviewers gate on `staging` environment (see below) means every deploy waits for manual approval — a broken workflow trying to run gets stopped at the gate, you click cancel instead of approve.
- All PRs in this task are revertable in <2 minutes via `git revert + push develop`.
- Post-merge validation periods (see "Post-merge validation period") catch issues before declaring task done.

## Required reviewers gate (operational, not in code)

For the duration of task 0110, request that repo admin enables
**Required reviewers** on the `staging` GitHub Environment with
`stkrolikiewicz`, `Efem67`, and `fikoayee` as approvers. Effect: every
deploy (push, manual, future tag) pauses at "Waiting for review" until
one of the reviewers manually approves.

This is the primary defense against accidental staging breakage during
the task. Zero code changes, reversible by toggling a checkbox.

**Action:** request via Slack/team chat with the prepared admin message.
Confirm enabled before starting PR 1.

**Branch policy (`develop` only) stays unchanged.** Do not request
widening to feature branches — see "Pre-merge testing" above for
rationale.

## Stop-loss for PR 2

Time budget: **2 working days from start of Phase 1.**

If after 2 days the measured improvement is <20% total deploy time reduction:

1. Ship whatever is working and justified by baseline data.
2. Spawn a follow-up backlog task for remaining optimization ideas.
3. Close PR 2 without forcing the 30% target.

This prevents the classic "one more cache key and it'll be faster" trap.

## Scope limits (hard)

Scope creep is the most common way small PRs become month-long rewrites.
Per PR:

- **PR 0:** 1 file. `deploy-staging.yml` only. ~3 lines.
- **PR 1:** 1 file. `deploy-staging.yml` only. Other `us-east-1` occurrences → spawned task.
- **PR 2:** at most 2 files. `deploy-staging.yml` + 1 minor Nx/CDK config tweak if absolutely required. Any deeper CDK change → stop, reconsider.
- **PR 3:** 2 files + 1 ADR. `deploy-staging.yml` + docs + `lore/2-adrs/NNNN_*.md`.

If you find yourself expanding scope mid-PR: **stop, commit current state, spawn a new task, narrow the PR.**

## Rollback plan

Every PR description **must** include an explicit rollback line:

> Rollback: `git revert <merge-commit>` → push develop → next deploy restores previous state.

Per-PR specifics:

- **PR 0:** trivial revert, no state to unwind.
- **PR 1:** trivial revert. GH variable can stay or be deleted later.
- **PR 2:** revert + optionally clear GH Actions caches via GH UI if stale entries are suspected to cause ongoing issues.
- **PR 3:** revert + manually remove protected-tag rules in repo settings + delete any test tags.

## Post-merge validation period

A PR is not "done" at merge. After each PR:

- **PR 0 / PR 1:** observe next deploy (manual or organic). Verify green. If green → done.
- **PR 2:** observe **next 3 deploys** (mix of cache-hit and cache-miss runs). Verify:
  - Timings match expected improvement.
  - SHA256 Lambda verification step is green on every run.
  - No cache-related errors in workflow logs.
  - Smoke test still passes.
  - If any check fails → revert immediately, do not try to fix forward in an unrelated PR.
- **PR 3:** observe **next 5 deploys or 1 week, whichever is longer**:
  - Tagging workflow actually gets used by the team (not just you).
  - No confusion about "why didn't my merge deploy to staging?" — if yes, update docs.
  - Rollback tested at least once (dry run: tag a known-good commit, verify behavior).

**Do not mark task completed until post-merge validation period is done for all PRs.**

## Success metrics discipline (PR 2)

"Deploy is faster" is not a metric. A valid success claim needs:

- **Sample size:** ≥3 no-op deploys + ≥3 meaningful-change deploys.
- **Measurement window:** within 1 week of PR 2 merge (to avoid confounding from unrelated changes).
- **Comparison baseline:** the Phase 0 baseline table.
- **Per-step breakdown:** not just total, so we know which cache actually helped.
- **Recorded in worklog** as `post-merge-validation-YYYY-MM-DD.md`.

If measurements are noisy or improvement can't be isolated, say so explicitly in the worklog. Do not overclaim.

## Cost tracking (optional but recommended)

GitHub Actions has minute and cache storage limits:

- 10 GB cache storage per repo (LRU eviction).
- Minute limits vary by plan.

Before and after PR 2, record:

- Total GH Actions cache size used (from repo settings → Actions → Caches).
- Average minutes per staging deploy (before/after).

If caching causes cache thrashing (constant eviction), it can paradoxically slow things down. Watch for this in post-merge validation.

## Regression guards

After PR 1 and PR 3 land, add a lightweight CI check to prevent regression.
Options:

1. **Grep-based lint step** in a new tiny workflow `ci-lint-workflows.yml`:

   ```yaml
   - name: Guard against hardcoded region
     run: |
       if rg -q 'us-east-1' .github/workflows/deploy-staging.yml; then
         echo "::error::deploy-staging.yml contains literal 'us-east-1' — use vars.AWS_REGION"
         exit 1
       fi
   ```

2. **Guard against `on.push.branches: [develop]` returning** to `deploy-staging.yml` after PR 3:

   ```yaml
   - name: Guard against push-to-develop trigger
     run: |
       if grep -A2 '^on:' .github/workflows/deploy-staging.yml | grep -q 'branches:'; then
         echo "::error::deploy-staging.yml should use tag trigger, not branch trigger"
         exit 1
       fi
   ```

These are 1-2 line checks that catch the most likely regressions from
future refactors. **Add them in the same PR as the fix** — not as a
follow-up task, because follow-ups never happen.

## ADR process for PR 3

- Draft ADR with a **concrete proposal**, not a list of options.
- Share with Filip (the only other developer per `lore/0-session/team.yaml`).
- **Response deadline: 5 working days.**
- No response → merge with note "no objections within review window; revisit if concerns arise".
- Objections → discuss, update ADR, restart deadline **once**.
- Persistent disagreement → move PR 3 to blocked, continue with PR 1/2. Do not let PR 3 stall the whole task.

## Worklog discipline

Update `worklog/` after each PR:

- **Facts:** what files changed, what numbers measured, what tests ran.
- **Emerged decisions:** anything decided without asking, especially cache keys, fallbacks, scope deviations. These are invisible debt if undocumented.

This is mandatory per `/lore-framework-tasks`. Writing incrementally is
much easier than reconstructing at task completion.

## Scope lock communication

`deploy-staging.yml` is a shared file. If someone else touches it while
PR 2 is in flight → merge conflicts + confusion about which improvement
broke things.

**Before starting PR 2 (the substantial one):**

- Notify Filip: "I'm modifying deploy-staging.yml for 0110 through [date]. Please hold conflicting changes or coordinate."
- Add note to lore session / team chat.
- Task 0103 (prod workflow) should not start until 0110 PR 1 and PR 2 are merged to avoid pattern drift.

## Hand-off for 0038

Task 0038 (CDK environment config) will introduce a central config module
for per-environment values. Region is a natural fit for that module, but
this task does NOT touch CDK internals.

**When 0038 becomes active, the person doing it must:**

1. Read this task's PR 1 — see how region is sourced (`vars.AWS_REGION` in GitHub `staging` environment).
2. Decide: consolidate region into the CDK config module, or leave workflow-level as-is.
3. Update `deploy-staging.yml` and `deploy-production.yml` accordingly within 0038's scope.
4. Add cross-reference to this task in 0038's README.

**Action for us:** when PR 1 merges, add a comment to 0038 README pointing at the merge commit so 0038's future implementer has context.

## Pre-flight checks (before opening each PR)

- [ ] **PR 0:** `workflow_dispatch` test manual-run works — verified against current workflow file
- [ ] **PR 1:** `infra/bin/staging.ts` / CDK entrypoint region hardcoding checked and documented
- [ ] **PR 1:** `AWS_REGION` variable exists in GitHub `staging` environment (admin confirmed)
- [ ] **PR 2:** Phase 0 baseline recorded (3 runs, per-step timings, deploy frequency)
- [ ] **PR 2:** `cargo-lambda` prebuilt binary download URL verified (linux-x86_64)
- [ ] **PR 2:** Which Rust crates actually compile in this workflow — documented in worklog
- [ ] **PR 2:** cargo-lambda target architecture (x86_64 vs aarch64) confirmed
- [ ] **PR 2:** `nx.json` `inputs`/`outputs` for CDK `build` target verified correct — or spawned PR to fix merged first
- [ ] **PR 2:** SHA256 Lambda verification step design confirmed
- [ ] **PR 2:** ROI sanity check passed (deploys ≥2×/day, total baseline >5 min)
- [ ] **PR 3:** ADR drafted with concrete proposal
- [ ] **PR 3:** ROI sanity check passed (deploys not already ≤1×/week organically)
- [ ] **PR 3:** Filip pinged about ADR — 5-day window started
- [ ] **All PRs:** scope lock communicated to team
- [ ] **All PRs:** rollback line in PR description
- [ ] **All PRs:** regression guard added in same PR where applicable
