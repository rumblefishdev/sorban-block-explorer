---
title: 'Caching strategy — Rust, cargo-lambda, Node, Nx'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [caching, rust, nx, cargo-lambda, ci]
links:
  - ../../../.github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: developing
    who: stkrolikiewicz
    note: 'Extracted from README during directory conversion.'
---

# Caching strategy for PR 2

Deep dive on what to cache, in what order, with what keys, and how to
verify correctness. **No caching work starts before Phase 0 baseline is
documented in worklog** (see G-subtask-breakdown.md → PR 2 → Phase 0).

## ROI ranking (to verify against Phase 0)

| Rank | Layer                              | Expected win    | Stale-binary risk       | Notes                                           |
| ---- | ---------------------------------- | --------------- | ----------------------- | ----------------------------------------------- |
| 1    | **Nx task cache** (`.nx/cache`)    | biggest if >30s | none (deterministic)    | Requires correct `nx.json` inputs/outputs first |
| 2    | **cargo-lambda prebuilt binary**   | certain ~30-60s | none                    | Replaces `pip3 install cargo-lambda`            |
| 3    | **Rust build cache** (tuned)       | variable        | **real**                | SHA256 Lambda verification MANDATORY            |
| 4    | **`node_modules/` cache**          | moderate        | medium (native modules) | Key must include OS + arch + Node version       |
| 5    | **`cache: npm`** (already present) | marginal        | none                    | Keep as-is                                      |

## Rust-specific caching

The existing workflow uses `Swatinem/rust-cache@v2` with default settings.
This is a **starting point, not a solution** — several Rust-specific
concerns need verification.

### 1. Which crates actually build in CI?

The `crates/` directory contains ~7 crates (`domain`, `xdr-parser`, `db`,
`db-migrate`, `api`, `db-partition-mgmt`, `indexer`). Not all are Lambda
functions — some are libraries, some CLIs. Before tuning cache, identify
which subset `nx build @rumblefish/soroban-block-explorer-aws-cdk`
actually compiles and packages via cargo-lambda. **Document in worklog.**
Caching crates that aren't built wastes the 10 GB GitHub Actions cache limit.

### 2. cargo-lambda uses cross-compilation

`cargo-lambda` builds via `cargo-zigbuild` to `aarch64-unknown-linux-gnu` or
`x86_64-unknown-linux-gnu`. Outputs land in:

- `target/<target-triple>/release/` — cross-compiled binaries
- `target/lambda/` — cargo-lambda's packaged artifacts
- `~/.cache/cargo-zigbuild/` — zig toolchain downloads

Default `Swatinem/rust-cache` key is based on `Cargo.lock`, `rustc version`,
and workflow context. It **may not include target triple**, risking false
hits when changing arch. Fix:

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    shared-key: 'lambda-<arch>' # explicit, stable
    cache-targets: 'true'
    cache-on-failure: 'false'
    save-if: ${{ github.ref == 'refs/heads/develop' }}
```

Additionally: cache `target/lambda/` and `~/.cache/cargo-zigbuild/`
separately via `actions/cache` if rust-cache doesn't cover them.

### 3. Three cache layers with different ROI

| Layer                        | Contents            | Change frequency         | ROI                                 |
| ---------------------------- | ------------------- | ------------------------ | ----------------------------------- |
| `~/.cargo/registry/cache/`   | downloaded tarballs | low                      | high hit rate, always worth caching |
| `~/.cargo/registry/index/`   | sparse index        | near-zero (sparse index) | skip — modern cargo handles this    |
| `target/` + `target/lambda/` | build artifacts     | high                     | needs careful key tuning            |

`Swatinem/rust-cache` handles all three, but knowing the breakdown helps
debug cache misses.

### 4. sccache as fallback — only if rust build dominates baseline

If Phase 0 shows rust compilation is >60% of deploy time and rust-cache
tuning plateaus, consider `sccache` with GHA backend. Granular per
compilation unit, not whole-`target/`. Setup cost: `RUSTC_WRAPPER` config +
sccache install. **Do not pursue speculatively** — only if rust-cache fails
to close the gap within the 2-day stop-loss.

### 5. Stale cache = silently broken Lambda (critical)

A too-loose cache key can reuse an old `target/` for new source code. The
Lambda zip gets built from stale compiled artifacts. CloudFormation sees
"same function, new zip" and deploys it. `/health` smoke test passes
because the old code still boots. **Result: deployed Lambda is running
yesterday's code. This is the nightmare failure mode.**

**Mandatory mitigations (required for PR 2 merge):**

1. Cache key MUST include hash of `Cargo.lock` AND all `crates/**/*.rs`
   files (or equivalent canary file set).
2. **Post-deploy SHA256 verification step.** After `cdk deploy`:
   - Compute SHA256 of the built Lambda zip locally (in CI runner).
   - Read deployed Lambda's `CodeSha256` via `aws lambda get-function`.
   - **Fail the workflow if they don't match.** This catches stale cache AND other classes of deploy bug (e.g. CDK skipping asset upload silently).
3. Test matrix row `crates/**/*.rs` (see below) must verify a source
   change actually produces a different deployed binary — not just a
   cache miss on the build step.

This SHA256 step should be added to the workflow **regardless of caching
outcome** — it protects against more than just cache bugs.

### cargo-lambda binary cache

`pip3 install cargo-lambda` runs every deploy (~30-60s). Replace with
prebuilt binary:

```bash
CARGO_LAMBDA_VERSION=1.x.x
curl -L https://github.com/cargo-lambda/cargo-lambda/releases/download/v${CARGO_LAMBDA_VERSION}/cargo-lambda-v${CARGO_LAMBDA_VERSION}.x86_64-unknown-linux-musl.tar.gz \
  | tar -xz -C ~/.local/bin
```

Prebuilt releases verified available for `linux-x86_64`. Alternative:
`actions/cache` on `~/.local/bin/cargo-lambda` keyed by version string.

## Node / Nx caching

Current workflow uses `actions/setup-node@v4` with `cache: npm`. **This
only caches `~/.npm` (download cache), not `node_modules/`** — `npm ci`
still runs extract + link + postinstall on every run.

### Layer 1: Nx task cache (`.nx/cache`) — probably the biggest win

Not currently in workflow. Nx caches task outputs (`build`, `lint`, `test`)
keyed on declared `inputs`. On cache hit Nx restores `dist/` in ~200ms and
marks the task "cached" — no actual build runs.

```yaml
- uses: actions/cache@v4
  with:
    path: .nx/cache
    key: nx-${{ runner.os }}-${{ hashFiles('package-lock.json', 'nx.json', 'tsconfig.base.json') }}-${{ github.sha }}
    restore-keys: |
      nx-${{ runner.os }}-${{ hashFiles('package-lock.json', 'nx.json', 'tsconfig.base.json') }}-
      nx-${{ runner.os }}-
```

`github.sha` in `key` ensures we always save a fresh cache; `restore-keys`
finds the closest previous match on restore.

**⚠️ Prerequisite — verify Nx config first.** Before adding GH Actions
cache for `.nx/cache`, confirm `nx.json` / `project.json` have correctly
declared `inputs` and `outputs` for the CDK `build` target. If not, Nx
caches a broken state and GH cache just replicates it. This may require a
spawned PR to fix Nx config **before** the caching PR.

Pre-flight check for `nx.json`:

- `targetDefaults.build.inputs` covers all files that affect build (source, tsconfig, tool configs)
- `targetDefaults.build.outputs` points to `{projectRoot}/dist`
- `namedInputs` properly split (`default`, `production`)

### Layer 2: `node_modules/` cache — medium win, medium risk

Cache `node_modules/` directly keyed on `package-lock.json` hash. On hit,
`npm ci` becomes effectively a no-op (or can be skipped).

```yaml
key: node-modules-${{ runner.os }}-${{ runner.arch }}-node${{ steps.node.outputs.node-version }}-${{ hashFiles('package-lock.json') }}
```

**⚠️ Native modules risk.** Packages like `esbuild`, `swc`, `better-sqlite3`,
`lightningcss` ship platform-specific binaries in `node_modules/`. Cache
key **must** include `runner.os`, `runner.arch`, and Node version.

**Conservative alternative:** leave `cache: npm` as-is, rely on Nx cache
(layer 1) for the bigger win. Add `node_modules` caching only if Phase 0
shows `npm ci` is still a meaningful bottleneck **after** Nx cache lands.

### What NOT to cache separately

- **`dist/` artifacts** — Nx already handles via `.nx/cache`. Two sources of truth → stale restores.
- **`node_modules/.cache/` (babel, swc, tsc incremental)** — covered by Nx cache if config is correct.
- **`tsconfig.tsbuildinfo`** — covered by Nx cache.
- **`cdk.out`** — excluded by ground rules (account-specific, correctness risk).

### Nx Cloud / remote cache — out of scope

Cross-dev + CI cache sharing is meaningful in large monorepos. For this
repo (2 devs, low deploy frequency) it's overkill. Note as potential
follow-up if team grows.

## Cache validation test matrix

Before merging PR 2, verify cache correctness against this matrix. Each
row = one test deploy via `workflow_dispatch` from PR branch. Expected
hit/miss pattern is what you should observe.

| Change                               | node_modules | rust cache | nx cache | cargo-lambda |
| ------------------------------------ | :----------: | :--------: | :------: | :----------: |
| `infra/aws-cdk/lib/*.ts`             |     hit      |    hit     |   miss   |     hit      |
| `crates/**/*.rs` (lambda source)     |     hit      |    miss    |   hit    |     hit      |
| `Cargo.toml` / `Cargo.lock`          |     hit      |    miss    |   hit    |     hit      |
| `package.json` / `package-lock.json` |     miss     |    hit     |   miss   |     hit      |
| `nx.json`                            |     hit      |    hit     |   miss   |     hit      |
| no source change (no-op redeploy)    |     hit      |    hit     |   hit    |     hit      |

**Success criteria per row:**

- Cache layers match the expected hit/miss pattern.
- For rust row and no-op row: **SHA256 of deployed Lambda zip is what you expect** (either the new build or the unchanged previous build). This is the real correctness test.
- Final deployment produces a working staging (smoke test passes).

If any row produces wrong result (false hit / unnecessary miss / stale
Lambda) → fix cache key before merge. Do not ship broken caching.

## Phase 2 — Deploy-time optimization (NOT caching)

Separate from the caching layers above:

- **Do NOT cache `cdk.out`.** (Ground rule.)
- Consider `cdk diff --all` as a pre-step; if diff is empty for all stacks, skip `cdk deploy` entirely. CloudFormation already no-ops unchanged stacks, but `cdk deploy` still waits on each — skipping saves the roundtrip.
- Do **not** introduce `nx affected` — single CDK project, no benefit, adds complexity with tag-based triggers (PR 3).
