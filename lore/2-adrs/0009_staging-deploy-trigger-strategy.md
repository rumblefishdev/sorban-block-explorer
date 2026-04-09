---
id: '0009'
title: 'Staging deploys triggered by git tags, not by push to develop'
status: accepted
deciders: [stkrolikiewicz, fmazur]
related_tasks: ['0110']
related_adrs: []
tags: [ci, cd, staging, process]
links:
  - .github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: proposed
    who: stkrolikiewicz
    note: 'ADR drafted as part of lore-0110 PR 3 prerequisite. 5-day review window opens on share.'
  - date: '2026-04-09'
    status: accepted
    who: stkrolikiewicz
    note: 'Accepted. Date-based tags staging-YYYY.MM.DD-N confirmed.'
---

# ADR 0009: Staging deploys triggered by git tags, not by push to develop

**Related:**

- [Task 0110: CI staging deploy optimization](../1-tasks/active/0110_FEATURE_ci-staging-deploy-optimization/README.md)

---

## Context

Currently `.github/workflows/deploy-staging.yml` deploys staging on every
push to `develop`. This means staging continuously mirrors the tip of
develop — every merge causes a full CDK deploy.

This has two problems:

1. **No control over what's in staging at any given moment.** Staging
   silently changes whenever anyone merges to develop, even for unrelated
   work (docs updates, lore edits, follow-up patches). Multiple PRs in
   close succession trigger overlapping deploys, racing the
   `concurrency: deploy-staging` group.
2. **Cost and noise.** Every merge consumes CDK deploy time, mirror-image
   ECR work, and CloudFormation churn. Most merges don't actually need to
   be tested in staging — they're pre-merged on local dev or just docs.

The team is small (3 developers): stkrolikiewicz, fmazur, plus collaborators.
Deploys to staging are not high-frequency; the project is in active build-out
where most work is local-first. Continuous-staging-as-mirror-of-develop is
not actually delivering value proportional to its overhead.

## Decision

**Staging deploys are triggered by git tags matching `staging-YYYY.MM.DD-N`,
not by push to `develop`.** Push-to-develop continues to run CI (lint /
test / build) but does NOT deploy.

### Concrete answers to open process questions

1. **What is staging for?**
   Release candidate environment. "I have a meaningful change ready to
   integration-test against real AWS resources". Not a continuous mirror
   of develop.

2. **Who tags and when?**
   The developer who wants to deploy. After their work is merged to
   develop and they have decided it's ready for integration testing,
   they create a tag manually:

   ```bash
   git checkout develop && git pull
   git tag staging-2026.04.08-1
   git push origin staging-2026.04.08-1
   ```

   No auto-tagging on merge — that just recreates the current "every
   merge is a deploy" problem with extra steps.

3. **Tag naming scheme:** `staging-YYYY.MM.DD-N`

   - Date-based, no version negotiation.
   - `N` is sequential within the day (1, 2, 3...).
   - Easy to read in `git tag -l 'staging-*'` listings.
   - No semantic-version negotiation overhead (the team is too small for
     SemVer to add value here).
   - Trivially sortable chronologically.

4. **Hotfix flow:**
   Tags can be created from any commit, not just `develop`. To deploy
   a hotfix from a non-develop branch:

   ```bash
   git checkout hotfix/whatever
   git tag staging-2026.04.08-99 # use high N to signal hotfix
   git push origin staging-2026.04.08-99
   ```

   The workflow trigger is `on.push.tags` so any tag matching the pattern
   fires the deploy regardless of which branch it points to.

5. **What replaces "continuous staging"?**
   - **Local development first.** Most iteration happens with local
     PostgreSQL + cargo-lambda local invoke.
   - **CI on develop pushes** continues to run lint/test/build (`ci.yml`).
     This catches breakage before tagging.
   - **Explicit tag = explicit "deploy this now"**. There is no implicit
     deploy. If you forgot to tag, staging didn't update — that is the
     intended behavior, not a bug.
   - **`workflow_dispatch`** (added by lore-0110 PR 0) remains as a
     safety valve for manually re-triggering a deploy without creating
     a new tag.

## Rationale

- **Explicit > implicit.** The cost of forgetting to tag (staging is
  slightly stale) is much lower than the cost of accidental deploys
  (broken staging during a demo, racing concurrent deploys, surprise
  changes).
- **Date-based tags require no negotiation.** Three developers don't
  need a SemVer release process. Date is monotonic and self-explanatory.
- **Hotfix-friendly.** Tag pattern works on any branch, no special
  handling needed.
- **Aligns with Required Reviewers gate.** Both safety mechanisms
  (lore-0110: env protection rules + tag gating) point in the same
  direction: deploys are deliberate human actions, not byproducts of
  routine development.
- **Reversible.** Switching back to push-trigger is a one-line YAML
  change if this turns out to be the wrong call.

## Alternatives considered

### A. Keep current behavior (push to develop)

Rejected: discussed above. Continuous staging is not delivering value
proportional to its cost.

### B. Auto-tag on merge to develop via workflow

Rejected: this is "every merge deploys" with extra steps. Doesn't solve
the original problem; just renames it.

### C. Nightly auto-tag

Rejected for staging (would make sense for production-like environments
with regular release cadence). For staging, even nightly is too automatic
— the whole point is to give developers explicit control.

### D. SemVer tags (`staging-vX.Y.Z`)

Rejected: requires version negotiation, extra ceremony for a 3-developer
team. Date-based is sufficient.

### E. Preview environments per PR

Rejected for this ADR scope: would solve different problem (per-PR
isolation). Considered for future task if/when team grows or PR
integration testing becomes a bottleneck. Not a substitute for tag-based
staging deploys; they could coexist.

## Consequences

### Positive

- Staging is a known-state environment until the next explicit tag.
- No accidental deploys from unrelated merges.
- Lower CI cost (fewer staging deploys per week).
- Clearer history: `git tag -l 'staging-*'` shows exactly what was deployed when.
- Lower CloudFormation churn → fewer transient issues from CDK deploy races.

### Negative

- Developers must remember to tag. Mitigation: document the procedure
  in repo `README.md` or `docs/staging-deploy.md`.
- Staging may drift from develop if no one tags for a while. Mitigation:
  this is the intended behavior. If staging drift becomes a problem, it
  signals that someone needs to tag — which is the desired feedback loop.
- New process to learn for the team. Mitigation: 3 developers; ~5 minute
  conversation.

### Neutral

- Tag protection in repo settings should be added (prevent
  force-push / deletion of `staging-*` tags) to keep deploy history
  reliable. Manual step, documented in PR description for lore-0110 PR 3.

## Validation plan

After implementation (lore-0110 PR 3):

1. Push to `develop` does NOT trigger a staging deploy (verify in
   GitHub Actions tab for first push after merge).
2. Pushing a `staging-test-<sha>` test tag DOES trigger a deploy
   (delete tag after verification).
3. `workflow_dispatch` from CLI still works as safety valve.
4. Document in repo: how to tag, how to roll back (revert + new tag).

## Open questions for review

- **Who else should be a deploy approver?** Currently the lore-0110
  Required Reviewers gate has stkrolikiewicz, fmazur (Efem67),
  fikoayee. Same set should apply to tag-deployed staging deploys
  (the gate is at GitHub Environment level, not workflow level, so
  this is automatic — but worth confirming).
- **Should we also tag-protect `staging-*` patterns?** Suggested yes
  to prevent accidental deletion / force-push. If accepted, this is a
  manual step for repo admin (adds <1 min to PR 3 implementation).

## Review window

This ADR has a **5-day review window** starting from the date it is
shared with the team. If no objections by then, it auto-promotes to
`accepted` and lore-0110 PR 3 implementation can proceed.

If objections raised: discuss, update ADR, restart deadline once. If
disagreement persists, lore-0110 PR 3 is moved to `blocked` status
and PR 1/PR 2 of lore-0110 continue without it.
