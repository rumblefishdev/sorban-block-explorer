---
id: '0197'
title: 'DB completeness audit + docs: list/detail field allocation verification, schema coverage matrix'
type: FEATURE
status: backlog
related_adr: ['0026', '0029', '0032', '0037']
related_tasks: ['0188', '0191', '0194', '0195', '0196']
tags: [priority-medium, effort-medium, layer-docs, layer-audit]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-05-06'
    status: backlog
    who: karolkow
    note: 'Spawned from M2 enrichment planning session 2026-05-06. Fourth and final of four tasks (0194-0197). Verifies the field allocation rule (ADR 0026) is followed end-to-end after 0194/0195/0196 land.'
---

# DB completeness audit + docs: list/detail field allocation verification, schema coverage matrix

## Summary

Final verification gate for the 0194-0197 task chain. Audits every list endpoint to confirm every returned field has a DB column that is indexed (where sortable/filterable) and populated (≠ always NULL on a production sample query). Audits every detail endpoint to confirm unique-to-detail fields do **NOT** have dedicated DB columns and are runtime type-2 enrichment instead. Refreshes all `docs/architecture/**` per ADR 0032 evergreen rule. Outputs a one-time snapshot to `docs/audits/`.

## Status: Backlog

Cannot start until **0194, 0195, AND 0196** all merge to develop. This task is purely verification + documentation — its value is exposing remaining gaps after the implementation tasks land. Running it before is wasteful (everything would fail).

## Context

### Field allocation rule (ADR 0026, locked in 0194 sub-block 1f)

> List endpoint + on-chain (data already in processed ledger) → indexer; off-chain (HTTP / oracle / per-row RPC) → enrichment Lambda 2; detail-only fields → runtime type-2 in API handler, NEVER persisted.

This task verifies the rule is followed across the entire codebase post-0194/0195/0196.

### What "list endpoint" means here

Paginated array endpoints currently include (verify exhaustive list during audit):

- `GET /v1/assets`
- `GET /v1/contracts`
- `GET /v1/liquidity-pools`
- `GET /v1/liquidity-pools/:id/participants` (list-of-rows even though under detail path)
- `GET /v1/liquidity-pools/:id/transactions` (same)
- `GET /v1/nfts`
- `GET /v1/nfts/:id/transfers` (same)
- `GET /v1/transactions`
- `GET /v1/operations`
- `GET /v1/ledgers`
- `GET /v1/accounts`

Each list endpoint has a corresponding DTO under `crates/api/src/{module}/dto.rs` (e.g. `AssetItem`, `PoolItem`, `NftItem`).

### What "detail endpoint" means here

`GET /v1/{resource}/:id` returning a single object, currently:

- `GET /v1/assets/:id` (`AssetDetailResponse` — includes `description`, `home_page` from runtime type-2 SEP-1 fetch per task 0188)
- `GET /v1/transactions/:hash`
- etc.

The pattern to verify: detail-only fields (e.g. `description`, `home_page` on assets) come from runtime SEP-1 fetch in the handler (`crates/api/src/assets/handlers.rs:176-201` per audit), NOT from a DB column. The drop migration `crates/db/migrations/20260424000000_drop_assets_sep1_detail_cols.up.sql` removed these columns and the runtime fetch is the new source of truth.

### Anti-patterns to flag

- **Column populated only by detail endpoint** — column exists, is written, but never read by any list endpoint. Candidate for drop (forces type-2 instead).
- **List endpoint field with no DB column AND no in-handler computation** — bug, returns NULL.
- **List endpoint sortable field with no index on the underlying column** — performance bug.
- **List endpoint field marked "populated by indexer" but always NULL on backfill sample** — wiring incomplete.
- **List endpoint field marked "populated by Lambda 2" but always NULL on backfill sample** — wiring incomplete.

## Implementation Plan

### Step 1: Coverage matrix (audit deliverable)

Output `docs/audits/{TIMESTAMP}-list-endpoint-completeness.md` with a single table:

| Endpoint              | DTO field         | DB column   | Indexed?     | Populated by               | Sample query result         |
| --------------------- | ----------------- | ----------- | ------------ | -------------------------- | --------------------------- |
| `/v1/assets`          | `id`              | `assets.id` | PK           | indexer (insert)           | non-NULL                    |
| `/v1/assets`          | `asset_type_name` | computed    | n/a          | SQL CASE                   | non-NULL                    |
| ...                   | ...               | ...         | ...          | ...                        | ...                         |
| `/v1/liquidity-pools` | `tvl`             | `lps.tvl`   | partial DESC | Lambda 2 (lp_tvl, 0195 2a) | (assert non-NULL on sample) |
| ...                   | ...               | ...         | ...          | ...                        | ...                         |

Workflow:

1. For each endpoint, read DTO struct from `crates/api/src/{module}/dto.rs`
2. For each field, trace the SQL query that produces it (canonical SQL files in repo, e.g. `crates/api/src/sql/15_get_assets_list.sql`)
3. Map to DB column or in-SQL computation
4. Lookup column index status from migrations / `\d` of staging DB
5. Identify population owner (indexer / Lambda 2 / handler-computed / SQL-computed)
6. Run sample COUNT-NOT-NULL query on staging or backfill DB

Expected outcome: every list endpoint field is a row in the table with no "FAIL" entries. Any FAIL = bug, spawn follow-up task.

### Step 2: Detail-endpoint anti-pattern sweep

For each `:id` detail endpoint:

- List unique-to-detail fields (in detail DTO but not in list DTO)
- For each, verify NO dedicated DB column (or column is dropped/scheduled to be dropped)
- For each, verify implementation = runtime fetch in handler OR computed in SQL on detail-only query
- Flag candidates to drop

Output: appended section in same audit md. Flag each anti-pattern with proposed fix (drop column, refactor handler).

### Step 3: Docs refresh per ADR 0032 evergreen

Per `lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md`, every PR changing the shape of the system updates `docs/architecture/**`. After 0194/0195/0196 land, the shape has changed substantially. This task picks up any gaps the implementation PRs missed:

- `docs/architecture/database-schema/**` — column matrix, every newly populated column attributed to its source (indexer / Lambda 2 / type-2 handler)
- `docs/architecture/indexing-pipeline/**` — runtime_enrichment umbrella, type-1 SQS model, type-2 runtime model, backfill crate
- `docs/architecture/backend/**` — list vs detail boundary, type-2 detail enrichment pattern, anti-patterns to avoid
- `docs/architecture/xdr-parsing/**` — new responsibilities (volume/fee_revenue/holder_count/classic credit `total_supply`); note `name` for classic credit lives in Lambda 2 (0195 2a), NOT indexer

### Step 4: ADR cross-check

- ADR 0026 (field allocation rule, from 0194) — re-affirm without amendment, OR amend if implementation revealed edge cases
- ADR 0029 (abandon-parsed-artifacts) — confirm 0195's amendment landed
- ADR 0037 (current-schema-snapshot) — confirm 0194's amendment landed

### Step 5: Audit doc supersession

`docs/audits/2026-04-10-pipeline-data-audit.md` Section 8/9 referenced multiple "write-only columns" + "enrichment pipeline gap" findings that are now obsolete. Add a header block to that doc noting it is "partially superseded by 2026-MM-DD list-endpoint-completeness.md" and link forward.

### Step 6: Spawn follow-up tasks for any FAIL findings

Each anti-pattern or wiring gap discovered → backlog task with `audit-gap` tag. Don't fix in this task — this task is the meta-audit, fixes go elsewhere.

## Acceptance Criteria

- [ ] `docs/audits/{TIMESTAMP}-list-endpoint-completeness.md` committed
- [ ] Every list endpoint field has a row in the matrix with non-FAIL status (or follow-up task spawned for each FAIL)
- [ ] Every detail endpoint anti-pattern flagged with proposed fix (or no anti-patterns found)
- [ ] `docs/architecture/database-schema/**` refreshed
- [ ] `docs/architecture/indexing-pipeline/**` refreshed
- [ ] `docs/architecture/backend/**` refreshed
- [ ] `docs/architecture/xdr-parsing/**` refreshed
- [ ] ADR 0026/0029/0037 cross-checked
- [ ] Audit doc 2026-04-10 supersession header added
- [ ] **Docs updated** — this is the task, mark all checked
- [ ] **API types regenerated** — N/A (audit-only, no code changes expected)

## Future Work (out of scope)

- **Periodic completeness check**: this audit is one-shot. If we want continuous protection, add a CI gate that diffs `docs/architecture/database-schema/**` against actual schema and fails if drift detected. Captured as a separate optional task.
- **API contract test**: end-to-end test that hits each list endpoint on a sample DB and asserts no NULL fields. More expensive than this audit. Defer until production complaints surface.

## Notes

- **Order in chain**: this task is intentionally LAST. Running it before 0194/0195/0196 land would surface tons of pre-existing failures — wasted effort. Running it after gives a clean baseline.
- **Skill invocation**: while writing the audit md, follow `/lore-framework` documentation patterns. While committing, follow `/lore-framework-git`. While archiving 0125 (now superseded), follow `/lore-framework-tasks`.
- **Sentinel-aware sample queries**: when checking "non-NULL" for `assets.icon_url`, treat both real URL and `''` sentinel as "populated" — see 0191 design decision #12 + sentinel taxonomy from 2026-05-06 session.
- **Dry-run audit performed 2026-05-06** (during planning session, BEFORE 0194/0195/0196 spawn): confirmed 95% pre-coverage, surfaced 1 misallocation (classic credit `assets.name` was placed in 0194 indexer, should be Lambda 2 — fixed via amendment to 0194 1b + new 0195 2a icon-name extension). Real run of this task should not need to surface that issue again. Other dry-run flags (`AssetDetailResponse.deployed_at_ledger`, `account_balances_current.first_deposit_ledger`) were false positives — first is a legitimate entity-record column read by `/v1/search`, second doesn't exist in that table (subagent confused with `lp_positions.first_deposit_ledger`). Real audit should still verify these independently.
