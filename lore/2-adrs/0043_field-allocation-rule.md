---
id: '0043'
title: 'How new fields reach the API: indexer column / enrichment Lambda column / runtime fetch (no column)'
status: accepted
deciders: [karolkow]
related_tasks: ['0188', '0191', '0194', '0195', '0196', '0197']
related_adrs: ['0007', '0022', '0023', '0029', '0032']
tags: [governance, enrichment, schema, indexer, lambda, milestone-2]
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: 2026-05-06
    status: accepted
    who: karolkow
    note: 'ADR created and accepted to codify the rule locked during the M2 enrichment planning session. Lands independently on develop before tasks 0194-0197 reference it.'
---

# ADR 0043: How new fields reach the API — indexer column / enrichment Lambda column / runtime fetch (no column)

**Related:**

- [Task 0188: SEP-1 fetcher + assets details enrichment](../1-tasks/archive/0188_FEATURE_sep1-fetcher-and-assets-details-enrichment.md)
- [Task 0191: Type-1 SQS-driven enrichment worker](../1-tasks/archive/0191_FEATURE_type1-enrichment-worker-lambda.md)
- [Task 0194: Schema additions + indexer on-chain fields](../1-tasks/active/0194_FEATURE_schema-additions-and-indexer-on-chain-fields.md)
- [Task 0195: Lambda 2 enrichment off-chain fields](../1-tasks/backlog/0195_FEATURE_lambda2-enrichment-off-chain-fields.md)
- [Task 0196: Enrichment backfill crate](../1-tasks/backlog/0196_FEATURE_enrichment-backfill-crate.md)
- [Task 0197: DB completeness audit + docs](../1-tasks/backlog/0197_FEATURE_db-completeness-audit-and-docs.md)

---

## Context

The explorer's data surface has three distinct write paths today:

1. **Indexer (Galexie Lambda)** — parses XDR ledger files, writes typed columns into Postgres. Source data is already in the processed ledger.
2. **Enrichment worker (SQS-driven Lambda, task 0191)** — fetches off-chain data (SEP-1 TOML, oracle prices, NFT metadata via per-token RPC) and persists it to typed columns. Triggered per indexer-emitted message.
3. **Runtime type-2 enrichment in API handler** — per-request, in-process, fail-soft, LRU-cached fetch executed inside `crates/api` (e.g. `runtime_enrichment::sep1` for `assets.description` / `home_page` per task 0188; `runtime_enrichment::stellar_archive` for E3 / E14 heavy fields per ADR 0029). Never persisted.

Without an explicit rule, every new field invites an ad-hoc allocation decision. Recent M2 planning surfaced multiple cases where a field was placed on the wrong path: classic credit `assets.name` was originally drafted into 0194 (indexer) but is off-chain (SEP-1 TOML); LP `volume` / `fee_revenue` was drafted into 0125 (Lambda 2 cron) but is on-chain (PathPayment delta + arithmetic). Migration `20260424000000_drop_assets_sep1_detail_cols.up.sql` removed `assets.description` and `assets.home_page` columns after the team converged on runtime type-2 for detail-only fields. These churns are expensive and avoidable with a stated rule.

---

## Scope

This ADR governs **fields that reach the API response shape** — list-endpoint columns, detail-endpoint columns, and detail-only fields served via runtime type-2. It does **not** govern internal / auxiliary columns that exist solely to make the indexer / API stack work and are never returned to clients:

- FK helpers and surrogate ids (e.g. `accounts.id`, `soroban_contracts.id`)
- Watermark columns (`first_seen_ledger`, `last_updated_ledger`)
- Search vectors (PostgreSQL GENERATED FTS columns)
- Appearance index tables (join helpers)
- Enrichment-internal discriminators (`enrichment_status`, sentinels)

Auxiliary columns trivially satisfy "indexer-written, on-chain-derived" by construction — they have no off-chain or runtime-fetch alternative. Their allocation needs no ADR-level rule; whichever component requires them owns their schema and writes.

---

## Decision

**Scope:** API-visible fields. See *Scope* above for the auxiliary-column carve-out.

**Rule:**

- **List endpoint + on-chain** (data already in the processed ledger) → **indexer**. Populate at ingest time, never via enrichment.
- **Off-chain** (HTTP fetch, oracle call, per-row RPC) → **enrichment Lambda 2** (the type-1 SQS-driven worker from task 0191). Persist to typed columns. List endpoints serve those columns directly; the worker keeps them populated.
- **Detail-only** (returned only by `/:id` endpoints, never by list endpoints) → **runtime type-2 enrichment in the API handler**. **Never persist.** No dedicated DB column.

"On-chain" here means *already in the processed ledger as it arrives at the indexer* — i.e. derivable from XDR without an additional network round-trip. Per-token Soroban RPC calls (`token_uri()` for NFTs) are off-chain under this rule, even though the underlying data lives on Stellar, because they require an extra RPC per row outside the indexer's normal stream.

---

## Rationale

1. **Latency parity for list endpoints.** A list endpoint returning N rows cannot afford N HTTP fetches per request. Anything required by a list response must be in a typed column at request time. Indexer (on-chain) and Lambda 2 (off-chain) both produce typed columns; runtime type-2 cannot.
2. **Cost parity for off-chain reads.** Off-chain data is rare-change per row (SEP-1 TOML, USD price, NFT metadata). Paying the HTTP cost once at write time and serving thousands of reads from Postgres is dramatically cheaper than paying it per request.
3. **Detail endpoints tolerate per-request fetch.** A single `/v1/assets/{id}` response with a 24 h LRU cache absorbs the SEP-1 fetch cost in the cold path and amortises it across warm requests. The data does not warrant the storage + invalidation cost of a column.
4. **Anti-pattern prevention.** A column that is *only ever written by the detail handler and never read by list endpoints* is a pure liability — schema bloat, index bloat, indexer write amplification — without any read benefit. The rule rejects this shape outright.
5. **Audit doc Section 9.3 override.** The 2026-04-10 pipeline audit proposed a single scheduled cron Lambda for both LP TVL (off-chain, USD-denominated) and LP volume (on-chain, PathPayment delta). This ADR splits them: TVL → Lambda 2, volume → indexer. The cleaner allocation surfaces the cost asymmetry that the cron-bundled proposal hid.

---

## Alternatives Considered

### Alternative 1: Universal "indexer for everything that fits, Lambda 2 for everything that doesn't"

**Description:** Drop the third tier. Anything not derivable from XDR goes to Lambda 2 and gets a typed column.

**Pros:** Simpler — two paths instead of three.

**Cons:** Forces unnecessary persistence for detail-only fields (e.g. `assets.description`). Schema bloat, indexer write amplification on rarely-read fields, and an invalidation problem for fields that mutate off-chain (issuer edits SEP-1 TOML, NFT metadata gateway changes). Runtime type-2 sidesteps invalidation entirely by fetching on demand.

### Alternative 2: Universal runtime type-2 enrichment

**Description:** Drop typed columns for off-chain data. Resolve every off-chain field at request time, with caching.

**Pros:** No write-side enrichment infra (no Lambda 2, no SQS, no backfill crate). Single fetch path for all off-chain data.

**Cons:** Breaks list endpoints. A `/v1/assets` list of 50 rows would issue 50 SEP-1 fetches per request — even with caching, the cold-cache p95 is unacceptable. Filtering and sorting on off-chain fields (e.g. sort by USD price) becomes impossible without a column.

### Alternative 3: Cron-based off-chain enrichment

**Description:** Replace SQS-driven Lambda 2 with a scheduled cron that scans tables and fills NULL columns.

**Pros:** Simpler trigger model — no producer hook in indexer.

**Cons:** Latency: a cron interval (5–15 min) means freshly indexed assets show NULL list-endpoint fields until the next cron tick. SQS delivery is sub-second. Scan cost: cron either re-scans the whole table or maintains a "last seen" watermark, both more expensive than the SQS push model. SQS-driven also gives natural backpressure (per-message visibility timeout, DLQ).

---

## Consequences

### Positive

- New columns get a clear allocation answer in one read of this ADR.
- Existing column placements can be audited against the rule (see task 0197).
- Detail-only fields stop accumulating dead columns.
- The runtime / write-time / type-2 boundary is explicit, so transport-specific code (`runtime_enrichment::sep1`, `enrichment-shared::sep1`) doesn't bleed across boundaries.

### Negative

- Three write paths instead of two — slightly more infrastructure surface (SQS queue + DLQ + worker Lambda for type-1; in-process LRU cache for type-2; standard indexer for on-chain).
- Edge cases require judgement (e.g. "data is on-chain but expensive to derive" — see Soroban DEX volume Phase 2 in task 0194 §1d Future Work).

### Neutral

- Existing code already follows the rule informally; this ADR formalises practice rather than introducing it. Migration `20260424000000_drop_assets_sep1_detail_cols.up.sql` is the precedent for "drop dedicated detail columns and serve via runtime type-2".

---

## Per-kind allocation matrix (informative)

Snapshot of current allocations under this rule. Updated by tasks 0194 / 0195 / 0197.

| Field | Path | Owning task |
|---|---|---|
| `assets.name` (Soroban / SAC) | indexer (on-chain `ContractData`) | 0156 |
| `assets.name` (classic credit) | Lambda 2 (SEP-1 TOML `CURRENCIES[].name`) | 0195 §2a |
| `assets.icon_url` | Lambda 2 (SEP-1 TOML `CURRENCIES[].image`) | 0191 |
| `assets.holder_count` | indexer (trustline delta) | 0194 §1c |
| `assets.total_supply` (classic credit) | indexer (SUM of trustline balances) | 0194 §1b |
| `assets.usd_price` + `usd_price_updated_at` | Lambda 2 (CoinGecko / StellarExpert) | 0194 §1a (column) + 0195 §2c (population) |
| `assets.description`, `assets.home_page` | runtime type-2 (`runtime_enrichment::sep1`) | 0188 |
| `liquidity_pool_snapshots.tvl` | Lambda 2 (Reflector / StellarExpert oracle) | 0195 §2b |
| `liquidity_pool_snapshots.volume`, `fee_revenue` | indexer (PathPayment delta + arithmetic) | 0194 §1d |
| `nfts.{collection_name, name, media_url, metadata}` | Lambda 2 (Soroban RPC `token_uri()` + IPFS gateway) | 0195 §2d |
| `account_balances_current` (trustline rows) | indexer (TrustLine ledger entries) | 0119 (completed), verified by 0194 §1e |
| Transaction `envelope_xdr`, full `events` payload | runtime type-2 (`runtime_enrichment::stellar_archive`) | per ADR 0029 / 0033 / 0034 |

---

## Notes

- **Independence from tasks:** this ADR lands directly on develop as a standalone commit, before tasks 0194 / 0195 / 0196 / 0197 start implementation. Governance docs land independently of the code that references them so the rule is canonical at the time of code review.
- **ADR 0029 boundary:** ADR 0029 covers the *read-time XDR fetch* path (E3 / E14 heavy fields from S3). It is a sibling of this ADR's runtime type-2 case, sharing the in-process LRU + fail-soft pattern. ADR 0029 does not need amendment for type-1 write-side concerns; this ADR is the home for those.
