---
title: 'Deploy optimization strategy (revised after Phase 0 baseline)'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [cdk, caching, ci]
links:
  - ../../../.github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: developing
    who: stkrolikiewicz
    note: 'Extracted from README during directory conversion.'
  - date: '2026-04-09'
    status: developing
    who: stkrolikiewicz
    note: 'Major rewrite after Phase 0 baseline. Original plan assumed Rust/npm were bottlenecks. Baseline shows CDK deploy (CloudFormation) is 76% of wall-clock. Rust/cargo-lambda/Nx caching dropped from scope.'
---

# Deploy optimization strategy for PR 2

**⚠️ This note was fully rewritten on 2026-04-09** after Phase 0 baseline
measurement invalidated the original caching plan. See
`worklog/2026-04-09-phase0-baseline.md` for raw data.

## Phase 0 baseline summary

Average staging deploy wall-clock: **8m 1s** (3 runs).

| Step                 | Avg   | % of total | Cacheable?                                                 |
| -------------------- | ----- | ---------- | ---------------------------------------------------------- |
| **CDK deploy --all** | 6m 7s | **76%**    | No (CloudFormation server-side). Skippable via `cdk diff`. |
| npm ci               | 47s   | 10%        | Yes (`node_modules/` cache)                                |
| mirror-image         | 15s   | 2%         | Already optimized (skip if digest unchanged)               |
| rust-cache restore   | 9s    | 2%         | Already cached                                             |
| cargo-lambda install | 8s    | 2%         | Marginal (8s saving not worth work)                        |
| Build CDK (nx build) | 6s    | 1%         | Marginal (6s saving not worth work)                        |
| Everything else      | ~49s  | 7%         | Setup overhead, not cacheable                              |

**Key insight:** the bottleneck is **CloudFormation operations** (change
set creation, stack update wait, rollback detection), not compilation or
dependency installation. Caching build inputs gives <30s total savings —
below the noise floor.

## Revised ROI ranking

| Rank | Optimization                     | Expected saving | Risk   | Notes                                                                                                        |
| ---- | -------------------------------- | --------------- | ------ | ------------------------------------------------------------------------------------------------------------ |
| 1    | **`cdk diff` early exit**        | **~6 min**      | low    | Skip entire `cdk deploy` when no stack changes                                                               |
| 2    | **`node_modules/` cache**        | ~42s            | medium | Native module platform risk                                                                                  |
| 3    | ~~Nx cache~~                     | ~~<6s~~         | —      | **Dropped.** Build CDK is 6s. Not worth it.                                                                  |
| 4    | ~~Rust cache tuning~~            | ~~<9s~~         | —      | **Dropped.** Already 9s, no room.                                                                            |
| 5    | ~~cargo-lambda prebuilt binary~~ | ~~<8s~~         | —      | **Dropped.** 8s via pip, not worth change.                                                                   |
| 6    | ~~SHA256 Lambda verification~~   | ~~N/A~~         | —      | **Dropped.** Was guard for Rust cache stale-binary risk. With Rust cache out of scope, guard is unnecessary. |

## Optimization 1 — `cdk diff` early exit (primary)

**Idea:** before running `cdk deploy --all`, run `cdk diff --all`. If
no stacks have changes, skip the deploy entirely.

```yaml
- name: CDK diff
  id: cdk_diff
  run: |
    cd infra
    DIFF_OUTPUT=$(npx cdk --app "node dist/bin/staging.js" diff --all 2>&1) || true
    if echo "$DIFF_OUTPUT" | grep -q "There were no differences"; then
      echo "has_changes=false" >> $GITHUB_OUTPUT
      echo "No stack changes detected — skipping deploy."
    else
      echo "has_changes=true" >> $GITHUB_OUTPUT
      echo "Stack changes detected:"
      echo "$DIFF_OUTPUT"
    fi
  env:
    CDK_DEFAULT_ACCOUNT: ${{ secrets.AWS_ACCOUNT_ID }}

- name: CDK deploy
  if: steps.cdk_diff.outputs.has_changes == 'true'
  run: |
    cd infra && npx cdk --app "node dist/bin/staging.js" deploy --all \
      --require-approval never \
      -c galexieImageTag=${GITHUB_SHA}
  env:
    CDK_DEFAULT_ACCOUNT: ${{ secrets.AWS_ACCOUNT_ID }}
```

### Caveats

1. **`cdk diff` itself takes time** — it must synthesize all stacks and
   compare with deployed state. Estimated ~30-60s (synthesis + API calls).
   Net saving on no-op deploy: 6m 7s - 60s = **~5 min**.
2. **`cdk diff` exit code** — CDK diff returns exit code 1 when there ARE
   differences (not when there's an error). The `|| true` prevents the
   step from failing. Parse stdout instead.
3. **`cdk diff` with context params** — must pass the same `-c` flags as
   `cdk deploy` to get accurate diff. If `galexieImageTag` changes every
   time (it's `${GITHUB_SHA}`), diff will always show a change even if
   code is identical. **This is a problem.** Need to investigate whether
   the image tag context value is embedded in stack template or just
   passed through.
4. **False negative risk** — if `cdk diff` misses a change, deploy gets
   skipped when it shouldn't. CloudFormation drift outside CDK control
   (manual console changes) would also be missed. Acceptable risk for
   staging.
5. **Doesn't help when there ARE changes** — the common case during
   active development IS "there are changes". This optimization only
   saves time for organic pushes (docs, lore, unrelated merges) that
   don't affect infra. Value depends on what percentage of pushes to
   develop are infra-relevant.

### `galexieImageTag` problem (critical)

The current deploy command passes `-c galexieImageTag=${GITHUB_SHA}`.
Every commit has a different SHA, so every `cdk diff` will report
a change in the ECS task definition (new image tag). This means
**`cdk diff` will never report "no differences"** even for pure
docs/lore commits.

Possible solutions:

- (a) **Don't pass `galexieImageTag` to `cdk diff`** — diff only the
  "structural" template without image tag. Then deploy always passes it.
  Risk: diff misses actual Galexie config changes that include tag.
- (b) **Pin `galexieImageTag` to a stable value for diff** — e.g. the
  currently deployed tag from `aws ecs describe-task-definition`.
  Complex but accurate.
- (c) **Accept that CDK diff always reports changes** when image tag
  is in context → optimization only works if we restructure how image
  tag is passed (separate from `cdk deploy`). Larger change.
- (d) **Skip diff approach entirely** — accept the 6-minute CDK deploy
  as cost of business for staging. Focus PR 2 on node_modules cache
  only (42s saving). Honest, low-effort.

**This is a blocking design decision for PR 2.** Until resolved, the
`cdk diff` optimization cannot be implemented.

## Optimization 2 — `node_modules/` cache (secondary)

```yaml
- name: Cache node_modules
  uses: actions/cache@v4
  with:
    path: node_modules
    key: node-modules-${{ runner.os }}-${{ runner.arch }}-node${{ steps.setup-node.outputs.node-version }}-${{ hashFiles('package-lock.json') }}

- name: Install dependencies
  if: steps.cache-node-modules.outputs.cache-hit != 'true'
  run: npm ci
```

Expected saving: ~42s (npm ci avg 47s → ~5s on cache hit for hash
verification).

**⚠️ Cache size concern.** Repo is at 9.8 GB / 10 GB cache limit.
`node_modules/` for this project is likely 200-500 MB. Adding it will
cause LRU eviction of existing caches. Needs cache cleanup first
(see pre-flight checks in G-quality-gates.md).

**Native module risk** remains — key includes `runner.os`, `runner.arch`,
and Node version to mitigate.

## What was dropped from original plan (and why)

The original `G-caching-strategy.md` (2026-04-08 version) contained
detailed analysis of Rust-specific caching, cargo-lambda binary, Nx
task cache, sccache, SHA256 Lambda verification, and a 6-row cache
validation test matrix. **All of this is dropped** because:

1. Phase 0 baseline showed these steps collectively are <2 minutes
   (mostly <30s each). Optimizing them saves <30s total.
2. Each optimization carried non-trivial risk (stale binary, native
   module mismatch, cache pollution).
3. The complexity budget (test matrix, SHA256 verification, pre-flight
   checks for Nx config) far exceeded the savings.

The correct senior response to "bottleneck is somewhere else" is to
**stop optimizing the wrong thing**, not to optimize it anyway "because
we already have a plan."

## Revised acceptance criteria for PR 2

- [ ] Phase 0 baseline documented (done — see worklog)
- [ ] Design decision on `galexieImageTag` problem resolved
- [ ] If `cdk diff` approach is viable: no-op deploy skips CDK deploy step, wall-clock ≤ 2 min
- [ ] If `cdk diff` approach is NOT viable: `node_modules/` cache implemented, wall-clock reduced by ~42s (target ≤ 7m 20s)
- [ ] Cache size stays under 10 GB (cleanup done before merge if needed)
- [ ] No correctness regressions (smoke test passes, all stacks deployed when there ARE changes)

## Decision needed

Before implementing PR 2, choose one of:

**(A)** Invest in solving `galexieImageTag` problem → `cdk diff` early exit. Biggest win (~5 min) but requires design work.

**(B)** Accept 6 min CDK deploy as unavoidable → implement only `node_modules/` cache (~42s saving). Minimal effort, honest about what's achievable.

**(C)** Cancel PR 2. Combined savings of (B) are <1 minute. For 1-3 deploys/week, that's <3 minutes saved per week. ROI is arguably negative given implementation + maintenance cost. Move on to PR 3.

**(D)** Hybrid — implement (B) now, spawn follow-up task for `galexieImageTag` investigation as a separate concern. Ship the easy win, defer the hard problem.
