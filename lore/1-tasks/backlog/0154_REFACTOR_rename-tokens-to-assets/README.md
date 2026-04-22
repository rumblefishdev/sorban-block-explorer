---
id: '0154'
title: 'REFACTOR: rename `tokens` → `assets` (Stellar taxonomy alignment)'
type: REFACTOR
status: backlog
related_adr: ['0022', '0023', '0027', '0032']
related_tasks: ['0118', '0120', '0124', '0135', '0155']
tags:
  [
    layer-db,
    layer-indexer,
    layer-api,
    layer-docs,
    schema,
    refactor,
    naming,
    priority-medium,
    effort-medium,
  ]
links:
  - notes/R-assets-vs-tokens-taxonomy.md
  - crates/db/migrations/0005_tokens_nfts.sql
  - crates/db/migrations/0002_identity_and_ledgers.sql
  - crates/domain/src/token.rs
  - crates/xdr-parser/src/types.rs
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/classification.rs
  - crates/indexer/src/handler/persist/mod.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/src/handler/persist/staging.rs
  - crates/indexer/src/handler/process.rs
  - crates/indexer/tests/persist_integration.rs
  - docs/architecture/technical-design-general-overview.md
  - docs/architecture/database-schema/database-schema-overview.md
  - docs/architecture/backend/backend-overview.md
  - docs/architecture/frontend/frontend-overview.md
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
history:
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from research note `notes/R-assets-vs-tokens-taxonomy.md`.
      Stellar official taxonomy (Anatomy of an Asset) treats "Stellar Assets"
      and "Contract Tokens" as two distinct categories, not synonyms. Our
      `tokens` table actually holds both (`native`, `classic`, `sac`,
      `soroban`) — it is de facto an `assets` table carrying a legacy name
      from the Soroban-first iteration. Additionally, the word "token"
      collides with `soroban_contracts.contract_type = 'token'` which
      classifies contract role. Rename eliminates the collision and aligns
      with official Stellar vocabulary: *contract is a token (role),
      represents an asset (value)*.
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Scope tightened and expanded after review. Label remap reduced to
      `classic` → `classic_credit` only (XDR-accurate disambiguation vs
      `native`); dropped the speculative `soroban` → `soroban_sep41`
      rename — T-REX / SEP-57 handling can be addressed by a dedicated
      ADR + additive value or a `compliance_flavour` column if/when the
      ecosystem materialises. Per-file scope for `docs/architecture/**`
      added after reading every overview doc; `infrastructure-overview`
      confirmed clean (no table / schema references). Related ADR 0032
      and task 0155 (docs evergreen policy + catch-up sweep) linked.
---

# REFACTOR: rename `tokens` → `assets` (Stellar taxonomy alignment)

## Summary

Rename the `tokens` table to `assets`, rename the `asset_type` label
`classic` → `classic_credit` (XDR-accurate disambiguation vs `native`),
and propagate the rename through the Rust domain, xdr-parser, indexer
persist path, integration tests, API surface, and every affected file
under `docs/architecture/**`. The schema itself does not change (same
partial unique indexes, same `ck_tokens_identity` / `ck_assets_identity`,
same FK to `soroban_contracts`) — only the name. The collision between
"token" as a table name and "token" as a SEP-41 contract role goes away.

## Context

Full motivation: [notes/R-assets-vs-tokens-taxonomy.md](notes/R-assets-vs-tokens-taxonomy.md).

Key points:

- Stellar official docs split tokenization into **Stellar Assets**
  (classic, G-issued, trustline-held) and **Contract Tokens** (SEP-41,
  C-address, contract-data-held). Not synonyms.
- Our table `tokens` already carries `native` + `classic` rows, i.e.
  rows that have no contract at all and no SEP-41 surface — a classic
  "asset" in Stellar-speak.
- `soroban_contracts.contract_type = 'token'` means "this contract
  implements SEP-41 Token Interface" — a role classification. Clashing
  with the table name is a real source of team ambiguity.
- The design overview already _explicitly_ expands the word in
  parentheses ("List of all known tokens (classic Stellar assets and
  Soroban token contracts)") — the name is known-too-narrow.

Related work:

- ADR 0032 / task 0155 — `docs/architecture/**` becomes evergreen; the
  doc changes here live in that same spirit.
- Tasks 0118 / 0120 / 0124 / 0135 — all touch the tokens-table surface.
  Coordinate sequencing so this rename does not collide mid-stream.

## Scope

### In scope — DB schema

One new reversible migration pair:

- `ALTER TABLE tokens RENAME TO assets` (FKs auto-follow).
- Rename constraints: `ck_tokens_asset_type` / `ck_tokens_identity` →
  `ck_assets_asset_type` / `ck_assets_identity`.
- Rename indexes: `uidx_tokens_native`, `uidx_tokens_classic_asset`,
  `uidx_tokens_soroban`, `idx_tokens_type`, `idx_tokens_code_trgm` →
  `uidx_assets_*`, `idx_assets_*`.
- Remap `asset_type`: `classic` → `classic_credit` (`native`, `sac`,
  `soroban` unchanged). Update `ck_assets_identity` branches
  accordingly.
- Historical migration filename `0005_tokens_nfts.sql` stays; new
  migration only ships the rename + label remap per MIGRATIONS.md
  reversible pair.
- Coordination with ADR 0031: if this task lands post-0031 (enum
  columns → SMALLINT), the remap lives in the Rust `TokenAssetType`
  enum module + the `token_asset_type_name` SQL helper, not in a
  VARCHAR `CHECK`. Update the integration test that iterates every
  enum variant.

### In scope — Rust code

- `crates/domain/src/token.rs` → `asset.rs`; `struct Token` → `Asset`.
- `crates/xdr-parser/src/types.rs` — `ExtractedToken` → `ExtractedAsset`.
- `crates/xdr-parser/src/state.rs` — `detect_tokens` → `detect_assets`.
- `crates/xdr-parser/src/classification.rs` — rename
  `TokenClassification` / `classify_token` if present (audit during
  implementation).
- `crates/indexer/src/handler/persist/staging.rs` — `TokenRow` →
  `AssetRow`, field `token_rows` → `asset_rows`.
- `crates/indexer/src/handler/persist/write.rs:743-910` —
  `upsert_tokens*` functions → `upsert_assets*`.
- `crates/indexer/src/handler/persist/mod.rs` — pipeline parameter
  name, step comment, `StepTimings.tokens_ms` → `assets_ms` (breaks
  Grafana / CloudWatch dashboards — see risks).
- `crates/indexer/src/handler/process.rs:127` — call site.
- `crates/indexer/tests/persist_integration.rs` — helpers
  `make_sac_token()` → `make_sac_asset()`, imports.

### In scope — API surface

- `GET /tokens` → `/assets`.
- `GET /tokens/:id` → `/assets/:id`.
- `GET /tokens/:id/transactions` → `/assets/:id/transactions`.
- `GET /search?type=token,...` — decide whether "token" stays as a
  search kind or becomes "asset". Preferred: both aliased server-side
  for one release, then drop "token".
- OpenAPI spec + generated client types regenerated.

### In scope — `docs/architecture/**`

Every file below was inspected; only the ones listed have real edits.
`infrastructure-overview` confirmed clean (no table / schema / route
references that collide with the rename).

#### `technical-design-general-overview.md`

From research note §9.3 (line numbers approximate; verify at
implementation):

- lines 46, 58-59, 85-86 — route tables listing `/tokens`, `/tokens/:id`.
- lines 158, 161, 163, 165, 170, 172, 174, 177 — "Tokens page", "Token
  detail", "trustline/token balances" wording.
- line 280 — ASCII diagram of backend Lambda `Tokens` module.
- lines 368-376 — `/tokens` API endpoints section.
- line 370 — "Paginated list of tokens (classic assets + Soroban token
  contracts)".
- line 414 — search `type=…,token,…`.
- line 470 — ASCII diagram of RDS listing the `tokens` table.
- line 739 — "Derived-state upserts (`accounts`, `tokens`, `nfts`,
  `liquidity_pools`)".
- lines 948, 951 — §6.7 header and `CREATE TABLE tokens` block. Drop
  the §6.7 drift between doc and real schema here — the broader sweep
  is ADR 0032 catch-up (task 0155).
- lines 1072, 1086, 1110-1111, 1208 — estimate tables / deliverables.

#### `database-schema/database-schema-overview.md`

- §3 Schema Shape Overview (lines 82-98) — entity list + high-level
  diagram: `tokens` → `assets`.
- **§4.7 Tokens (lines 297-326) — rewrite whole section**: rename to
  "Assets", update `asset_type` CHECK to 4 values (add `native`,
  `sac` — this doc still shows 3) in line with current reality,
  document partial unique indexes, document `ck_assets_identity`.
  Explain that the table covers both Stellar Assets and Contract
  Tokens per official taxonomy. Label the rename as originating from
  this task.
- §5.1 Ingestion Flow (line 461) — "tokens" in derived-entities list;
  rename.
- Cross-link to `soroban_contracts.contract_type = 'token'` in §4.4
  to surface that the role label intentionally stays.

#### `backend/backend-overview.md`

- §3.1 Runtime diagram (line 86) — `├─ Tokens ──────────` → `Assets`.
- §5.1 Primary Modules (line 178) — "`Tokens` - classic asset and
  Soroban token listing" → "`Assets` - classic and Soroban-native
  asset listing and detail retrieval".
- §6.2 Endpoint Inventory (line 211) — rename `/tokens*` row to
  `/assets*`; update `filter[type]` values from
  `(classic/sac/soroban)` to `(native/classic_credit/sac/soroban)`.
- §6.2 (line 215) — search type enum `type=…,token,…` — alias or
  replace per the ADR.
- **§6.3 Tokens subsection (lines 290-302) — rewrite**: route names,
  filter values, description ("The backend must preserve the
  distinction between classic assets and contract-based tokens while
  still serving both through a unified explorer API") updated to
  "…preserve the distinction between native, classic credit, SAC,
  and Soroban-native assets…".

#### `frontend/frontend-overview.md`

- §4.1 runtime diagram (lines 132-134) — `/tokens*` routes. UX
  decision whether the browser route also renames to `/assets*` —
  recommended yes for consistency with API, with a 301 redirect from
  `/tokens*`.
- §6.1 route inventory (lines 247-248) — rows for `/tokens` and
  `/tokens/:id`.
- **§6.7 Account page (line 375)** — "Balances - native XLM balance
  and trustline/token balances" is an active example of the
  imprecise wording the research note §5 flags. Rewrite to "native
  XLM balance and credit asset balances" (or the more verbose and
  more accurate "native, classic credit, SAC, and Soroban-native
  asset balances" — pick per editorial).
- **§6.8 Tokens (lines 386-402) — rename to "Assets"**, rewrite
  filters `type (classic, SAC, Soroban)` →
  `type (native, classic credit, SAC, Soroban)`.
- **§6.9 Token (lines 403-418) — rename to "Asset"**, update type
  badge copy.
- §7 Shared UI Elements (line 525) — "Navigation - links to home,
  transactions, ledgers, tokens, contracts…" — `tokens` → `assets`.
  Nav label copy decision (keep "Tokens" for familiarity vs. switch
  to "Assets" for accuracy) — recommended "Assets" to match API and
  documentation.

#### `indexing-pipeline/indexing-pipeline-overview.md`

Nomenclature cleanup only; no structural changes.

- §2 Architectural Role (line 55) — "higher-level explorer entities
  such as contracts, accounts, tokens, NFTs…" → `assets`.
- §5.2 Live Processing Steps (line 161) — "detect token contracts,
  NFT contracts, and liquidity pools" — keep "token contracts" here
  because that phrase describes SEP-41 contract _role_ detection,
  not the table. Acceptable to clarify: "detect SEP-41 token
  contracts (→ `assets`), NFT contracts, liquidity pools".
- §5.3 Write Target (line 172) — "derived explorer-facing state
  such as accounts, tokens, NFTs, and liquidity pools" → `assets`.

#### `xdr-parsing/xdr-parsing-overview.md`

Nomenclature cleanup only.

- §3.4 Frontend Parsing Boundary (line 98) — "account, token, NFT,
  and pool views" → `account, asset, NFT, and pool`.
- §6.2 Ingestion Owns Materialization (line 251) — "derived account,
  token, NFT, and liquidity-pool state" → `asset`.

#### `infrastructure/infrastructure-overview.md`

**No changes.** Reviewed whole file — it covers VPC, RDS, Lambda,
ECS Fargate, Secrets Manager, observability. No references to the
`tokens` table, the `/tokens` routes, or to asset taxonomy. Clean.

### Out of scope

- ADR-level documents (0022, 0023, 0027): not renamed. ADRs are
  historical records; a new ADR captures the decision and context
  shift. See research note §9.5.
- `soroban_contracts.contract_type = 'token'`: stays. After table
  rename the collision disappears and the role label is precise.
- `nfts` table rename: not needed — it holds instances, no ambiguity.
- `asset_code VARCHAR(12)` deduplication / dictionary: separate
  concern (ADR candidate in 0031 follow-ups).
- XLM-SAC linkage gap (research note §6.6): separate bug / ADR,
  unrelated to the rename.
- **`soroban` → `soroban_sep41` label rename**: dropped from scope
  as speculative (see "On the `asset_type` label remap" below).

## On the `asset_type` label remap

Only one label is being renamed: `classic` → `classic_credit`. The
initial research-note draft also proposed `soroban` → `soroban_sep41`,
but on review that's YAGNI and dropped.

**`classic` → `classic_credit`** — kept. Justification:

- Stellar XDR calls these `CREDIT_ALPHANUM4` / `CREDIT_ALPHANUM12`
  (see `AssetType` enum in stellar-xdr, referenced by ADR 0031).
  "Classic credit" mirrors the protocol term exactly.
- Without the rename, the word "classic" is ambiguous: XLM is also
  "classic" in the sense of being a classic-ledger asset, yet we
  label it `native` because that's the XDR variant. The only reason
  `classic` works today is because `ck_tokens_identity` disambiguates
  at CHECK-constraint level — not at label level. A reader skimming
  the table sees `native` and `classic` side by side and has to read
  the constraint to understand they partition correctly.
- `classic_credit` removes that cognitive step.

**`soroban` → `soroban_sep41`** — dropped. Justification:

- The only reason given in the research note (§7.3) is "leaves space
  for `soroban_trex`". But §4.2 of the same note says "ekosystem T-REX
  na Stellarze nascent" — we'd be adding precision for an ecosystem
  that doesn't exist on Stellar mainnet yet.
- If T-REX (SEP-57) ever lands, its on-chain identity is still a
  `C...` SEP-41 contract plus compliance extensions. The more natural
  modelling would be a boolean `is_compliance` or a separate
  `compliance_flavour` column — not splitting `asset_type` into
  sibling labels.
- Until then, T-REX tokens would classify as `soroban` with no loss
  of fidelity. Renaming pre-emptively costs a DB migration and a
  label-breaking change for no user today.
- Additive: if T-REX materialises, a follow-up ADR can add a
  `soroban_trex` (or equivalent) value without renaming the existing
  `soroban`. That's the symmetric cost at the point we actually need
  the distinction.

Net: the rename stays mechanical and low-risk. One label change, all
other values preserved.

## Implementation Plan

1. **Draft the ADR documenting the decision.** Capture reasoning
   (incl. the `classic_credit`-only label change), coordination with
   ADR 0031 (SMALLINT enums) and ADR 0032 (docs evergreen), and the
   migration strategy (single transaction, reversible pair per
   MIGRATIONS.md).
2. **Write reversible migration pair** — up renames table,
   constraints, indexes, remaps `classic` → `classic_credit`; down
   reverses all of it. Run against a restored staging dump to confirm
   FK and index behavior.
3. **Rust rename pass** — start from `domain/src/token.rs`, follow
   the compiler through xdr-parser, staging, write, process, tests.
   Expect ~15–20 touched files. `cargo check` as guard, `cargo clippy
--all-targets -- -D warnings` before commit.
4. **Axum API rename** — new routes, keep `/tokens*` aliases for one
   release emitting a `Deprecation` header; drop aliases in a follow-up.
5. **OpenAPI regen + frontend sync** — coordinate with frontend lead.
6. **docs/architecture sync** — walk every file listed in the "In
   scope — `docs/architecture/**`" section above and apply the edits
   called out there. If task 0155 (ADR 0032 catch-up sweep) has
   already landed, this step shrinks to rename-only deltas; otherwise
   the rename lands alongside rename-local doc updates and 0155 picks
   up the broader sweep.
7. **Metrics/dashboards** — rename `tokens_ms` → `assets_ms`, notify
   ops channel before merge.
8. **Bench 100-ledger partition** — confirm no regression on
   `upsert_assets*` vs baseline `upsert_tokens*`.

## Acceptance Criteria

- [ ] ADR drafted and `accepted`, referenced from this task's
      `related_adr`.
- [ ] Reversible migration pair lands (up + down), tested both
      directions on a dump of staging.
- [ ] `ALTER TABLE tokens RENAME TO assets` + constraint/index
      renames + `classic` → `classic_credit` remap execute in one
      transaction.
- [ ] Post-migration: partial unique indexes (`uidx_assets_native`,
      `uidx_assets_classic_asset`, `uidx_assets_soroban`) behave
      identically to pre-migration equivalents (integration test
      with the four `asset_type` row patterns).
- [ ] `ck_assets_identity` enforces the same invariants as
      `ck_tokens_identity` did, adjusted for the `classic_credit`
      rename.
- [ ] All Rust call sites renamed; `SQLX_OFFLINE=true cargo build
    --workspace` + `cargo clippy --all-targets -- -D warnings` green.
- [ ] `cargo test -p indexer persist_integration` green.
- [ ] Axum routes: `/assets*` live; `/tokens*` either aliased with
      deprecation header or dropped depending on API contract
      versioning decision in the ADR.
- [ ] OpenAPI regenerated; frontend client types updated or aliased.
- [ ] Each `docs/architecture/**` file listed in Scope updated per
      its bullet list; `infrastructure-overview.md` unchanged and
      explicitly noted as verified-clean in the PR description.
- [ ] Grafana/CloudWatch dashboards referencing `tokens_ms` updated.
- [ ] 100-ledger backfill bench (p95 timing) within ±5 % of
      pre-rename baseline.

## Risks

- **Public API breakage** if the API is already live to external
  consumers. Mitigation: aliases + deprecation window, document in
  ADR.
- **Dashboard silent breakage** from metric rename. Mitigation: land
  dashboard PR the same day, announce in ops channel.
- **Drift window** between this rename and tasks 0118 / 0120 /
  0124 / 0135 (all touch the same table). Mitigation: sequence —
  land this as a low-risk refactor when no other tokens-table task
  is mid-stream, or coordinate explicitly in the ADR.
- **Frontend/end-to-end lag** if API route rename lands before
  frontend updates. Mitigation: alias window.
- **Overlap with task 0155** on `docs/architecture/**` file edits —
  coordinate which task opens first; both PRs should not be open
  against the same file simultaneously.

## Notes

- Research note §9 has the full file-by-file inventory used to
  generate the per-file scope above (1–2 developer days excluding
  API versioning decision).
- If ADR 0031 lands before this task is picked up, the enum remap
  happens in `crates/domain/src/enums/` (the `TokenAssetType` — or
  renamed `AssetClass` — Rust enum) and in the `token_asset_type_name`
  SQL helper. See ADR 0031 §3.
- Keep the rename mechanical: no opportunistic schema changes, no
  opportunistic endpoint reshaping. A clean rename commit is easier
  to review and easier to revert.
