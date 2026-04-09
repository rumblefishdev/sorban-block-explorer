# 2026-04-09 — Phase 0 baseline measurement

## Source data

Three successful staging deploy runs from 2026-04-08 (before Required
Reviewers gate was enabled). All triggered by `push` to `develop`.

- Run 1: [#24128010530](https://github.com/rumblefishdev/soroban-block-explorer/actions/runs/24128010530) — 09:22 UTC
- Run 2: [#24126952369](https://github.com/rumblefishdev/soroban-block-explorer/actions/runs/24126952369) — 09:06 UTC
- Run 3: [#24122360752](https://github.com/rumblefishdev/soroban-block-explorer/actions/runs/24122360752) — 06:59 UTC

## Per-step timings

### mirror-image job

| Step                   | Run 1   | Run 2   | Run 3   | Avg     |
| ---------------------- | ------- | ------- | ------- | ------- |
| Mirror Galexie image   | 9s      | 15s     | 10s     | 11s     |
| **mirror-image total** | **13s** | **20s** | **13s** | **15s** |

Note: all three runs hit the "Image already in ECR, skipping pull" path
(digest unchanged). A cold-mirror (new digest) would be longer (~60-120s
for docker pull + push).

### CDK deploy job

| Step                              | Run 1      | Run 2      | Run 3      | Avg        | % of total |
| --------------------------------- | ---------- | ---------- | ---------- | ---------- | ---------- |
| Set up job + checkout             | 4s         | 4s         | 4s         | 4s         | <1%        |
| setup-node                        | 9s         | 10s        | 10s        | 10s        | 2%         |
| rust-toolchain                    | 3s         | 1s         | 0s         | 1s         | <1%        |
| Swatinem/rust-cache restore       | 7s         | 9s         | 12s        | 9s         | 2%         |
| Install cargo-lambda (pip3)       | 8s         | 8s         | 9s         | 8s         | 2%         |
| npm ci                            | 47s        | 48s        | 46s        | 47s        | 10%        |
| Build CDK (nx build)              | 6s         | 7s         | 6s         | 6s         | 1%         |
| **CDK deploy (cdk deploy --all)** | **6m 14s** | **6m 15s** | **5m 51s** | **6m 7s**  | **76%**    |
| Smoke test                        | 3s         | 3s         | 2s         | 3s         | <1%        |
| Post-cache save + cleanup         | 4s         | 3s         | 2s         | 3s         | <1%        |
| **CDK deploy job total**          | **7m 46s** | **7m 49s** | **7m 26s** | **7m 40s** |            |

### Wall-clock total (both jobs sequential)

| Metric           | Run 1 | Run 2  | Run 3  | Avg       |
| ---------------- | ----- | ------ | ------ | --------- |
| Total wall-clock | 8m 4s | 8m 14s | 7m 44s | **8m 1s** |

## Bottleneck analysis

```
CDK deploy (CloudFormation): ███████████████████████████████████████  76%
npm ci:                      █████  10%
mirror-image:                █  2%
rust-cache restore:          █  2%
cargo-lambda install:        █  2%
setup-node:                  █  2%
Build CDK (nx build):        ░  1%
Everything else:             ░  5%
```

**CDK deploy (`cdk deploy --all`) is 76% of wall-clock.**

This is fundamentally different from what I expected. The dominant cost is
CloudFormation stack operations (create/update change sets, wait for
completion) — not build, not dependency install, not compilation.

## What this means for PR 2 caching strategy

### High ROI (attack the 76%)

1. **`cdk diff` as pre-step → skip deploy if no changes.**
   CloudFormation already no-ops unchanged stacks, but `cdk deploy`
   still waits for each stack's change set evaluation (~30-60s per stack
   even for no-op). Skipping the entire `cdk deploy --all` when diff is
   empty saves the full 6 minutes.

   This is not caching — it's **early termination**. The biggest win by far.

2. **Per-stack selective deploy.** If only one stack changed, deploy only
   that stack instead of `--all`. Requires knowing which stacks exist and
   which are affected. More complex but could save 4-5 min when only e.g.
   the API Lambda code changed.

### Medium ROI (attack the 10%)

3. **`node_modules/` cache.** npm ci takes 47s avg. Caching node_modules
   directly (keyed on package-lock.json + OS + arch + Node version) could
   reduce this to ~5s on cache hit. Net saving: ~42s.

### Low ROI (attack the <5%)

4. **Rust cache tuning** — already 9s, barely worth touching.
5. **cargo-lambda install** — 8s via pip3. Prebuilt binary saves ~6s. Marginal.
6. **Nx cache** — Build CDK is 6s. Caching saves <6s. Not worth the setup.
7. **setup-node** — 10s, mostly Node version download. Already cached via `cache: npm`.

### Not applicable

- **Rust build cache for Lambda binaries** — `Build CDK` step runs
  `npx nx build` which builds TypeScript only. Cargo-lambda build
  happens inside `cdk deploy` via CDK's `RustFunction` construct
  bundling step. Timing is **inside** the 6m 7s CDK deploy number.
  Hard to split out without CDK-level tracing.

## Deploy frequency (last 30 days)

```
31 runs total (but all in last 3 days — workflow is very new)
2026-04-07: 5 deploys
2026-04-08: 25 deploys
2026-04-09: 1 deploy (cancelled — Phase 0 test)
```

Frequency will normalize once initial setup stabilizes. Estimated
steady-state: ~1-3 deploys/week based on team size (3 devs) and project
phase (active build-out slowing to maintenance).

## Go/no-go decision for PR 2

**From G-subtask-breakdown.md:**

> Go/no-go gate: if total avg <5 min and deploys <2×/day, close PR 2
> as canceled: obsolete and move on.

**Measured:** avg 8m 1s (>5 min) and frequency is currently high.
**Decision: GO.** PR 2 proceeds.

**However** — the plan in G-caching-strategy.md was built around the
assumption that Rust compilation and npm install were the bottlenecks.
**They are not.** CDK deploy (CloudFormation) is 76%.

**Revised PR 2 focus (emerged from Phase 0):**

1. **Primary:** `cdk diff` early exit — skip `cdk deploy --all` when no
   stacks have changes. **This is the single biggest win.**
2. **Secondary:** `node_modules/` cache — saves ~42s.
3. **Drop:** Rust cache tuning, cargo-lambda prebuilt binary, Nx cache,
   SHA256 Lambda verification (all <10s savings each, combined <30s,
   not worth the complexity).

This is a **major plan revision** — the caching strategy note
(G-caching-strategy.md) needs to be updated to reflect this.
