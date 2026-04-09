---
id: '0112'
title: 'CI: optimize ci.yml workflow (Rust + TypeScript — arm64, path filter, Nx cache, node_modules cache)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0110']
tags:
  [
    ci,
    rust,
    typescript,
    cargo-lambda,
    nx,
    performance,
    priority-low,
    effort-small,
  ]
milestone: 1
links:
  - .github/workflows/ci.yml
history:
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from lore-0110 discussion. Optimizations identified for ci.yml rust job; out of scope of 0110 (which targets deploy-staging.yml).'
---

# CI: optimize Rust workflow

## Summary

The `rust` job in `.github/workflows/ci.yml` is dominated by
`cargo lambda build --release --arm64`. Several independent optimizations
can cut wall-clock substantially with low risk — but several pre-flight
checks must pass first.

## Verified baseline

From actual run `#24140017053` (2026-04-08 14:12, master):

| Step                                          | Time        | % of total |
| --------------------------------------------- | ----------- | ---------- |
| Set up + checkout + toolchain + cache restore | ~3s         | <1%        |
| Install cargo-lambda (`pip3`)                 | 6s          | 1%         |
| `cargo fmt --check`                           | 1s          | <1%        |
| `cargo clippy --all-targets`                  | 1m 35s      | 16%        |
| `cargo test`                                  | 1m 59s      | 20%        |
| **`cargo lambda build --release --arm64`**    | **5m 47s**  | **59%**    |
| Post-cache save                               | 15s         | 3%         |
| **Total**                                     | **~9m 47s** | 100%       |

**Bottleneck:** lambda build (cross-compilation x86_64 → aarch64 via
`cargo-zigbuild`, full release optimization). Other slow steps share
artifacts in `target/` so they benefit from the same cache strategy.

## Pre-flight checks done

| Check                                      | Result                                                                                                                         |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Branch protection on `master`              | ❌ none (gh api 404). Path filter is safe.                                                                                     |
| Branch protection on `develop`             | ❌ none. Same.                                                                                                                 |
| Repo rulesets                              | empty                                                                                                                          |
| Lambda crates                              | 4: `api`, `db-migrate`, `db-partition-mgmt`, `indexer`                                                                         |
| Library crates                             | 3: `db`, `domain`, `xdr-parser`                                                                                                |
| `cargo test` scope                         | runs full workspace including lambda crates → splitting test/build into 2 jobs causes double-compile. **Don't split.**         |
| **GH Actions cache size**                  | **9.8 GB / 10 GB (98%)** — 11 active caches. Adding new cache directories will trigger LRU eviction. **Cleanup needed first.** |
| arm64 runner labels                        | `ubuntu-24.04-arm`, `ubuntu-22.04-arm` — confirmed GA via `actions/partner-runner-images`                                      |
| arm64 free for THIS public repo in 2026-04 | ⚠️ **uncertain** — see "Pre-flight checks remaining" below                                                                     |
| `cargo-lambda` latest version              | v1.9.1 (verified via api.github.com)                                                                                           |
| `pip3 install cargo-lambda` time           | only 6s in baseline (not 30-60s) → prebuilt-binary swap not worth doing                                                        |

## Pre-flight checks remaining (block implementation)

These must be resolved **before** opening the implementation PR:

- [ ] **Confirm arm64 runner is free for this public repo as of 2026-04.** Method: 1. Create throwaway workflow on a test branch with `runs-on: ubuntu-24.04-arm` and a 30s job. 2. Note GH Actions billing/minutes used before run. 3. Trigger run, wait for success. 4. Check minutes-used delta. If 0 → free, proceed. If charged → pivot (see below).
      Sources are ambiguous: 2025-01-16 blog announced free in public preview, but billing docs in 2026-04 do not unambiguously confirm continued free status.
- [ ] **Decide cache `save-if` branch.** `master` (default) vs `develop` (where work happens). Check rhythm of develop → master merges via `git log master...develop`. If frequent (>1/week) → `master`. If rare → `develop` or both.
- [ ] **Cache cleanup.** Free up space in the 10 GB cache budget. Either: - Delete old / unused caches via `gh api -X DELETE /repos/.../actions/caches/<id>` - Or shorten `actions/cache` retention (default 7 days idle eviction) - Document size before/after in worklog
- [ ] **Measure cache hit rate** of current `Swatinem/rust-cache` setup over 5 recent runs. If already >90%, cache tuning ROI is low — focus on arm64 runner.

## Proposed changes (in ROI order, conditional on pre-flight)

### 1. Native arm64 runner (biggest expected win)

**Conditional on:** arm64-is-free verification.

```yaml
rust:
  name: Rust (fmt, clippy, test, lambda build)
  runs-on: ubuntu-24.04-arm # native arm64 — free for public repos (verify in pre-flight)
  env:
    SQLX_OFFLINE: 'true'
  steps:
    # ... unchanged
    - run: cargo lambda build --release --arm64
```

On a native arm64 runner, `cargo lambda build --release --arm64` becomes a
**native build** — no `cargo-zigbuild`, no zig install, no cross-compile
overhead. Expected reduction: **5m 47s → 1-2 min** for lambda build step
alone (3-5x speedup based on similar projects, to be verified empirically).

**Caveat:** GitHub may queue arm64 jobs longer than x86_64 during peak.
Acceptable for non-blocking CI. Monitor for 1 week post-merge.

**No need to split into two jobs** — pre-flight check confirmed `cargo test`
covers lambda crates, so splitting causes double-compile. Single arm64 job
benefits all steps (test, clippy, lambda build) via shared `target/`.

### 2. Path filter — skip rust job when no rust changes

**Safe** (verified: no branch protection, no required checks).

```yaml
on:
  push:
    branches:
      - master
  pull_request:
    paths:
      - 'crates/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'rust-toolchain.toml'
      - '.github/workflows/ci.yml'
```

**Limitation:** this filter covers the entire workflow including
TypeScript job. If you want to filter only the rust job, use
`dorny/paths-filter` for granular per-job filtering, or split into two
workflow files. Recommend the simple top-level filter unless you need
TypeScript-only PRs to also skip rust validation (which is the current
behavior anyway).

**Expected impact:** ~50-90% of PRs don't touch rust → those PRs skip the
~10-minute job entirely.

### 3. Cache tuning

**Conditional on:** cache cleanup pre-flight done.

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    shared-key: 'rust-${{ runner.os }}-${{ runner.arch }}'
    cache-targets: 'true'
    cache-directories: |
      ~/.cache/cargo-zigbuild
      target/lambda
    save-if: ${{ github.ref == 'refs/heads/master' }} # or develop, decide pre-flight
```

`shared-key` includes `runner.arch` so x86_64 and arm64 don't share cache
(different artifacts). `cache-directories` adds zigbuild + cargo-lambda
output dirs. `save-if` restricts cache writes to default branch — PRs
read from latest, don't pollute.

**Expected impact:** 10-30% improvement on warm cache. Less than arm64
runner switch.

### NOT in scope (skipped after baseline analysis)

- **Prebuilt cargo-lambda binary.** Saves 6s. Not worth the change.
- **`sccache` with GHA backend.** Speculative; only consider if (1)+(2)+(3) plateau.
- **`--profile release-ci` with reduced opt-level.** Hides bugs; testing purpose of CI defeated.
- **Splitting into rust-quality + rust-lambda-build jobs.** Pre-flight confirmed `cargo test` covers lambda crates → double-compile cost > parallelism benefit.

## Out of scope (delegated)

- `.github/workflows/deploy-staging.yml` → owned by task **0110 PR 2**. Caching strategies should be coordinated — adopt the same prebuilt-binary / cache-key conventions if 0110 PR 2 lands first.
- `.github/workflows/deploy-production.yml` → owned by task **0103**.
- Splitting crates / changing Rust source structure.
- Replacing `cargo-lambda` with another tool.

## Coordination with 0110

Task 0110 PR 2 will introduce caching for `deploy-staging.yml` rust steps.
Decisions to align:

- Cache key naming convention (`shared-key` format)
- `cache-directories` list (what extra paths to cache)
- `save-if` strategy

**Sequencing:** 0110 PR 2 should land first. 0112 then adopts whatever
key/directories convention 0110 PR 2 establishes. Avoids two unrelated PRs
inventing different conventions.

## Acceptance criteria

- [ ] Pre-flight checks all resolved (see list above)
- [ ] Wall-clock for typical PR rust job ≤ 4 minutes (baseline 9m 47s, target ≥50% reduction)
- [ ] PRs that don't touch `crates/**` etc. skip the rust job entirely
- [ ] Cache size used after merge is documented (must stay under 10 GB after cleanup)
- [ ] No correctness regression — `cargo lambda build` produces a valid Lambda artifact (verify via SHA256 of zip output if possible, otherwise manual smoke test)
- [ ] Each optimization is justified by measured improvement, not speculation
- [ ] Worklog with before/after timings for at least 3 runs each
- [ ] Coordination with 0110 PR 2 documented (which lands first, which conventions adopted)

## Stop-loss

**Time budget: 1 working day from start.** If after that improvement is
<30% wall-clock reduction:

1. Ship whatever is justified by measurements.
2. Spawn follow-up backlog task for remaining ideas (sccache, etc.).
3. Close 0112 without forcing higher targets.

## Risks

- **arm64 runner not actually free** → pre-flight catches this, pivot to (2)+(3) only. Wall-clock target drops to 6-7 min.
- **arm64 runner queue waits** → mitigated by 1-week post-merge monitoring; fallback to x86_64 if queues are unacceptable.
- **Cache cleanup deletes a cache that was actually being used** → might temporarily slow down some other workflow (typescript). Acceptable; recovers on next run.
- **Path filter false negative** → workflow change to ci.yml self-references in the filter, so changes to the workflow itself trigger it. Other false negatives are caught on master push.
- **Cargo registry cache invalidation by `shared-key` change** → first run after merge will be cold cache, slower. Document expected first-run impact in PR description.

## Rollback

Single PR. `git revert <merge-commit>` → push develop → next CI run uses
prior workflow. Cache differences resolve themselves on next run. No state
to clean up.

## Why low priority

Quality-of-life improvement. Current CI works, just slower than necessary.
~10 min CI per PR with rust changes is annoying but not a blocker. Pick up
after 0110 PR 2 lands so caching conventions align.
