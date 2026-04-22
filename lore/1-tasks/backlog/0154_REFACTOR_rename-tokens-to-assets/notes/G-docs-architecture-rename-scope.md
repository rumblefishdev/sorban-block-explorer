---
title: 'docs/architecture/** rename scope — per-file edits'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: [docs, rename, scope]
links:
  - ../../../../docs/architecture/technical-design-general-overview.md
  - ../../../../docs/architecture/database-schema/database-schema-overview.md
  - ../../../../docs/architecture/backend/backend-overview.md
  - ../../../../docs/architecture/frontend/frontend-overview.md
  - ../../../../docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - ../../../../docs/architecture/xdr-parsing/xdr-parsing-overview.md
  - ../../../../docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: '2026-04-22'
    status: developing
    who: stkrolikiewicz
    note: >
      Extracted from task README during review cleanup. Holds the
      per-file rename scope across `docs/architecture/**` so the README
      stays focused on summary + acceptance criteria per
      `lore/1-tasks/CLAUDE.md` convention (~50-100 lines).
---

# `docs/architecture/**` rename scope — per-file edits

Every file below was inspected against `docs/architecture/`; only the
ones listed have real edits. `infrastructure-overview` is confirmed
clean (no table / schema / route references that collide with the
rename).

Line numbers are approximate — verify at implementation time, since
they drift as the ADR 0032 / task 0155 evergreen sweep lands updates.

## `technical-design-general-overview.md`

From [`R-assets-vs-tokens-taxonomy.md`](R-assets-vs-tokens-taxonomy.md) §9.3:

- lines 46, 58-59, 85-86 — route tables listing `/tokens`, `/tokens/:id`.
- lines 158, 161, 163, 165, 170, 172, 174, 177 — "Tokens page",
  "Token detail", "trustline/token balances" wording.
- line 280 — ASCII diagram of backend Lambda `Tokens` module.
- lines 368-376 — `/tokens` API endpoints section.
- line 370 — "Paginated list of tokens (classic assets + Soroban
  token contracts)".
- line 414 — search `type=…,token,…`.
- line 470 — ASCII diagram of RDS listing the `tokens` table.
- line 739 — "Derived-state upserts (`accounts`, `tokens`, `nfts`,
  `liquidity_pools`)".
- lines 948, 951 — §6.7 header and `CREATE TABLE tokens` block.
  Drop the §6.7 drift between doc and real schema here — the broader
  sweep is ADR 0032 catch-up (task 0155).
- lines 1072, 1086, 1110-1111, 1208 — estimate tables / deliverables.

## `database-schema/database-schema-overview.md`

- §3 Schema Shape Overview (lines 82-98) — entity list + high-level
  diagram: `tokens` → `assets`.
- **§4.7 Tokens (lines 297-326) — rewrite whole section**: rename to
  "Assets", update `asset_type` CHECK to 4 values (add `native`,
  `sac` — this doc still shows 3) in line with current reality,
  document partial unique indexes, document `ck_assets_identity`.
  Explain that the table covers both Stellar Assets and Contract
  Tokens per the official taxonomy. Label the rename as originating
  from this task.
- §5.1 Ingestion Flow (line 461) — "tokens" in derived-entities list;
  rename.
- Cross-link to `soroban_contracts.contract_type = 'token'` in §4.4
  to surface that the role label intentionally stays.

## `backend/backend-overview.md`

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

## `frontend/frontend-overview.md`

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

## `indexing-pipeline/indexing-pipeline-overview.md`

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

## `xdr-parsing/xdr-parsing-overview.md`

Nomenclature cleanup only.

- §3.4 Frontend Parsing Boundary (line 98) — "account, token, NFT,
  and pool views" → `account, asset, NFT, and pool`.
- §6.2 Ingestion Owns Materialization (line 251) — "derived account,
  token, NFT, and liquidity-pool state" → `asset`.

## `infrastructure/infrastructure-overview.md`

**No changes.** Reviewed whole file — it covers VPC, RDS, Lambda,
ECS Fargate, Secrets Manager, observability. No references to the
`tokens` table, the `/tokens` routes, or to asset taxonomy. Clean.
