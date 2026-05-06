---
id: '0195'
title: 'Lambda 2 enrichment: off-chain NULL fields (icon-name extension, lp_tvl, asset_usd_price, nft_metadata)'
type: FEATURE
status: backlog
related_adr: ['0026', '0029', '0032']
related_tasks: ['0125', '0188', '0191', '0194', '0196', '0197']
tags: [priority-medium, effort-large, layer-enrichment, layer-lambda, audit-gap]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-05-06'
    status: backlog
    who: karolkow
    note: 'Spawned from M2 enrichment planning session 2026-05-06. Second of four tasks (0194-0197) implementing the field allocation rule. Subsumes 0125 (LP TVL part) and 0191 future-work bullets #2 (asset_usd_price implied) + LP analytics kind.'
---

# Lambda 2 enrichment: off-chain NULL fields (icon-name extension, lp_tvl, asset_usd_price, nft_metadata)

## Summary

Four sub-blocks layered on 0191's SQS-driven type-1 enrichment worker, each populating a column that **cannot** be derived from the processed ledger and therefore needs an external source: extension of the existing `icon` kind to also persist classic credit `assets.name` from the same SEP-1 fetch (added 2026-05-06 after 0197 dry-run audit), USD price oracle for LP TVL, USD price feed for asset list sort-by-value, and Soroban RPC `token_uri()` for NFT metadata. All four reuse the existing `enrichment-shared` crate, `enrichment-worker` dispatch, indexer SQS producer, and the permanent-vs-transient EnrichError taxonomy from 0191.

## Status: Backlog

Cannot start until task **0194 sub-block 1a** (schema migration adding `assets.usd_price` + `assets.usd_price_updated_at`) merges to develop. Sub-blocks 2a (icon-name extension), 2b (lp_tvl) and 2d (nft_metadata) write existing columns and are unblocked once 0191 is in production. Sub-block 2c (asset_usd_price) is hard-blocked on 0194 1a.

## Context

### Field allocation rule (from 0194 sub-block 1f / ADR 0026)

Off-chain = data NOT already in the processed ledger. Decision points crystallised this session:

- **`assets.name` (classic credit only)**: full names like "USD Coin" come from issuer SEP-1 TOML `CURRENCIES[].name`. Off-chain. Soroban/SAC `name` continue indexer-side (task 0156). **Added 2026-05-06 after 0197 dry-run audit** caught misallocation in original 0194 sub-block 1b.
- **`liquidity_pools.tvl` + `liquidity_pool_snapshots.tvl`**: USD-denominated, requires price oracle. Off-chain.
- **`assets.usd_price`**: USD price feed (CoinGecko / Reflector / StellarExpert). Off-chain.
- **`nfts.{collection_name, name, media_url, metadata}`**: requires Soroban RPC `token_uri()` per NFT, often dereferences to HTTP/IPFS gateway for JSON. Per the rule "on-chain = already in processed ledger", per-token RPC counts as off-chain (audit `docs/audits/2026-04-10-pipeline-data-audit.md` line 644-647 explicitly: "requires `token_uri()` RPC calls to the contract — not available from XDR events. This is an enrichment job"). Karol-confirmed Option A in 2026-05-06 session.

LP `volume` and `fee_revenue` are **NOT** in this task's scope — they're on-chain (PathPayment delta + arithmetic), handled by 0194 sub-block 1d.

### Reuse from 0191

The 0191 PR (branch `feat/0191_type1-enrichment-worker-lambda`, commit `25c93c9`) delivers:

- `crates/enrichment-shared` library: `sep1/` (HTTP fetcher with LRU cache) + `enrich_and_persist/` (`icon.rs` + `error.rs` with permanent/transient EnrichError split)
- `crates/enrichment-worker` Lambda binary: `SqsEvent` dispatch via `EnrichmentMessage` tagged enum (`#[serde(tag = "kind")]`)
- `crates/indexer/src/handler/enrichment_publish.rs`: `Publisher` struct, `publish_for_extracted_assets`
- CDK: `enrichmentQueue` (visibility 60s, retention 14d, maxReceiveCount 3) + dlq + DepthAlarm with Slack wiring + worker ErrorRateAlarm + dashboard widget
- Sentinel pattern (icon.rs:140 `write_sentinel`): writes empty-string `''` for permanent fails so producer SQL `WHERE icon_url IS NULL` short-circuits re-publish

This task adds three `EnrichmentMessage` variants and three new modules under `enrichment-shared/src/enrich_and_persist/`. Worker dispatch gets three new `match` arms. Producer gets three new SELECT-and-batch hooks.

### Permanent / transient taxonomy (0191 design decision #11)

Each new kind must map its failure modes to:

- `EnrichError::Database(#[from] sqlx::Error)` — DB write failed, transient, SQS retry
- `EnrichError::Transient(String)` — recoverable upstream (5xx, network, timeout), SQS retry
- Permanent fails → write sentinel + ack (no DLQ spam)

Sentinel value depends on column type:

- VARCHAR/TEXT (existing icon, NFT fields): `''` empty string
- NUMERIC: TBD per kind (e.g. `0` or a separate `_status` column — design decision per kind)
- TIMESTAMPTZ: `NULL` is fine (column meaning "last attempt" can be tracked via `updated_at` companion column)

## Implementation Plan

### Sub-block 2a — Icon kind extension: also persist `assets.name` (classic credit)

**Added 2026-05-06 after 0197 dry-run audit** revealed classic credit `assets.name` is off-chain (SEP-1 TOML `CURRENCIES[].name`) and was incorrectly placed in 0194 sub-block 1b. Per ADR 0026, off-chain → Lambda 2. Cheapest implementation: extend the existing 0191 `icon` kind (which already fetches the same TOML for `image`) to additionally extract and persist `name` in the same SQL UPDATE.

**Spec:**

- Extend `crates/enrichment-shared/src/sep1/dto.rs:32-41` `Sep1Currency` struct with `pub name: Option<String>`. Confirms with SEP-1 spec field `[[CURRENCIES]] name` (e.g. "USD Coin").
- Extend `crates/enrichment-shared/src/enrich_and_persist/icon.rs` `enrich_asset_icon` to also UPDATE `assets.name` when SEP-1 yields one. Combined SQL: `UPDATE assets SET icon_url = $1, name = COALESCE($2, name) WHERE id = $3` — `COALESCE` so we don't overwrite existing Soroban/SAC names extracted by the indexer.
- Sentinel `''` for `icon_url` continues unchanged. For `name`: leave NULL when not present in TOML (no sentinel needed — `name IS NULL` is fine, list endpoint UI falls back to `asset_code`).
- Producer SQL on `enrichment_publish.rs` extends asset selection: `WHERE icon_url IS NULL OR (asset_type = 1 AND name IS NULL)` (asset_type=1 = classic credit; we don't re-emit Soroban/SAC for `name` because indexer/0156 owns those).
- No new `EnrichmentMessage` variant — reuse `EnrichmentMessage::Icon { asset_id }` since it's the same SEP-1 fetch.

**Why piggyback on icon kind, not separate `asset_name` kind:**

- Single TOML fetch yields both `image` and `name` from the same `CURRENCIES[]` entry. Two kinds = two HTTP fetches per asset. Wasteful.
- Sep1Fetcher LRU cache makes the second fetch cheap, but the message ↔ row dispatch overhead doubles.
- The existing 0191 worker error taxonomy (permanent → sentinel + ack, transient → DLQ retry) covers `name` failures identically — name failures are exactly the same TOML-fetch failures as icon failures.

### Sub-block 2b — `lp_tvl` `EnrichmentMessage` variant

**Supersedes 0125** (LP price oracle / TVL part). The volume/fee_revenue part of 0125's scope already moved to 0194 sub-block 1d. 0125 archived as `superseded by: ["0194", "0195"]`.

**Spec:**

- New variant: `EnrichmentMessage::LpTvl { pool_id: [u8; 32], snapshot_id: i64 }` (binary pool_id matches schema's BYTEA(32))
- New module `crates/enrichment-shared/src/enrich_and_persist/lp_tvl.rs` exposing `enrich_pool_tvl(pool, pool_id, snapshot_id, oracle: &impl PriceOracle)`
- Compute `tvl = reserve_a × price_a_usd + reserve_b × price_b_usd`. Both legs queried from oracle. UPDATE both `liquidity_pools.tvl` (latest) and `liquidity_pool_snapshots.tvl` (specific snapshot row).
- **Oracle source decision**:
  1. **Reflector** primary — Soroban on-chain price feed contract (used by Soroswap), no rate limit, native to Stellar
  2. **StellarExpert API** fallback — `/asset/<code>-<issuer>` returns `price7d`, free, caches CoinGecko underneath
  3. **Horizon `/trade_aggregations`** sanity check + USDC/USDT pegged direct (price=1, no oracle call)
  4. CoinGecko skipped — duplicates StellarExpert, adds rate limit
- **Producer hook**: `crates/indexer/src/handler/enrichment_publish.rs` — after each new `liquidity_pool_snapshots` row, emit `LpTvl { pool_id, snapshot_id }`
- **Permanent fails**: pool legs without any oracle data → sentinel decision TBD (proposal: write `tvl=0` + log warn). Transient (5xx, network) → `EnrichError::Transient`, SQS retry.

### Sub-block 2c — `asset_usd_price` `EnrichmentMessage` variant

**New territory** — no precursor task. Captured as 0191 future-work bullet #6 ("stellarchain.io/markets parity") without a dedicated task.

**Spec:**

- New variant: `EnrichmentMessage::AssetUsdPrice { asset_id: i32 }`
- New module `crates/enrichment-shared/src/enrich_and_persist/asset_usd_price.rs`
- Source: **CoinGecko Stellar list** primary (~30 req/min free tier, batch endpoint `/simple/price?ids=...`), StellarExpert fallback. Reflector unsuitable here because most classic credit assets (where USD price matters most) aren't in Reflector feed.
- Writes `assets.usd_price = ?, assets.usd_price_updated_at = NOW()`
- **Producer SQL with TTL refresh**: `WHERE usd_price IS NULL OR usd_price_updated_at < NOW() - INTERVAL '24h'`. This embeds periodic refresh into the producer — replaces a dedicated janitor cron Lambda for this kind. Cap producer batch size so each ledger doesn't re-emit thousands of stale rows.
- Alternative trigger model (decide in spec phase): daily-cron Lambda that scans + emits, vs. embedding TTL in indexer producer SQL. Tradeoff: cron is simpler but requires separate infra; embedded TTL re-uses producer but adds complexity to indexer SQL.
- **Permanent fails** (asset not in CoinGecko list): sentinel `usd_price = 0` with `updated_at = NOW()` so TTL refresh doesn't immediately re-attempt. Reconsider after first ops report.

### Sub-block 2d — `nft_metadata` `EnrichmentMessage` variant

**New territory** — supersedes audit-mention only (audit doc line 644-647), no dedicated task.

**Spec:**

- New variant: `EnrichmentMessage::NftMetadata { nft_id: i32 }` (where `nft_id` references `nfts.id`)
- New module `crates/enrichment-shared/src/enrich_and_persist/nft_metadata.rs`
- Pipeline:
  1. SELECT `contract_id, token_id` from `nfts` table
  2. Soroban RPC call: `token_uri(token_id)` on the NFT contract — returns URI string
  3. URI may be `https://...`, `ipfs://...`, or on-chain ContractData reference
  4. Resolve URI → fetch JSON metadata (HTTP for `https://`, IPFS gateway for `ipfs://`)
  5. Parse JSON → extract `name`, `description` → maps to `collection_name`, `name`, `media_url`, `metadata`
  6. UPDATE `nfts` row with parsed fields
- **IPFS gateway choice**: Pinata public gateway or Cloudflare `cloudflare-ipfs.com` — free, cache-friendly, no auth. Decide in spec phase (DDoS resilience may favour multi-gateway round-robin).
- **Producer hook**: indexer emits `NftMetadata { nft_id }` after each new `nfts` row insert (NFT mint event). DOES NOT re-emit on transfer/burn (those don't change metadata).
- **Sentinel handling**: permanent fail (404 on token_uri, malformed JSON, IPFS gateway gives 404) → write `name=''`, `media_url=''`, `metadata='{}'` so producer dedup `WHERE name IS NULL` short-circuits. Transient (gateway 5xx, RPC timeout) → `EnrichError::Transient`, SQS retry.
- **Cost concern**: NFT collections can have 10k+ tokens. Per-NFT call is justified for off-chain pattern (this is exactly why it's Lambda 2 not indexer — see ADR 0026). If volume becomes prohibitive, consider per-collection batching (one RPC call returns all metadata) — captured as Future Work.

### Common: ADR amendment + docs

- ADR 0029 (`abandon-parsed-artifacts-read-time-xdr-fetch`) amendment: extend "runtime_enrichment umbrella" section with type-1 SQS model + new kinds. Captured already as 0191 Future Work bullet #3.
- Docs `docs/architecture/indexing-pipeline/enrichment.md` (or create if absent) — kind-by-kind matrix
- Docs `docs/architecture/database-schema/**` — column source attribution updated

## Acceptance Criteria

- [ ] Sub-block 2a: `Sep1Currency.name` field added; icon kind extended to UPDATE `assets.name` via `COALESCE` (no overwrite of indexer-set Soroban/SAC names); sample query shows non-NULL `name` on classic credit assets with SEP-1 TOML support; producer SQL re-emits classic credit assets with NULL name
- [ ] Sub-block 2b: `lp_tvl` kind dispatched, sample query shows non-NULL `tvl` on production-region pools with valid oracle data
- [ ] Sub-block 2c: `asset_usd_price` kind dispatched, sample query shows non-NULL `usd_price` on top-50 assets by activity
- [ ] Sub-block 2d: `nft_metadata` kind dispatched, sample query shows non-NULL `name`/`media_url` on minted NFTs from at least one well-known collection
- [ ] Each kind has its permanent / transient EnrichError mapping documented + tested
- [ ] Each kind has integration test (mock oracle / mock RPC / mock IPFS gateway)
- [ ] DepthAlarm thresholds per CDK reviewed for new producer rates
- [ ] **Docs updated**: ADR 0029 amendment, `docs/architecture/indexing-pipeline/enrichment.md`, `docs/architecture/database-schema/**`
- [ ] **API types regenerated** — if any DTO field is exposed (e.g. `nfts.metadata` JSON shape), codegen committed in same PR
- [ ] 0125 archived as `superseded by: ["0194", "0195"]`

## Future Work (out of scope, spawn separate tasks)

- **Per-collection NFT batching** if per-token cost becomes prohibitive in production
- **Periodic janitor for `lp_tvl`**: cron Lambda re-emitting stale snapshots — only if observe stale TVLs in production
- **Worker observability per kind**: CloudWatch custom metrics (success/fail counters, fetch latency histogram per kind)
- **`asset_usd_price` extended sources**: if CoinGecko coverage gaps observed, evaluate Coinbase API or Kraken public ticker

## Notes

- **Sentinel semantics confirmation (2026-05-06 session)**: current behavior on `assets.icon_url` (write `''` on permanent fail, transient retries to DLQ) stays as-is. Each new kind designs its own sentinel value but follows the same "permanent → sentinel + ack, transient → retry/DLQ" split per 0191 design decision #11.
- **Bundling rationale**: 4 sub-blocks share the SQS scaffolding, EnrichError taxonomy, dispatch pattern, and ops/CDK surface. Splitting per kind would force 4× repeated context. Each sub-block is one new module (or module extension for 2a) + one match arm + one producer hook + one CDK threshold review — ~50-300 LoC each.
- **2a is intentionally NOT a new EnrichmentMessage variant**: extending the existing `Icon` variant is correct because (a) one TOML fetch yields both `image` and `name`, (b) failure modes are identical (same SEP-1 endpoint), (c) sentinel behaviour for icon_url stays untouched while `name` simply stays NULL on permanent fails (no display impact — UI falls back to `asset_code`). Adding a separate `AssetName` variant would double the queue traffic for zero benefit.
