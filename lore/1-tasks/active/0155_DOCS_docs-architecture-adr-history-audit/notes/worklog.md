---
title: 'Per-file reconciliation worklog'
type: generation
status: developing
spawned_from:
  - ../README.md
  - ./G-adr-doc-matrix.md
tags: [docs, audit, worklog]
history:
  - date: '2026-04-23'
    status: developing
    who: Karol Kowalczyk
    note: >
      Step 2 output of task 0155. One entry per doc file with verdict
      (no changes / minor sync / rewritten) and a diff summary.
  - date: '2026-04-24'
    status: developing
    who: Karol Kowalczyk
    note: >
      Scope expanded from ADRs 0022-0031 to full 0001-0036 range.
      Second pass added:
      (a) ADR 0035 pre-applied — account_balance_history removed from
          every doc (DB §4.18, §3 sketch, §3.3 enum list, §5.3, §6.2,
          §7.3; TD §6.12, §6.8 accounts pointer, ASCII diagram; IX §5.2
          step 14, §5.3; XD §6.1);
      (b) ADR 0020 drift fix — DB §4.5 transaction_participants had
          spurious `role SMALLINT` column; corrected to 3-column schema
          matching migrations;
      (c) ADR 0010 drift fix — IN §2.2, §5.2, §6.4; TD §3 connections,
          §3 components table, §4.3 Historical Backfill; IX §6
          Historical Backfill Flow all updated: backfill is a local
          `crates/backfill-bench` CLI, not a Fargate task.
      ADRs 0033/0034/0036 formalised as in-scope (were collateral).
---

# Per-file reconciliation worklog (Step 2)

Order matches `G-adr-doc-matrix.md` §"Reconciliation order".

| #   | File                                                                | Verdict                         | Summary                                                                                                                                                                                             |
| --- | ------------------------------------------------------------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `docs/architecture/database-schema/database-schema-overview.md`     | rewritten + 2nd-pass fixes      | §1, §2, §3, §4 (all subsections + §4.5 TP fix, §4.18 dropped per 0035), §5, §6, §7.3, §8 reconciled — see below                                                                                     |
| 2   | `docs/database-audit-first-implementation.md`                       | minor sync (snapshot preserved) | stale-notice + per-section pointers for three most-changed tables; snapshot intentionally not regenerated — it's a historical artifact, current shape lives in `database-schema-overview.md`        |
| 3   | `docs/architecture/technical-design-general-overview.md`            | rewritten + 2nd-pass fixes      | §2.2, §3 connections + components, §4 pipeline + §4.2 + §4.3 (ADR 0010 local backfill), §5 XDR parsing, §6 every schema block (§6.8 + §6.12 updated for ADR 0035), ASCII diagrams, acceptance tests |
| 4   | `docs/architecture/backend/backend-overview.md`                     | minor sync                      | §4.1 raw-XDR bullet rewritten for ADR 0029; surrogate-key resolution bullet added; §4.2 dependency list updated; §7.1 source-of-data reflects read-time archive fetch                               |
| 5   | `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` | rewritten + 2nd-pass fixes      | §5.2 (14-step `persist_ledger`, ADR 0035 step 14), §5.3 write-target inventory, §6 Historical Backfill Flow rewrite (ADR 0010 local CLI), §8.3 `stellar-xdr`                                        |
| 6   | `docs/architecture/xdr-parsing/xdr-parsing-overview.md`             | rewritten + 2nd-pass fixes      | all sections touched; §6.1 storage contract updated for ADR 0035                                                                                                                                    |
| 7   | `docs/architecture/infrastructure/infrastructure-overview.md`       | minor sync + 2nd-pass           | §2.2 managed-runtime list (ADR 0010 backfill-bench), §2.3 ingestion path, §5.3 Ledger Processor Rust, §5.4 read-time archive, §6.4 dependency boundary (ADR 0010), §8 `stellar-xdr`                 |
| 8   | `docs/architecture/frontend/frontend-overview.md`                   | no changes                      | no schema-shape references; API response fields (`envelope_xdr`, etc.) preserved because API surface unchanged                                                                                      |

## Per-file notes

### 1. `database-schema/database-schema-overview.md`

**Verdict:** rewritten (major). **ADRs applied:** 0022, 0023, 0024, 0026, 0027, 0030, 0031, 0032, 0036 (collateral: 0033, 0034, 0035 linked-only).

Changes:

- **§1 Purpose and Scope** — removed the "skeletal implementation" disclaimer.
  Doc now declares itself authoritative-for-schema per ADR 0032.
- **§2 Ownership and Design Goals** — reworked §2.1 principles (surrogate FKs,
  `SMALLINT` enums, `BYTEA(32)` hashes, typed-over-JSONB). Reworked §2.2 "not"
  list to acknowledge the read-path dependency on the public Stellar archive
  per ADR 0029.
- **§3 Schema Shape Overview** — complete rewrite of the table inventory and the
  relationship sketch. Added new §3.1 (surrogate key discipline, ADRs 0026/0030),
  §3.2 (binary hashes, ADR 0024), §3.3 (enum columns, ADR 0031).
- **§4 Table Design** — every DDL block replaced to match live migrations:
  - §4.1 Ledgers — `hash BYTEA(32)` + CHECK.
  - §4.2 Transactions — composite PK, partitioned, `source_id BIGINT` FK,
    removed `envelope_xdr`/`result_xdr`/`result_meta_xdr`/`operation_tree`/
    `result_code`/`memo_type`/`memo` per ADR 0029 read-time XDR.
  - §4.3 **NEW** Transaction Hash Index — explains why the `hash` uniqueness
    lives off the partitioned parent.
  - §4.4 Operations — `type SMALLINT`, surrogate FKs, composite PK, cascade via
    composite FK; removed `details JSONB`.
  - §4.5 **NEW** Transaction Participants.
  - §4.6 Soroban Contracts — `id BIGSERIAL` surrogate + `contract_id UNIQUE`,
    `wasm_hash BYTEA`, `deployer_id BIGINT`, `contract_type SMALLINT`.
  - §4.7 **NEW** WASM Interface Metadata.
  - §4.8 Soroban Events — renamed to `soroban_events_appearances` (ADR 0033
    collateral — old `soroban_events` table does not exist in migrations);
    described as pure appearance index with read-time XDR expansion.
  - §4.9 Soroban Invocations — renamed to `soroban_invocations_appearances`
    (ADR 0034 collateral).
  - §4.10 Assets — already post-ADR 0036/0031 but enriched ADR links.
  - §4.11 Accounts — `id BIGSERIAL` surrogate + `account_id UNIQUE`, removed
    the JSONB `balances` (balances live in §4.17/§4.18).
  - §4.12 NFTs — `contract_id BIGINT` FK, `current_owner_id BIGINT` FK.
  - §4.13 **NEW** NFT Ownership — partitioned history with `event_type SMALLINT`.
  - §4.14 Liquidity Pools — `pool_id BYTEA(32)`, typed `asset_*_type` SMALLINT
    with `asset_*_code` + `asset_*_issuer_id` pair.
  - §4.15 Liquidity Pool Snapshots — composite PK, `pool_id BYTEA`, typed
    `reserve_a`/`reserve_b` columns.
  - §4.16 **NEW** LP Positions.
  - §4.17 **NEW** Account Balances (Current).
  - §4.18 **NEW** Account Balance History (with ADR 0035 drop-notice link).
- **§5 Relationships and Data Flow** — updated §5.2 cascade list
  (added `nft_ownership`, renamed appearance tables); rewrote §5.3 to explain
  Pattern A / Pattern B surrogate-key resolution.
- **§6 Indexing, Partitioning, and Retention** — expanded §6.1 index-type
  inventory (GIN, trigram, partial, prefix), added BYTEA/SMALLINT economics
  note. Rewrote §6.2 partition list for current physical tables (includes
  `nft_ownership`, `account_balance_history`, renamed appearance tables).
- **§7 Read and Write Patterns** — rewrote §7.3 to replace "raw XDR in
  transactions" description with the ADR 0029 split: typed summary columns in
  DB, heavy fields fetched at read time from the public archive.
- **§8 Evolution Rules and Delivery Notes** — updated §8.2 implementation
  status; added ADR 0032 evergreen pointer.

**Out-of-scope drift noted:** ADR 0033 (events → appearances), ADR 0034
(invocations → appearances) are physically present as renamed tables so were
described in §4.8/§4.9; their full rationale is in their own ADRs. ADR 0035
(`account_balance_history` drop) is still an active task (0159) so §4.18
remains but links ADR 0035 for awareness.

### 2. `database-audit-first-implementation.md`

**Verdict:** minor sync — historical-snapshot notice added; body intentionally
preserved as a point-in-time artifact.

Rationale: this doc is a 2026-04-15 per-table audit (column, write-paths with
`file:line` refs, post-insert mutability). By stakeholder decision in task 0155
(2026-04-24) the two documents serve different purposes and both are valuable:

- `database-audit-first-implementation.md` = **historical snapshot** with
  `file:line` write-path references from the 2026-04-15 codebase
- `database-schema-overview.md` = **living design reference**, maintained
  evergreen per ADR 0032

The audit body is therefore **not** regenerated; trying to keep a snapshot
"current" would defeat its purpose. Readers looking for current shape are
redirected to the overview + live migrations via the header notice.

The audit file is also not under `docs/architecture/**` (it lives at the
`docs/` root), so it is not in scope for the evergreen policy.

Changes applied:

- Header rewritten as a prominent **historical-snapshot notice** listing the
  ADRs that landed after the generation date and redirecting readers to the
  authoritative current-state sources.
- ToC updated: `tokens` → `assets`; `soroban_events` / `soroban_invocations`
  flagged as superseded by their `_appearances` counterparts.
- Per-section historical-context notices on the three most-changed tables
  (`soroban_events`, `soroban_invocations`, `tokens` → `assets`) pointing
  readers to the current shape in `database-schema-overview.md`.

**No follow-up planned.** The chip spawned earlier in the 2026-04-23 pass to
regenerate this audit was dismissed on 2026-04-24 — regeneration would
destroy the snapshot's value as a historical artifact.

### 3. `technical-design-general-overview.md`

**Verdict:** rewritten (major, multiple sections). **ADRs applied:** 0022, 0023, 0024, 0026, 0027, 0029, 0030, 0031, 0036 (collateral: 0033, 0034).

Changes:

- **§2.2 Backend responsibilities** — rewrote the "Raw XDR on demand" bullet and
  the "no Horizon dependency" paragraph to reflect ADR 0029: heavy-field
  endpoints fetch from the public Stellar archive at request time.
- **§4.1 Indexing Pipeline diagram** — 11-step Ledger Processor flow updated
  to match the current 14-step `persist_ledger` (ADR 0027). Removed "envelope
  XDR, result XDR" storage step; replaced event/invocation extraction steps
  with their appearance-index counterparts (ADRs 0033/0034); added surrogate-
  key resolution step (ADRs 0026/0030); switched parser from `@stellar/stellar-sdk`
  to the Rust `stellar-xdr` crate (matches ADR 0004/0005).
- **§4.2 What `LedgerCloseMeta` Contains** — clarified the public Stellar archive
  is a read-time dependency for heavy fields, not an ingest-time one.
- **§4.6 Protocol upgrades** — flipped the pinned dep from JS SDK to Rust crate.
- **§5 XDR Parsing (all subsections)** — complete rewrite of §5.1–§5.4. Ingest
  writes typed summary columns + appearance indexes only; parsed event /
  invocation detail is re-expanded at read time via `xdr_parser::extract_*`.
- **§6 Database Schema (all subsections)** — every DDL snippet replaced to match
  live migrations. Key changes:
  - Added cross-cutting disciplines banner linking ADRs 0024, 0026, 0029, 0030, 0031
  - §6.1 ledgers — hash BYTEA
  - §6.2 transactions — partitioned, composite PK, BYTEA hash, surrogate source_id,
    removed raw-XDR columns (ADR 0029)
  - §6.3 operations — partitioned, composite PK + FK, SMALLINT type, surrogate FKs,
    BYTEA pool_id, removed JSONB details
  - §6.4 soroban_contracts — BIGSERIAL id + contract_id UNIQUE, BYTEA wasm_hash,
    surrogate deployer_id, SMALLINT contract_type
  - §6.5 / §6.6 — renamed to `_appearances` with read-time detail (ADRs 0033/0034)
  - §6.7 assets — existing ADR-0036/0031 shape enriched with metadata-worker ADR 0022/0023 context
  - §6.8 accounts — BIGSERIAL surrogate, removed JSONB balances
  - §6.9 nfts — BIGINT FK, surrogate owner FK, pointer to nft_ownership partitioned history
  - §6.10 liquidity*pools — BYTEA pool_id, typed asset*\*\_type per leg, removed JSONB asset blobs
  - §6.11 liquidity_pool_snapshots — composite PK, typed reserve_a/reserve_b
  - §6.12 Partitioning — complete table re-inventory per live migrations +
    correct `<table>_y{YYYY}m{MM}` naming
- **ASCII diagrams** — table-inventory diagram in §3 updated to include current
  physical table names (`soroban_events_appearances`,
  `soroban_invocations_appearances`, `wasm_interface_metadata`, `nft_ownership`,
  `lp_positions`, `account_balances_current`, `account_balance_history`).
- **§7 Estimates** — left unchanged; the acceptance-test list in Deliverable 1
  (`soroban_events` → `soroban_events_appearances`) updated to match current
  table name.

**Out-of-scope drift noted:** same collateral as DB overview — ADRs 0033/0034
described in §5/§6 because the physical tables renamed; ADR 0035 (`account_balance_history`
drop) still pending task 0159.

### 4. `backend/backend-overview.md`

**Verdict:** minor sync. **ADRs applied:** 0026, 0029, 0030.

Changes:

- **§4.1** — Rewrote the "Raw XDR passthrough" bullet to reflect ADR 0029: no raw
  XDR stored, archive fetched at request time for E3/E14. Added a new bullet
  describing the ADR 0026/0030 surrogate-key resolution pattern at the API
  boundary (Pattern A / Pattern B).
- **§4.2** — Updated the "must not do" list to replace the over-strong "no
  external chain API" claim with the accurate "no private chain API; public
  archive is a read-time dep" framing.
- **§7.1** — Rewrote "Source of Data" to match reality: list endpoints
  DB-local, heavy-field endpoints pull from the public archive.

Out-of-scope observations:

- The endpoint URL table (§6.2) and example JSON bodies (§6.3) already use
  the post-ADR-0036 `assets` naming and the response fields `source_account`,
  `envelope_xdr`, etc. survive because the public API surface is unchanged
  (fields are populated from archive fetch at read time).
- The module list in §5 still uses generic names ("Network", "Transactions",
  etc.) without schema dependencies; no drift there.

### 5. `indexing-pipeline/indexing-pipeline-overview.md`

**Verdict:** rewritten (§5.2, §5.3 major; §8.3 minor). **ADRs applied:** 0004,
0024, 0026, 0027, 0029, 0030, 0031, 0033, 0034, 0036.

Changes:

- **§5.2 Live Processing Steps** — replaced the 11-step narrative pipeline with
  the 14-step `persist_ledger` method from `crates/indexer/src/handler/persist/mod.rs`.
  Each step now links to the ADR that motivates it. Explicit call-outs:
  Rust `stellar-xdr` parse (ADR 0004), surrogate-key resolution (ADRs 0026/0030),
  BYTEA hashes (ADR 0024), SMALLINT enums (ADR 0031), no-raw-XDR (ADR 0029),
  appearance indexes (ADRs 0033/0034), tokens→assets rename (ADR 0036).
- **§5.3 Write Target** — expanded the table inventory to match live
  migrations (added `transaction_participants`, `wasm_interface_metadata`,
  `nft_ownership`, `lp_positions`, `account_balances_current`,
  `account_balance_history`, renamed appearance tables). Added explicit note
  that the indexing pipeline itself never calls the public archive — the
  ADR 0029 read-path is a backend-only concern.
- **§8.3** — protocol-upgrade path updated: Rust `stellar-xdr` crate is the
  pinned dep, not `@stellar/stellar-sdk`.

### 6. `xdr-parsing/xdr-parsing-overview.md`

**Verdict:** rewritten (major). **ADRs applied:** 0004, 0024, 0026, 0027, 0029, 0030, 0031, 0033, 0034; collateral: 0022, 0023.

Changes:

- **§1** — removed "skeletal" disclaimer; pointed at `crates/xdr-parser/` and ADR 0032.
- **§2** — replaced "preserve raw payloads" job with "re-decode heavy-field XDR at
  request time for E3/E14 via public Stellar archive (ADR 0029)"; added note
  that ingest + read use the same shared parser crate.
- **§3.1 / §3.2** — rewrote parsing-strategy section. Used to claim
  "single parsing path — Rust at ingestion time"; now correctly describes the
  two paths (ingest + read-time archive fetch) with one shared `crates/xdr-parser`
  crate. Added a "What is not stored" subsection listing the four things
  formerly retained verbatim that are now re-derived from the archive.
- **§4 Data Extracted from XDR (all subsections)** — replaced each subsection:
  - §4.2 transactions — typed summary columns, BYTEA hash, surrogate source_id,
    removed raw-XDR retention claim
  - §4.3 operations — typed SMALLINT type, surrogate FKs, BYTEA pool_id;
    removed JSONB `details` claim
  - §4.4 soroban events — now "appearance index" (ADR 0033)
  - §4.5 NEW soroban invocations — appearance index (ADR 0034)
  - §4.6 renumbered from §4.5 — ledger entry changes, updated account balance /
    LP outputs to match current typed-column schema
- **§5 Soroban-Specific Handling** — each subsection (CAP-67 events, return
  values, invocation tree, contract interface) rewritten to the
  appearance-index + read-time decode pattern.
- **§6 Storage Contract** — rewrote §6.1 to enumerate what IS stored (typed
  columns + appearance indexes + registries) and what IS NOT (the four items
  listed in §3.2); §6.2 split materialization into two phases (ingest vs
  read-time); §6.3 advanced-view contract rewritten to source from archive,
  preserving response field names.
- **§7 Error Handling** — §7.1 / §7.2 rewritten: ingest writes partial
  typed-column rows + `parse_error = true`; read-time has its own retry budget
  against the archive. Unknown-op-type path now surfaces raw XDR from archive.
- **§8 Boundaries** — §8.1 clarified the two-path split; §8.2 updated workspace
  state to reflect `crates/xdr-parser/` implementation + ADR 0032 evergreen
  pointer.

### 7. `infrastructure/infrastructure-overview.md`

**Verdict:** minor sync. **ADRs applied:** 0004, 0027, 0029.

Changes:

- **§2.3 Event-Driven Ingestion Path** — expanded step 4 to name the
  typed-summary / appearance-index / derived-state write model and the atomic
  per-ledger transaction. Added step 5 note on the read-time archive fetch for
  E3 / E14.
- **§5.3 Processing Components** — flipped Ledger Processor parser from
  `@stellar/stellar-sdk` to the Rust `stellar-xdr` / `crates/xdr-parser` stack
  per ADR 0004; called out the 14-step `persist_ledger` + single-transaction
  write per ADR 0027.
- **§5.4 API and Delivery Components** — added the read-time archive fetch bullet
  and retained the "no Horizon / Soroban RPC / third-party indexer" negative
  statement.
- **§6.4 External Dependency Boundary** — added the public ledger archive as
  the third read-only Stellar data source, ingest-time vs read-time split
  explicit.
- **§8 Protocol upgrades** — flipped SDK bump to `stellar-xdr` Rust crate bump.

**Out-of-scope observation:** no parsed-ledger / parsed-artifact bucket is
described anywhere in the infrastructure doc, so ADR 0028 abandonment
(via ADR 0029) produced no doc drift here — nothing to remove.

### 8. `frontend/frontend-overview.md`

**Verdict:** no changes. **ADRs surveyed:** 0022–0031; **2nd-pass surveyed:** 0036, 0008.

The frontend doc does not embed DDL, table column names, or schema-shape
references. The only API-field names it mentions (`envelope_xdr`, `result_xdr`,
`result_meta_xdr` in the advanced transaction view, §6.3) are fields that still
appear in the API response — they are just populated at read time from the
public archive per ADR 0029 instead of from stored DB columns, which is
transparent to the frontend. No drift in the 0022–0031 scope. Task 0154's
rename pass had already removed any stale "tokens" references. 2nd-pass
(ADR 0036 + 0008) — nothing to add; FE uses canonical names already.

---

## 2nd pass (2026-04-24) — scope expansion + additional drift

Applied after stakeholder widened scope to full 0001-0036 range.

### ADR 0035 pre-application — `account_balance_history` removed

Coordinated removal across 6 files so migrations ↔ docs don't race when
task 0159 merges:

- **DB overview** §3 relationship sketch, §3.3 enum list, §4.18
  (replaced with drop-notice), §4.11 accounts pointer, §5.3 accounts
  FK list, §6.2 partitioning list, §7.3 raw-vs-derived
- **TD overview** ASCII diagram in §3, §6.8 accounts pointer, §6.12
  partitioning list
- **IX overview** §5.2 step 14 (no-op until chart re-scoped), §5.3
  write-target inventory
- **XD overview** §6.1 storage contract entry

### ADR 0020 drift fix — `transaction_participants`

I had introduced a spurious `role SMALLINT` column in the first pass.
Verified against migration `0003_transactions_and_operations.sql`:
real table is 3 cols `(transaction_id, account_id, created_at)` with PK
`(account_id, created_at, transaction_id)` and index `idx_tp_tx`. DB
overview §4.5 rewritten; linked to ADR 0020 for the design rationale.

### ADR 0010 drift fix — backfill is a local CLI, not Fargate

Multiple files described backfill as "ECS Fargate batch task"; ADR 0010
makes it a local `crates/backfill-bench` CLI on a developer workstation.
Updated:

- **TD overview** §3 connections list, §3 components table, §4.3
  Historical Backfill (full rewrite)
- **IN overview** §2.2 managed-runtime list (removed "backfill tasks"
  from Fargate bullet, added explicit backfill-bench bullet), §6.4
  dependency boundary
- **IX overview** §6 Historical Backfill Flow (full rewrite)
  — shared code path, not shared storage

### ADR 0033/0034/0036 formalised

Originally logged as "collateral outside scope" in the 2026-04-23 pass.
2026-04-24 scope expansion promotes them to formal in-scope. Docs
content unchanged; matrix + this worklog updated to reflect.

### ADRs 0001–0010, 0019–0021 — verified, minimal drift

- **0001** (OIDC + Secrets Manager): IN overview already references
  Secrets Manager in §5.5 and OIDC in §9.1. No change.
- **0002** (Rust Ledger Processor): applied in the 2026-04-23 pass
  (every `@stellar/stellar-sdk` flipped to Rust `stellar-xdr`).
- **0004, 0005** (Rust-only stack): already referenced in BE §5, XD §3.
- **0006** (no S3 lifecycle): IN §5.5 already states "no lifecycle
  rules on `stellar-ledger-data`". No change.
- **0007** (2-Lambda architecture): docs already describe 2 Lambdas
  (Ledger Processor + API). No Event Interpreter anywhere. No change.
- **0008** (error envelope + pagination): BE §7.3 describes cursor
  pagination. Full `ErrorEnvelope{code, message, details}` shape not
  embedded in doc but visible in `crates/api/src/openapi/schemas.rs` —
  acceptable since BE doc is narrative-level, ADR 0008 is the source
  of truth. No change.
- **0019** (schema sizing reference): capacity numbers survive in TD
  §7 estimates tables; no drift.
- **0020** (TP cut + contract index cut): applied above.
- **0021** (coverage matrix): BE §6 endpoint list matches current
  post-0029 reality (E3 + E14 do archive fetch). No separate matrix
  embedded but the individual routes + their §6 data access section
  convey the same information.
