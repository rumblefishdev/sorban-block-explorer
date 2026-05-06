---
id: '0196'
title: 'Enrichment backfill: new crate that drains pre-existing un-enriched DB rows for every kind'
type: FEATURE
status: backlog
related_adr: ['0026', '0029', '0032']
related_tasks: ['0188', '0191', '0194', '0195', '0197']
tags: [priority-medium, effort-medium, layer-cli, layer-enrichment]
milestone: 2
links: []
history:
  - date: '2026-05-06'
    status: backlog
    who: karolkow
    note: 'Spawned from M2 enrichment planning session 2026-05-06. Third of four tasks (0194-0197). Explicit override of 0191 Future-Work bullet #1 (which specified extending backfill-runner) — Karol directed 2026-05-06 that this must be a NEW crate, never a backfill-runner subcommand.'
---

# Enrichment backfill: new crate that drains pre-existing un-enriched DB rows for every kind

## Summary

A standalone CLI crate (`crates/enrichment-backfill`) that drains pre-existing rows the live SQS-driven worker never saw — tens of thousands of pubnet assets that pre-date 0191's queue, every existing LP snapshot that pre-dates `lp_tvl`, every NFT that pre-dates `nft_metadata`, every asset before `asset_usd_price` columns existed. Reuses the same `enrichment-shared::enrich_and_persist::*` functions that the live worker uses, so drain logic and live logic share a single implementation; no SQS involved — direct DB writes via streaming SELECTs and bounded concurrency.

## Status: Backlog

Cannot start until task **0195** has merged the three new `enrich_*` functions to `enrichment-shared` (`lp_tvl`, `asset_usd_price`, `nft_metadata`). The `icon` subcommand can theoretically land sooner since `enrich_asset_icon` is already in 0191's PR — split-PR option captured below.

## Context

### Why a new crate, NOT a backfill-runner subcommand

0191's "Future Work" section (line 362-367 of `lore/1-tasks/archive/0191_FEATURE_type1-enrichment-worker-lambda.md`) reads:

> Local backfill CLI subcommand — `backfill-runner enrichment run / status`. Reuses `enrichment_shared::enrich_and_persist::icon::enrich_asset_icon` to drain rows that pre-date the live producer.

Karol explicitly overrode this in the 2026-05-06 planning session: enrichment backfill must be a **separate crate**, not a `backfill-runner` subcommand. Stored as memory `feedback_backfill_new_crate.md`.

Reasoning:

- `backfill-runner` re-ingests Stellar ledgers from XDR archives via Galexie. Enrichment backfill drains pre-existing DB rows by calling external enrichment APIs. **Different concerns, different data sources, different operational profiles.**
- 0191 design decision #8 was emphatic that backfill must not be modified at all (process_ledger refactored mid-implementation specifically to keep `backfill-runner` diff at zero). New crate keeps that guarantee.
- Separate crate = independent versioning, independent CI, no risk of changes spilling into the ledger backfill code path.

### What "drain" means

The live SQS-driven worker (0191 + 0195 kinds) is **forward-only**: it processes assets/snapshots/NFTs as the indexer emits them. It does not see anything that pre-dates the queue's deployment. For every kind, there's a population gap covering the historical region:

- **`assets.icon_url`** (kind: `icon`): 0191's producer SQL `WHERE icon_url IS NULL` correctly skips already-processed rows, but assets that existed in DB before the queue went live were never published. Backfill streams `SELECT id FROM assets WHERE icon_url IS NULL` and calls `enrich_asset_icon` directly.
- **`liquidity_pool_snapshots.tvl`** (kind: `lp_tvl`): every snapshot row created before 0195's hook lands has `tvl IS NULL`.
- **`assets.usd_price`** (kind: `asset_usd_price`): until 0194 1a lands the column, none exists. After 0194 1a, every existing asset starts with `NULL`.
- **`nfts.{collection_name, name, media_url, metadata}`** (kind: `nft_metadata`): every NFT minted before 0195 2c lands has all four NULL.

### Force-retry semantics (clarified 2026-05-06)

Three semantics were considered:

- **α**: clear `''` sentinel only, then run with standard filter `WHERE icon_url IS NULL` — re-runs only previously-failed rows
- **β**: clear ALL output column to NULL, then run with standard filter — re-fetches everything via two-step
- **γ**: bypass standard filter entirely, SELECT all rows, call enrich on each — idempotent in-place overwrite

**Decision: γ (idempotent overwrite, no clear step).** Reasoning:

- `enrich_*` functions are idempotent — they always write whatever they find (real value, sentinel, or transient retry). No need to NULL-out first.
- Issuer changed TOML → new URL overwrites old. Issuer removed image → real URL gets replaced with sentinel (legitimate re-classification). Issuer fixed 404 → sentinel replaced with real URL. All handled by the existing enrich function unchanged.
- β has a millisecond window where API would render NULL icons across the entire asset table. γ has no such window.
- Single-step UX: one command does the whole thing.

CLI shape:

```
enrichment-backfill icon [--limit N] [--asset-id N]            # standard, WHERE icon_url IS NULL OR (asset_type=1 AND name IS NULL)
enrichment-backfill icon --force-retry [--limit N]             # γ: no filter, idempotent overwrite
enrichment-backfill lp-tvl ...
enrichment-backfill asset-usd-price ...
enrichment-backfill nft-metadata ...
enrichment-backfill status                                      # COUNT(*) per-column un-populated, per-kind
```

**Note on `icon` subcommand scope** (post-2026-05-06 dry-run audit, see 0195 sub-block 2a): `icon` is a misnomer once 0195 lands — the underlying `enrich_asset_icon` will write both `assets.icon_url` AND `assets.name` (for classic credit) from a single SEP-1 fetch. The subcommand name stays `icon` for continuity, but the standard filter must include classic-credit-name backfill: `WHERE icon_url IS NULL OR (asset_type=1 AND name IS NULL)`. Status subcommand reports both counts separately.

## Implementation Plan

### Step 1: Crate scaffolding

- New crate `crates/enrichment-backfill` (kebab-case mirroring `backfill-runner`, `audit-harness`)
- `Cargo.toml` deps: `enrichment-shared` (workspace path), `sqlx`, `tokio`, `clap`, `tracing`, `tracing-subscriber`
- Binary `src/main.rs` with `clap` subcommand parser
- `src/lib.rs` with shared streaming + concurrency helpers
- Add to workspace `Cargo.toml` members
- `nx project.json` if `nx` orchestration applies

### Step 2: Streaming + concurrency primitive

Per-subcommand structure:

```rust
async fn drain_kind(pool: &PgPool, fetcher: &Sep1Fetcher, force_retry: bool, limit: Option<u64>) {
    let mut last_id = 0i32;
    loop {
        let chunk: Vec<i32> = sqlx::query_scalar(SELECT_SQL_FOR_KIND)
            .bind(last_id)
            .bind(CHUNK_SIZE)
            .fetch_all(pool).await?;
        if chunk.is_empty() { break; }
        last_id = *chunk.last().unwrap();

        stream::iter(chunk)
            .map(|id| async move { enrich_asset_icon(pool, id, fetcher).await })
            .buffer_unordered(10)
            .for_each(|r| async { /* log, count */ })
            .await;
    }
}
```

The `WHERE` clause depends on `force_retry`:

- standard: `WHERE icon_url IS NULL AND id > $1 ORDER BY id LIMIT $2`
- force_retry: `WHERE id > $1 ORDER BY id LIMIT $2` (no filter)

Concurrency target: `buffer_unordered(10)` matches 0191 design — 10 concurrent SEP-1 fetches saturate typical issuer hosts without triggering rate limits.

### Step 3: Per-kind subcommand wiring

Each subcommand wires kind-specific:

- SELECT SQL (which table, which column, which filter)
- Call to the matching `enrich_and_persist::*` function
- Sentinel-aware `--force-retry` filter override

```rust
match cli.command {
    Cmd::Icon(args) => drain_kind(pool, fetcher, args.force_retry, args.limit, ICON_KIND).await,
    Cmd::LpTvl(args) => drain_kind(pool, oracle, args.force_retry, args.limit, LP_TVL_KIND).await,
    Cmd::AssetUsdPrice(args) => ...,
    Cmd::NftMetadata(args) => ...,
    Cmd::Status => print_status(pool).await,
}
```

### Step 4: Status subcommand

`enrichment-backfill status` runs five COUNT queries:

```sql
SELECT COUNT(*) FILTER (WHERE icon_url IS NULL) AS icon_pending,
       COUNT(*) FILTER (WHERE icon_url = '')   AS icon_sentinel,
       ...
```

Output one row per kind. Operator runbook visibility.

### Step 5: Single-asset / single-row mode

`--asset-id N` / `--snapshot-id N` / `--nft-id N` for surgical reruns. Bypasses streaming, calls enrich once. Exit code 0 on success, non-zero on error (so ops can chain in shell scripts).

### Step 6: Tests

Per subcommand integration test: spin up postgres test container, populate fixture rows, mock SEP-1 / oracle / RPC, run subcommand, assert post-run row state. Reuse 0191 SEP-1 mock infrastructure.

Benchmark: 50K assets `icon` backfill on local laptop (M-series Mac) target < 30 min wall clock with `buffer_unordered(10)` and Sep1Fetcher LRU cache (issuer hits cache after first asset).

### Step 7: Optional split-PR — icon-only first

If 0195 timeline slips, ship `crates/enrichment-backfill` with `icon` subcommand only against 0191's `enrich_asset_icon`. Add other kinds in follow-up PRs as 0195 sub-blocks land. Captured as PR-staging option, decide at start.

## Acceptance Criteria

- [ ] `crates/enrichment-backfill` crate builds, lints, integrated into workspace
- [ ] All four kind subcommands (`icon`, `lp-tvl`, `asset-usd-price`, `nft-metadata`) wired
- [ ] `--force-retry` flag implemented per γ semantics (no filter, in-place overwrite)
- [ ] `--asset-id`/`--snapshot-id`/`--nft-id` surgical mode per kind
- [ ] `status` subcommand prints per-kind un-populated counts
- [ ] Integration test per subcommand
- [ ] Benchmark: 50K asset icon backfill < 30 min on local laptop documented in README
- [ ] README runbook with example invocations + post-deployment ops checklist
- [ ] **Docs updated**: `docs/architecture/indexing-pipeline/**` — section on backfill mechanics; ADR 0026 mentions backfill as the rule's drain path
- [ ] **API types regenerated** — N/A, this crate ships no API surface
- [ ] 0191 Future Work bullet #1 marked obsolete in 0191 archive notes (override note already in 0196 history)

## Future Work (out of scope, spawn separate tasks)

- **Production scheduling**: backfill is jobs-on-demand for ops, NOT a cron Lambda. Periodic refresh for `asset_usd_price` is embedded in producer SQL TTL (0195 2b). If other kinds ever need periodic refresh, spawn a dedicated cron Lambda task — separate from this backfill crate.
- **Multi-region distributed backfill**: not needed for current pubnet asset volume. Re-evaluate if drain target exceeds 1M rows.
- **Status web dashboard**: `enrichment-backfill status` is CLI-only. If ops wants live progress in a dashboard, plumb metrics to existing CloudWatch dashboard — separate task.

## Notes

- **Bundling rationale**: 4 subcommands share crate scaffolding, streaming+concurrency primitive, status query pattern, and integration test infrastructure. Splitting per subcommand would micro-decompose ~80 LoC each into separate PRs.
- **Why no SQS path**: backfill bypasses SQS entirely because (a) 50K-row queue publish would hit SQS rate limits and (b) per-message overhead (visibility timeout, delete after ack) wastes time when we already have DB connection. Direct call into `enrichment-shared` proves a clean library boundary too.
- **Sep1Fetcher LRU cache** (from 0191): one shared `Sep1Fetcher` per process. Cache survives across all `icon` subcommand calls — issuer with 100 assets pays SEP-1 fetch cost once.
