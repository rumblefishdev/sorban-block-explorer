---
id: '0194'
title: 'DB completeness: schema additions + indexer for on-chain NULL fields needed by list endpoints'
type: FEATURE
status: active
related_adr: ['0029', '0032', '0037']
related_tasks:
  ['0119', '0125', '0135', '0156', '0188', '0191', '0195', '0196', '0197']
tags:
  [
    priority-medium,
    effort-large,
    layer-indexer,
    layer-db,
    layer-xdr-parser,
    audit-gap,
  ]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-05-06'
    status: backlog
    who: karolkow
    note: 'Spawned from M2 enrichment planning session 2026-05-06. First of four tasks (0194-0197) implementing the field allocation rule: list endpoint + on-chain → indexer; off-chain → enrichment Lambda; detail-only → runtime type-2.'
  - date: '2026-05-06'
    status: active
    who: karolkow
    note: 'Activated to start implementation. Branch cut from develop; 0191 SQS enrichment branch will be merged into the feature branch since 0191 PR has not yet landed on develop and 0194 needs its enrichment-shared crate context.'
---

# DB completeness: schema additions + indexer for on-chain NULL fields needed by list endpoints

## Summary

Audit of list-endpoint DTOs vs DB schema vs actual writes shows a population gap: several columns exist in the schema but are always NULL because the indexer never writes them, and at least two list-endpoint sort fields (asset USD price + timestamp) need new schema columns. This task lands the schema additions atomically, wires indexer-side population for every NULL field whose source data is **already in the processed ledger** (no external HTTP, no per-row RPC), and codifies the field-allocation rule as a new ADR. Off-chain fields (oracle prices, SEP-1, NFT `token_uri()` RPC) are the sister task 0195's scope.

## Status: Backlog

Cannot start until 0191 PR (`feat/0191_type1-enrichment-worker-lambda`) merges to develop — that PR introduces the SQS enrichment infrastructure and `enrichment-shared` crate that 0195 builds on, and 0194 should land first so 0195's column writes have somewhere to go.

## Context

### Field allocation rule (locked this session)

Per Karol 2026-05-06: any field returned by a **list endpoint** (paginated array endpoints — `/assets`, `/liquidity-pools`, `/nfts`, `/transactions`, etc.) whose source data is **already in the processed ledger** must be populated by the indexer, **not** by enrichment Lambda 2. Off-chain data (HTTP fetches, per-row RPC, oracle calls) goes to Lambda 2. Detail-only fields (returned only by `/:id` endpoints) must NOT have dedicated DB columns — they are runtime type-2 enrichment in the API handler (per task 0188 SEP-1 fetcher pattern, e.g. `assets.description` and `assets.home_page` were dropped in migration `20260424000000_drop_assets_sep1_detail_cols.up.sql`).

### NULL-column inventory verified this session

Subagent audit confirmed by reading `crates/xdr-parser/src/state.rs`, `crates/indexer/src/handler/persist/{staging.rs,write.rs}`, `crates/api/src/{assets,liquidity_pools,nfts}/dto.rs`, and `crates/db/migrations/`:

| Table.column                            | DB type       | Currently                                                | On-chain?                                         | This task scope                                                                           |
| --------------------------------------- | ------------- | -------------------------------------------------------- | ------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `assets.holder_count`                   | INTEGER       | always NULL                                              | ✅ trustline delta (`change_trust create/delete`) | sub-block 1c                                                                              |
| `assets.name` (classic credit)          | VARCHAR(256)  | NULL for classic credit (Soroban handled by 0156 active) | ❌ — classic credit names come from SEP-1 TOML    | **OUT OF SCOPE — moved to 0195 sub-block 2a** (icon kind extended to also persist `name`) |
| `assets.total_supply` (classic credit)  | NUMERIC(28,7) | NULL for classic credit                                  | ✅ SUM of trustline balances                      | sub-block 1b (depends on 1e)                                                              |
| `liquidity_pool_snapshots.volume`       | NUMERIC(28,7) | always NULL                                              | ✅ PathPayment ops + LP swap event delta          | sub-block 1d                                                                              |
| `liquidity_pool_snapshots.fee_revenue`  | NUMERIC(28,7) | always NULL                                              | ✅ derived `volume × fee_bps / 10000`             | sub-block 1d                                                                              |
| `account_balances_current` (trustlines) | row data      | only native XLM populated                                | ✅ TrustLine ledger entries                       | sub-block 1e                                                                              |

Sources hardcoded `None`:

- `crates/xdr-parser/src/state.rs:468` → `tvl: None` (LP, off-chain → 0195)
- `crates/xdr-parser/src/state.rs:484-486` → `tvl/volume/fee_revenue: None` (snapshot, mixed → split)
- Audit doc `docs/audits/2026-04-10-pipeline-data-audit.md` §5.2 line 261-264 confirms

### List-endpoint schema gaps (new columns needed)

`assets.usd_price` + `assets.usd_price_updated_at` are needed for any future stellarchain.io/markets-style sort-by-value on `/v1/assets` list. Captured as 0191 future-work bullet #6 ("Asset USD prices stellarchain.io/markets parity"). Task 0195 will populate them via Lambda 2; this task lands the columns + index.

NFT/LP/transactions/ledgers/contracts list DTOs all map cleanly to existing columns — no schema additions needed for those.

### Why split from sister tasks

- **vs 0195** (Lambda 2 enrichment): 0195 fills off-chain NULL columns. 0195 depends on 0194 sub-block 1a (the `assets.usd_price` column) being merged.
- **vs 0196** (enrichment-backfill crate): 0196 drains pre-existing un-enriched rows for fields populated by 0195 (or 0191's `assets.icon_url`). 0196 depends on 0195 having shared `enrich_*` functions ready.
- **vs 0197** (audit + docs): 0197 is the final verification — confirms every list field is in schema, indexed, and populated.

## Implementation Plan

### Sub-block 1a — Schema migrations (atomic, FIRST commit)

Single migration `crates/db/migrations/{TIMESTAMP}_db-completeness-additions.up.sql` adding:

```sql
-- New columns
ALTER TABLE assets ADD COLUMN usd_price NUMERIC(28,7);
ALTER TABLE assets ADD COLUMN usd_price_updated_at TIMESTAMPTZ;

-- New indexes for soon-to-be-populated fields
CREATE INDEX idx_assets_usd_price
  ON assets (usd_price DESC) WHERE usd_price IS NOT NULL;
CREATE INDEX idx_assets_holder_count
  ON assets (holder_count DESC) WHERE holder_count IS NOT NULL;
CREATE INDEX idx_lp_snapshots_volume
  ON liquidity_pool_snapshots (pool_id, volume DESC) WHERE volume IS NOT NULL;
CREATE INDEX idx_lp_snapshots_fee_revenue
  ON liquidity_pool_snapshots (pool_id, fee_revenue DESC) WHERE fee_revenue IS NOT NULL;
CREATE INDEX idx_abc_balance
  ON account_balances_current (balance DESC) WHERE balance > 0;
```

Down migration: drop in reverse order. Integration test: round-trip migrate up→down→up. ADR 0037 (`current-schema-snapshot`) amended in same PR per ADR 0032 evergreen rule.

**Rust side:** `assets::dto::AssetItem` gets `pub usd_price: Option<String>`; `crates/api/src/assets/queries.rs` SQL extends SELECT. **Trigger CI gate `API types freshness`** — run `npx nx run @rumblefish/api-types:generate` after Rust DTO change, commit `libs/api-types/src/{openapi.json,generated/}` in same commit per CLAUDE.md.

### Sub-block 1b — Classic credit `assets.total_supply`

**Gap origin:** `crates/xdr-parser/src/extract_assets/` only emits Soroban + SAC deployments. Classic credits (USDC, EURT, etc.) reach the DB only via the `account_state` TrustLine path and never carry `total_supply`. The `total_supply` part of 0191 known gap "priority #2 classic credit enrichment" is on-chain (SUM of trustline balances) → indexer per ADR 0026.

**Scope clarification (post-2026-05-06 dry-run audit, see 0197 dry-run notes):** classic credit `assets.name` is **OUT OF SCOPE for this sub-block**. Classic credits have no on-chain `name` field — full names like "USD Coin" come from issuer SEP-1 TOML `CURRENCIES[].name`. Per ADR 0026 (1f) that's off-chain → Lambda 2 territory. Allocated to **0195 sub-block 2a (icon kind extension)** which already fetches the same TOML and can persist `name` alongside `icon_url` in a single fetch. `Sep1Currency.name` field will be added to the DTO there.

For Soroban tokens, `name` continues to be populated by task **0156** (active) — `name` from on-chain `ContractData`. SAC `name` continues to be populated by indexer at deploy time. This sub-block does NOT touch `assets.name`.

**Implementation (`total_supply` only):**

- Audit `crates/indexer/src/handler/persist/staging.rs:1234-1264` PathPayment + ChangeTrust branches
- For classic credit `total_supply`: derivable as `SUM(account_balances_current.balance) WHERE asset_code/issuer matches`. Compute incrementally on trustline writes, persist on the `assets` row.
- Depends on sub-block 1e (trustline balance extraction) being implemented — without trustline rows there is nothing to SUM.

### Sub-block 1c — `assets.holder_count` inline indexer counter

**Reactivates blocked task 0135** (`0135_FEATURE_token-holder-count-tracking`).

- Inline `+1` on `change_trust create` (new trustline), `-1` on `change_trust delete` (trustline removal), no-op on balance updates
- Edge cases: trustline-flag changes, authoreized-to-maintain-liabilities transitions — verify with audit's holder-count semantics
- One-time recount Lambda subcommand needed post-backfill — captured as Future Work, separate ops job
- Wire in `crates/xdr-parser/src/account_state.rs` and `crates/indexer/src/handler/persist/staging.rs` UPSERT path

### Sub-block 1d — LP `volume` + `fee_revenue` (Phase 1 classic AMM)

**Phase 1 scope only — classic AMM via PathPayment ops.** Phase 2 (Soroban DEX adapters: Soroswap, Phoenix) is explicit Future Work, separate task.

Implementation:

- In `crates/indexer/src/handler/persist/staging.rs:1234` PathPayment branch, detect when path contains a `liquidityPoolId` (already extracted on line 1254 for op detail). Compute reserve delta from before/after `LiquidityPoolEntry` ledger entry change.
- Volume contribution per swap = the asset amount that crossed the pool. Increment the **live current snapshot row** for that pool (per the existing snapshot windowing logic — verify whether windowing is hourly/daily and where rollover happens).
- `fee_revenue = volume × fee_bps / 10000` computed in the same write — `fee_bps` lives on `liquidity_pools` row.
- Drop `volume: None, fee_revenue: None` hardcoding at `xdr-parser/src/state.rs:485-486`.

**Audit doc Section 9.3** (`docs/audits/2026-04-10-pipeline-data-audit.md:512-535`) originally proposed scheduled cron Lambda for both TVL **and** volume. The volume part is explicitly overridden here per the field allocation rule — volume is on-chain derivable, no oracle, no HTTP, so it belongs in the indexer. ADR 0026 (sub-block 1f) records this override; ADR 0032 evergreen requires `docs/architecture/indexing-pipeline/**` + `docs/audits/2026-04-10-pipeline-data-audit.md` Section 9.3 amendment in same PR.

### Sub-block 1e — `account_balances_current` trustline balances

Audit finding F7 (`docs/audits/2026-04-10-pipeline-data-audit.md`): `extract_account_states()` populates only native XLM; trustline balances are extracted nowhere despite the column existing. This was task **0119** which is archived — first sub-step is to verify whether 0119 actually completed this work or was archived prematurely.

If 0119 incomplete:

- Extend `crates/xdr-parser/src/account_state.rs` to emit balance rows for every TrustLine ledger entry change (create, modify, delete)
- Wire into `staging.rs` upsert for `account_balances_current`
- Sub-block 1c (holder_count) and sub-block 1b (classic credit `total_supply` SUM) both depend on this being complete — same code path
- All NOT NULL columns in `account_balances_current` (`account_id`, `asset_type`, `asset_code`, `issuer_id`, `balance`, `last_updated_ledger`) populated atomically on every TrustLine row write — schema enforces, INSERT will fail otherwise. Acceptance must spot-check non-NULL on every column for non-XLM rows on backfill region.

### Sub-block 1f — ADR 0026: Field allocation rule

**New ADR** locking the rule: "List endpoint + on-chain (data already in processed ledger) → indexer; off-chain (HTTP / oracle / per-row RPC) → enrichment Lambda 2; detail-only fields → runtime type-2 in API handler, NEVER persisted." References:

- ADR 0029 (abandon-parsed-artifacts-read-time-xdr-fetch) — companion read-time pattern
- Task 0188 (SEP-1 type-2 detail enrichment, the precedent)
- Task 0191 (SQS-driven type-1 enrichment, the precedent)
- Migration `20260424000000_drop_assets_sep1_detail_cols.up.sql` (the precedent for "no detail-only columns")
- Audit doc Section 9.3 (now amended)

This is the linchpin governance doc. Task 0195/0196/0197 reference it verbatim.

## Acceptance Criteria

- [ ] Migration up/down round-trip green; integration test landed
- [ ] `assets.usd_price` + `assets.usd_price_updated_at` columns + 5 new indexes present in schema
- [ ] Sub-block 1b: sample query on backfill region shows non-NULL `total_supply` for classic credit assets (incrementally maintained as `SUM(account_balances_current.balance)` per asset). `name` for classic credits is NOT in this task's scope — see 0195 sub-block 2a (icon kind extension to also persist SEP-1 `name`).
- [ ] Sub-block 1c: `assets.holder_count` non-NULL on backfill region; one-time recount tooling spawned as separate task
- [ ] Sub-block 1d: `liquidity_pool_snapshots.volume/fee_revenue` non-NULL on backfill region for pools with PathPayment activity; Phase 2 DEX adapters spawned as separate task
- [ ] Sub-block 1e: `account_balances_current` shows non-XLM trustline rows on backfill region with all NOT NULL columns populated (`balance`, `last_updated_ledger`, `asset_type`, `asset_code`, `issuer_id`)
- [ ] ADR 0026 merged
- [ ] **Docs updated** per ADR 0032: `docs/architecture/database-schema/**` (column matrix), `docs/architecture/indexing-pipeline/**` (volume/fee_revenue path), `docs/audits/2026-04-10-pipeline-data-audit.md` Section 9.3 amendment, ADR 0037 schema-snapshot refresh
- [ ] **API types regenerated** — `assets::dto::AssetItem` gains `usd_price`, codegen committed in same PR

## Future Work (out of scope, spawn separate tasks)

- **Phase 2 LP volume**: Soroban DEX adapters (Soroswap, Phoenix, etc.) — per-DEX event format, dynamic fees. Spawn after Phase 1 lands.
- **Holder_count one-time recount**: post-backfill ops Lambda subcommand to fully recount. Spawn after 1c lands.
- **Classic credit `assets.name`** moved out of this task entirely — see 0195 sub-block 2a (icon kind extended to also persist `name` from same SEP-1 fetch). Decision rationale: classic credit names are off-chain (issuer SEP-1 TOML `CURRENCIES[].name`) → Lambda 2 per ADR 0026.

## Notes

- **Branching**: cut from develop after 0191 PR merge.
- **Bundling rationale** (per Karol's "bundle related work" rule): 5 sub-blocks 1a-1f are heterogeneous (schema + 4 indexer sub-systems + ADR) but all share the rule "fix what indexer should have populated", same migration, same test surface, same ADR. Splitting per sub-block would be 5 PRs of <100 lines each — micro-decomposition penalty exceeds review-load benefit.
- **0125 disposition**: superseded by 0195 sub-block 2a (LP TVL via Lambda 2). The volume/fee_revenue parts of 0125's scope move to **this** task (1d).
