---
title: 'ADR 0001-0036 → docs/architecture impact matrix'
type: generation
status: mature
spawned_from:
  - ../README.md
tags: [docs, audit, adr, matrix]
history:
  - date: '2026-04-23'
    status: mature
    who: Karol Kowalczyk
    note: >
      Step 1 output for task 0155. Built from ADR frontmatter + bodies
      and grounded against `crates/db/migrations/**` as of 2026-04-23
      (post-ADR 0036 rename `tokens → assets` landed via task 0154).
  - date: '2026-04-24'
    status: mature
    who: Karol Kowalczyk
    note: >
      Scope expanded from ADRs 0022-0031 to full 0001-0036 range per
      stakeholder decision to eliminate "docs partially stale after
      merge" risk. Matrix below now carries:
      (a) process/infra/API ADRs 0001-0010 (LIVE — BE/IX/IN/XD impact),
      (b) superseded schema chain 0011-0021 (mostly obsolete via 0029;
          LIVE reference entries kept),
      (c) core rework 0022-0031 (unchanged from prior version),
      (d) evergreen policy 0032 (drives Step 4),
      (e) post-0031 refinements 0033-0036 (LIVE).
      ADR 0035 pre-applied ahead of task 0159 to eliminate the
      docs↔migration race risk.
  - date: '2026-04-24'
    status: mature
    who: Karol Kowalczyk
    note: >
      3rd pass after merging origin/develop. Findings: task 0159
      landed on develop (2026-04-24) dropping `account_balance_history`
      from migrations + indexer code; my pre-apply now matches code
      reality — updated DB §4.18 to drop "migrations still carry"
      language. ADR 0035 flipped `proposed → accepted` (status column
      updated below). No new ADRs 0037+ on develop. New active tasks
      noted: 0160 (SAC asset identity bug — no schema change),
      0163 (operations → operations_appearances refactor — ADR not
      written yet, code unchanged; future doc sweep trigger when it
      lands). New backlog: 0161, 0162. Only bookkeeping — no doc
      reshapes required this pass.
---

# Matrix: ADR 0001 → 0036 impact on `docs/architecture/**`

Purpose: drive the per-file reconciliation sweep of task 0155. Each
column below names the doc file(s) that must reflect the ADR's
outcome. "§" refers to H2 sections; `—` = no expected change.

Legend for docs:

- **TD** — `docs/architecture/technical-design-general-overview.md`
- **DB** — `docs/architecture/database-schema/database-schema-overview.md`
- **BE** — `docs/architecture/backend/backend-overview.md`
- **FE** — `docs/architecture/frontend/frontend-overview.md`
- **IX** — `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md`
- **IN** — `docs/architecture/infrastructure/infrastructure-overview.md`
- **XD** — `docs/architecture/xdr-parsing/xdr-parsing-overview.md`
- **AU** — `docs/database-audit-first-implementation.md`

## Matrix — Process / Infrastructure / API-Contract ADRs (0001–0010)

All LIVE. Mostly BE / IX / IN / XD impact.

| ADR  | Status   | Decision in one line                                                      | TD  | DB  | BE     | FE  | IX     | IN            | XD  | AU  |
| ---- | -------- | ------------------------------------------------------------------------- | --- | --- | ------ | --- | ------ | ------------- | --- | --- |
| 0001 | accepted | OIDC CI/CD + AWS Secrets Manager; no secrets in git                       | —   | —   | —      | —   | —      | §6 (security) | —   | —   |
| 0002 | accepted | Ledger Processor Lambda written in Rust (`crates/indexer`)                | §4  | —   | —      | —   | §5, §7 | §5            | §3  | —   |
| 0004 | accepted | Rust-only XDR parsing; `stellar-xdr` crate is the single decoder          | §5  | —   | §2     | —   | §5     | §5            | §3  | —   |
| 0005 | accepted | Backend API also Rust (axum + utoipa + sqlx + lambda_http)                | §2  | —   | §10    | —   | —      | §5            | —   | —   |
| 0006 | accepted | No S3 lifecycle on `stellar-ledger-data` bucket                           | —   | —   | —      | —   | —      | §5            | —   | —   |
| 0007 | accepted | 2-Lambda architecture (Ledger Processor + REST API; no Event Interpreter) | §3  | —   | §5     | —   | §7     | §3            | —   | —   |
| 0008 | accepted | Error envelope `{code, message, details}` + cursor-based pagination       | —   | —   | §6, §7 | §8  | —      | —             | —   | —   |
| 0010 | accepted | Local `backfill-bench` CLI (no Fargate for backfill)                      | §4  | —   | —      | —   | §6     | §5, §7        | —   | —   |

## Matrix — Schema Evolution Chain (0011–0021)

Mostly OBSOLETE via ADR 0029 (S3 offload abandoned). LIVE survivors called out.

| ADR       | Status                                                                   | Decision in one line                                                 | Surviving impact                                                                                                                                     |
| --------- | ------------------------------------------------------------------------ | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0011–0018 | proposed (superseded by 0029 for S3 part; schema shape subsumed by 0027) | Schema iterations in the "lightweight DB + S3 offload" era           | Column/index/partitioning decisions that survived are in 0027's schema; DB overview §4 describes final state. No separate doc-level treatment needed |
| 0019      | proposed                                                                 | Schema snapshot + sizing reference at 11M ledgers                    | LIVE as capacity baseline; DB overview §6.3 references retention model                                                                               |
| 0020      | proposed                                                                 | Drop `transaction_participants.role` + drop `idx_contracts_deployer` | LIVE — DB overview §4.5 describes `transaction_participants` as 3+PK cols; §4.6 omits deployer index                                                 |
| 0021      | proposed                                                                 | Schema ↔ endpoint ↔ frontend coverage matrix (22 endpoints)          | LIVE as reference; BE overview §6 carries current endpoint list post-0029 (E3/E14 call archive)                                                      |

## Matrix — Core Schema Rework (0022–0031)

| ADR  | Status                               | Decision in one line                                                            | TD         | DB             | BE               | FE       | IX         | IN     | XD     | AU                                                                    |
| ---- | ------------------------------------ | ------------------------------------------------------------------------------- | ---------- | -------------- | ---------------- | -------- | ---------- | ------ | ------ | --------------------------------------------------------------------- |
| 0022 | proposed (Part 3 superseded by 0023) | Schema snapshot correction + async metadata enrichment worker                   | §6         | §4, §5         | §7               | —        | §7         | §5     | —      | §assets                                                               |
| 0023 | proposed                             | Typed SEP-1 columns on `assets`: `description`, `icon_url`, `home_page`         | §6.7       | §4 assets      | §6 endpoints     | —        | —          | —      | —      | §assets                                                               |
| 0024 | proposed                             | All hashes + `pool_id` → `BYTEA(32)` with `octet_length=32` CHECK               | §6         | §4 (hash cols) | §6               | —        | —          | —      | §4, §6 | §ledgers, §transactions, §liquidity_pools                             |
| 0025 | superseded by 0027                   | Pre-surrogate final schema v1 (22 endpoints feasible)                           | §6         | §3, §4         | §6               | §6, §8   | —          | —      | —      | —                                                                     |
| 0026 | accepted                             | `accounts.id BIGSERIAL` surrogate; 17 `_id` BIGINT FK renames                   | §6         | §3, §4         | §7               | —        | §4, §7     | —      | §6     | §accounts + all FK sections                                           |
| 0027 | superseded by 0030                   | Post-accounts-surrogate snapshot + 22/22 endpoint SQL proof                     | §6         | §3–§6          | §6 endpoints     | §6 pages | §5, §7     | —      | §6     | all tables                                                            |
| 0028 | superseded by 0029                   | ParsedLedgerArtifact v1 JSON shape (never shipped)                              | — (none)   | —              | —                | —        | —          | —      | —      | —                                                                     |
| 0029 | proposed                             | Abandon parsed-ledger S3 artifacts; fetch XDR on-demand for E3/E14              | §3, §4, §5 | —              | §6 (E3, E14), §7 | —        | §3, §5, §6 | §3, §5 | §1, §3 | —                                                                     |
| 0030 | accepted                             | `soroban_contracts.id BIGSERIAL` surrogate; 5 `contract_id` → BIGINT FK renames | §6         | §3, §4         | §7               | —        | §5, §7     | —      | §5, §6 | §soroban_contracts + dependents                                       |
| 0031 | accepted                             | 9 enum VARCHAR cols → SMALLINT + Rust enum + CHECK range + `_name` SQL helpers  | §6         | §4, §6         | §5, §7           | —        | §7         | —      | —      | §operations, §tokens→assets, §nfts, §soroban_events, §liquidity_pools |

## Matrix — Governance + Post-0031 Refinements (0032–0036)

| ADR  | Status                                 | Decision in one line                                                                                         | TD       | DB                            | BE                      | FE            | IX       | IN       | XD               | AU                                  |
| ---- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------ | -------- | ----------------------------- | ----------------------- | ------------- | -------- | -------- | ---------------- | ----------------------------------- |
| 0032 | accepted                               | `docs/architecture/**` evergreen maintenance policy                                                          | §preface | §preface                      | §preface                | §preface      | §preface | §preface | §preface         | §header                             |
| 0033 | accepted                               | `soroban_events` → `soroban_events_appearances` (appearance index, read-time detail via archive)             | §6.6     | §4.8                          | §6 events endpoint      | —             | §5       | —        | §4.4, §5.1       | §soroban_events (stale-notice)      |
| 0034 | accepted                               | `soroban_invocations` → `soroban_invocations_appearances` (appearance index + `caller_id`, read-time detail) | §6.5     | §4.9                          | §6 invocations endpoint | —             | §5       | —        | §4.5, §5.2, §5.3 | §soroban_invocations (stale-notice) |
| 0035 | accepted (task 0159 landed 2026-04-24) | Drop `account_balance_history` (unused denormalization)                                                      | §6.12    | §4 (remove §4.18 + §6.2 list) | —                       | —             | §5.3     | —        | §4.6             | —                                   |
| 0036 | accepted                               | Rename `tokens` → `assets`; `classic` → `classic_credit`                                                     | §6.7     | §4.10                         | §6 assets endpoint      | §6 asset page | §5       | —        | —                | §assets (stale-notice)              |

~~Pre-application notice:~~ **Resolved 2026-04-24 (3rd pass).** The pre-apply
bet paid off — task 0159 landed on develop the same day, migrations dropped
`account_balance_history`, ADR 0035 flipped to `accepted`. Docs and code are
now consistent without a second PR. The pre-apply language ("migrations still
carry the table") was removed from DB §4.18 on 2026-04-24 after the merge.

## Post-0155 backlog watch (future doc sweep triggers)

Tasks created / activated on 2026-04-24 that are **not** in 0155 scope but
will require a doc-sync PR when they land (per ADR 0032). Logged here so
the next ADR / task PR remembers to update the architecture docs at merge
time:

| Task | Type     | Status                                            | Impact when it lands                                                                                                                                                                                                                                                                                                               |
| ---- | -------- | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0160 | BUG      | active                                            | SAC asset identity extraction — fix. **No schema change**; no doc reshape required                                                                                                                                                                                                                                                 |
| 0161 | BUG      | backlog                                           | Native XLM singleton seed in `assets` — migration seed INSERT. Minor DB §4.10 narrative note (4 variants always populated after seed)                                                                                                                                                                                              |
| 0162 | FEATURE  | backlog                                           | Parser stops dropping `pool_share` trustlines; producer for `lp_positions`. Minor IX §5 / XD §4.6 note (pool_share now produces `lp_positions` upserts instead of being skipped)                                                                                                                                                   |
| 0163 | REFACTOR | **RESOLVED 4th pass (PR #118 merged 2026-04-24)** | `operations → operations_appearances` refactor. Applied in 0155 4th pass: DB §4.4 rewrite, TD §6.3 rewrite, TD ASCII + pipeline, IX §5.2 step 6, XD §4.3 rewrite, MIGRATIONS.md 0003/0006/partition-list. `transfer_amount` + `application_order` dropped; `amount BIGINT` + `uq_ops_app_identity UNIQUE NULLS NOT DISTINCT` added |

## Key ground-truth facts (from `crates/db/migrations/**` at 2026-04-24 post-merge)

These are the authoritative "what the docs must now say":

1. **Table `tokens` is renamed to `assets`** (ADR 0036 / task 0154 — landed before this audit).
   All constraint and index names follow: `ck_assets_*`, `uidx_assets_*`, `idx_assets_*`.
2. **`accounts.id BIGSERIAL PK`** + `account_id VARCHAR(56) UNIQUE`. Every account FK column in
   other tables is `BIGINT REFERENCES accounts(id)` with an `_id` suffix
   (`source_id`, `destination_id`, `issuer_id`, `deployer_id`, `owner_id`,
   `current_owner_id`, `asset_issuer_id`, etc.).
3. **`soroban_contracts.id BIGSERIAL PK`** + `contract_id VARCHAR(56) UNIQUE`. Every contract
   FK column is `BIGINT REFERENCES soroban_contracts(id)` (operations, soroban_events,
   soroban_invocations, assets, nfts).
4. **Hashes are `BYTEA(32)`** with `CHECK (octet_length(<col>) = 32)`:
   `ledgers.hash`, `transactions.hash`, `transactions.inner_tx_hash`,
   `transaction_hash_index.hash`, `soroban_contracts.wasm_hash`,
   `wasm_interface_metadata.wasm_hash`, `operations.pool_id` (nullable),
   `liquidity_pools.pool_id`, `liquidity_pool_snapshots.pool_id`.
   Domain layer: `Hash32` newtype; serde renders lowercase hex. **API surface unchanged**.
5. **Enum columns are `SMALLINT`** with CHECK range + Rust `#[repr(i16)]` enums + SQL helper
   functions (`op_type_name`, `event_type_name`, `token_asset_type_name`,
   `nft_event_type_name`, `lp_asset_type_name`, `contract_type_name`):
   - `operations.type` (0-127)
   - `soroban_events.event_type` (0-15)
   - `assets.asset_type` (0-15): `0=native`, `1=classic_credit`, `2=sac`, `3=soroban`
   - `account_balances_current.asset_type` (0-15)
   - `nft_ownership.event_type` (0-15): `0=mint`, `1=transfer`, `2=burn`
   - `liquidity_pools.asset_a_type`, `liquidity_pools.asset_b_type` (0-15)
   - `soroban_contracts.contract_type` (0-15; nullable) — task 20260422000100 added `nft`/`fungible_token` variants.
6. **Partitioning**: `PARTITION BY RANGE (created_at)` monthly on `transactions`,
   `operations`, `transaction_participants`, `soroban_events`, `soroban_invocations`,
   `nft_ownership`, `liquidity_pool_snapshots`.
   Partitions provisioned by partition-management Lambda (task 0139, `crates/db-partition-mgmt`).
   **No `operations_pN` naming** — follow monthly `operations_y{YYYY}m{MM}` convention.
7. **No parsed-ledger S3 artifact** (ADR 0029). Ingest writes directly to RDS (ADR 0027).
   Heavy read fields for `/transactions/:hash` (E3) and `/contracts/:id/events` (E14)
   fetch raw `.xdr.zst` from the public Stellar ledger archive at request time.
   No internal parsed-JSON bucket.
8. **Assets table shape** (post-rename):
   - 4 `asset_type` variants (0-3), **not 3** — `native` is the distinct fourth.
   - `contract_id BIGINT REFERENCES soroban_contracts(id)` — not VARCHAR(56).
   - `issuer_id BIGINT REFERENCES accounts(id)` — not VARCHAR(56) `issuer_address`.
   - Partial UNIQUE indexes + `ck_assets_identity` CHECK (not plain `UNIQUE`).
   - Typed metadata columns from ADR 0023: `description TEXT`, `icon_url VARCHAR(1024)`,
     `home_page VARCHAR(256)`. Plus ADR 0022 stock columns: `total_supply NUMERIC(28,7)`,
     `holder_count INTEGER`, `name VARCHAR(256)`.

## Drift hotspots pre-identified

From task 0154's research note §5.2 (assets-vs-tokens-taxonomy):

- **TD §6.7 `tokens` → `assets` table description**: 5 concrete mismatches
  (3 vs 4 variants, VARCHAR vs SMALLINT, plain vs partial UNIQUE, missing
  `ck_assets_identity`, VARCHAR(56) vs BIGINT `contract_id` FK).
- **TD §6** analogous drift in `transaction_hash_index`,
  `transaction_participants`, `wasm_interface_metadata`, `lp_positions`
  (if mentioned), `nft_ownership`, `account_balances_current`,
  `account_balances_history`.

From grep sweep (task 0139 worklog, as reported in 0155 task frontmatter):

- **TD**: 16 stale schema hits per grep — main sink.
- `docs/database-audit-first-implementation.md`: post-0139 partial clean-up
  for operations partitioning (lines 130, 136-140 already corrected); rest
  needs sweep for `tokens → assets` + SMALLINT enums + surrogate FKs.

## Reconciliation order (coordination)

Order chosen to let each later pass benefit from the earlier one's fixes:

1. **DB first** — `database-schema-overview.md` is the canonical reference the
   other files link into. Fix it first, then other files can cite it.
2. **AU** — `database-audit-first-implementation.md` shares subjects with DB.
3. **TD** — the big sink; §6 (data model) pulls from DB, rest cross-cuts.
4. **BE** — endpoints; §6/§7 reflect DB column shapes.
5. **IX** — pipeline; ADRs 0026/0029/0030 change ingest wiring.
6. **XD** — xdr-parsing; ADRs 0024/0030 change type boundaries.
7. **IN** — infrastructure; ADR 0029 removes the parsed-ledger bucket.
8. **FE** — frontend; surface-level (API shapes unchanged); should be the
   lightest pass — only flagged if docs reference old table/column names.

## Out of scope

- **Research note §6.6** (XLM-SAC linkage gap) — data-model question, not a
  doc-drift question. Left for a dedicated data-model task if/when it blocks
  a user-visible issue.
- **`docs/database-audit-first-implementation.md` full regeneration** — this
  is a point-in-time per-table audit with write-path file:line refs; fully
  regenerating it is orthogonal to `docs/architecture/**` sync. Stakeholder
  decision on 2026-04-24 was to **preserve the snapshot** as a historical
  artifact rather than regenerate. Task 0155 leaves a top-level
  historical-snapshot notice and per-section markers on the three
  most-changed tables; no follow-up planned.
- **ADRs 0003 + 0009** — pure process/CI ADRs with no `docs/architecture/**`
  surface (task milestones, staging deploy trigger). Listed in the ADR
  survey but carry `—` across every doc-file column above.

## Previously "out of scope" — now in scope (2026-04-24 expansion)

- ADR 0033, 0034 — physically already applied to the docs during the
  2026-04-23 pass because the old table names (`soroban_events`,
  `soroban_invocations`) do not exist in migrations. The 2026-04-24 scope
  expansion formalised these as in-scope rather than "collateral".
- ADR 0035 — pre-applied to docs in this task to remove the
  docs↔migration race with task 0159.
- ADR 0036 — already applied by task 0154 before 0155 started; now
  listed as in-scope since the task frontmatter covers the full ADR set.
