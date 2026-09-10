# Stellar Block Explorer - Backend Overview

> This document expands the backend portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same API scope and operational assumptions, but specifies the backend
> architecture in more detail so it can later serve as input for implementation task
> planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Architectural Role](#2-architectural-role)
3. [Runtime Architecture](#3-runtime-architecture)
4. [Responsibilities and Boundaries](#4-responsibilities-and-boundaries)
5. [Module Design](#5-module-design)
6. [API Surface](#6-api-surface)
7. [Data Access and Response Model](#7-data-access-and-response-model)
8. [Caching and Performance](#8-caching-and-performance)
9. [Fault Tolerance and Operational Notes](#9-fault-tolerance-and-operational-notes)
10. [Workspace Placement and Delivery Notes](#10-workspace-placement-and-delivery-notes)

---

## 1. Purpose and Scope

The backend is the public server-side API layer of the Stellar Block Explorer. Its role is
to expose explorer data through a stable REST interface that the frontend can consume
without understanding ingestion internals, raw ledger storage layout, or XDR parsing flows.

This document covers the target design of the backend application only. It does not define
infrastructure provisioning, ingestion implementation, or the full database schema beyond
what is needed to explain backend behavior.

The target workspace structure (per ADR 0005, tasks 0094/0095) reserves the backend boundary as:

- `crates/api` - application entrypoint for the public REST API (Rust/axum)
- `crates/domain` - shared explorer-domain types used by backend crates

This document describes the intended production architecture for that boundary. It is not
a description of the current implementation state, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
general overview document takes precedence. This file is a backend-focused refinement of
that source, not an independent redesign.

## 2. Architectural Role

The backend sits between the public clients and the block explorer's own ClickHouse
store. It is the only supported read interface for explorer consumers.

Its job is to make indexed chain data usable:

- hide ingestion and storage details behind stable REST resources
- normalize raw indexed records into frontend-friendly responses
- provide unified search and consistent pagination semantics
- expose raw XDR only where advanced inspection genuinely requires it

The backend is intentionally not a chain-ingestion component and not an external-chain
proxy.

## 3. Runtime Architecture

### 3.1 Runtime Model

The backend is a Rust application (axum) running on AWS Lambda behind API Gateway. It is a
REST API. The backend does not perform chain indexing; it reads from the block explorer's
own ClickHouse store, which is populated by the Galexie-based ingestion pipeline.

The public explorer API serves anonymous read traffic. Browser clients do not carry API
keys; abuse controls are enforced at the ingress layer through throttling, request
validation, and the Cloudflare edge that fronts the API hostname. There is no AWS WAF —
both WebACLs were dropped ([ADR 0048](../../../lore/2-adrs/0048_cloudflare-edge-over-aws-waf.md),
task 0302). If API keys are introduced, they are reserved for trusted non-browser
consumers.

```
┌──────────┐ HTTPS ┌────────────┐ +X-Edge-Secret ┌─────────────┐  ┌──────────────────────┐
│  Client  │──────>│ Cloudflare │───────────────>│ API Gateway │─>│  Lambda (Rust/axum)  │
└──────────┘       │ WAF · DDoS │                └─────────────┘  │                      │
                   │ rate limit │                                 │  axum Modules:       │
                   └────────────┘                                 │  ├─ Network ─────────┤
                                                                  │  ├─ Transactions ────┤
                                                                  │  ├─ Ledgers ─────────┤
                                                                  │  ├─ Accounts ────────┤
                                                                  │  ├─ Assets ──────────┤
                                                                  │  ├─ Contracts ───────┤
                                                                  │  ├─ NFTs ────────────┤
                                                                  │  ├─ Liquidity Pools ─┤
                                                                  │  └─ Search ──────────┤
                                                                  └──────────┬───────────┘
                                                                             │
                                                                             ▼
                                                                  ┌──────────────────────┐
                                                                  │  ClickHouse (Hetzner)│
                                                                  │  (block explorer DB) │
                                                                  └──────────────────────┘
```

### 3.2 Request Flow

The typical request path is:

1. client calls a public REST endpoint on the Cloudflare-fronted hostname; Cloudflare
   applies its managed rules, DDoS and rate limiting, and a Transform Rule stamps
   `X-Edge-Secret` before forwarding to API Gateway
2. API Gateway routes the request to the Rust/axum Lambda handler, which rejects
   anything arriving without that header (`crates/api/src/common/edge_lock.rs`)
3. the relevant module validates input and queries the explorer database
4. backend-level normalization and enrichment are applied where needed
5. the response is returned in a frontend-friendly form

### 3.3 Technology Direction

The backend implementation direction implied by the current design is:

- **axum** for modular API composition and transport-layer structure (per ADR 0005)
- **Rust** for typed application code with compile-time safety
- **ClickHouse** (via the `clickhouse` crate) as the read store (per ADR 0044 / 0047)
- **utoipa** for OpenAPI spec generation (per ADR 0005). The spec is the single
  source of truth for API contracts and is consumed by the frontend via the
  `libs/api-types` codegen pipeline (task 0096). A secondary `extract_openapi`
  binary in the `api` crate dumps the spec at build time, so codegen does not
  require booting the Lambda. The spec also declares the access-layer security
  schemes (task 0277/0287) — `api_key` (`x-api-key` header, paid tier) and
  `bearer_jwt` (free-tier session JWT) — as a global OR requirement, so Swagger
  UI renders an "Authorize" dialog and "Try it out" can reach the gated `/v1`
  surface; `/health` opts out with an empty per-path requirement.
- **AWS Lambda** for serverless compute and on-demand scaling (via cargo-lambda)
- **API Gateway** for public HTTP ingress, throttling, request validation, and response
  caching
- **Cloudflare** for managed-rule abuse protection, DDoS and rate limiting on the API
  hostname, with the AWS origin locked to it by a secret request header
  (`crates/api/src/common/edge_lock.rs`) — replacing the AWS WAF WebACL that used to sit
  on the API Gateway stage
- **ClickHouse** as the only source of indexed chain data served by the API
- **No XDR dependencies** — API serves pre-materialized data; raw XDR is passthrough only (per ADR 0004)

This document assumes the backend follows the implementation direction already
reflected in the general overview, including axum and utoipa (per ADR 0005), while keeping the API
behavior here as the primary contract to preserve.

## 4. Responsibilities and Boundaries

### 4.1 Responsibilities

The backend serves data from the block explorer's own database, adding:

- **Data normalization** - transforms raw indexed records into a consistent,
  frontend-friendly format (e.g. flattening nested fields, attaching human-readable
  operation summaries)
- **Soroban enrichment** - decorates contract invocations with metadata and function names
  stored at ingestion time
- **Search** - unified search across transaction hashes, account IDs, contract IDs, token
  identifiers, NFT identifiers, pool IDs, and indexed metadata via ClickHouse
  classification-gated per-bucket lookups (task 0318)
- **Runtime details enrichment** — per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md),
  the backend resolves enrichable detail fields at request time rather than
  persisting them. Two transport-specific submodules under
  `crates/api/src/runtime_enrichment/` share the architectural shape
  (per-request, fail-soft, in-process LRU-cached). Status surfacing is
  per-submodule: archive-backed endpoints expose a `heavy_fields_status`
  discriminator (`ok` / `unavailable`); SEP-1 enrichment surfaces failures
  silently as `null` description / home_page (warn-logged) and adds no
  status field today:
  - **`runtime_enrichment::stellar_archive`** — fetches `.xdr.zst` ledger files
    from the public Stellar archive on S3, decompresses with `crates/xdr-parser`
    and merges decoded payload into responses. Drives E3 `/transactions/:hash`
    (full envelope + parsed invocation tree, per
    [ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md) /
    [ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md))
    and E14 `/contracts/:id/events` (full event detail). List endpoints never
    call the archive and answer from typed summary columns + appearance indexes only.
    The S3 GET is **cross-region** (the API Lambda runs in eu-central-1; the
    public archive bucket is in us-east-2), which dominates E3 latency (~2–3 s).
    For E3 the zstd-decompress + XDR parse runs on `spawn_blocking` and overlaps
    the DB ops query (`tokio::join!`); a per-Lambda in-process cache of the heavy
    block was trialled (task 0330) but **removed** after it showed a 0% hit rate
    in production (request scatter across the Lambda fleet). The remaining
    server-side latency lever is edge caching (the response is already
    `public, max-age=300`).
  - **`runtime_enrichment::sep1`** — issues HTTPS GETs to
    `https://{issuer.home_domain}/.well-known/stellar.toml`, parses the SEP-1
    schema, and merges `[[CURRENCIES]]` per-token fields plus
    `[DOCUMENTATION]` org info into asset detail responses (task 0188).
    Built-in safeguards: 100 KB body cap (per SEP-1 spec), 1 s connect / 2 s
    request timeouts, RFC 1035 hostname validation rejecting IP literals, and
    a 24 h LRU cache (1024 entries) keyed by lowercase home_domain. Currently
    consumed only by `GET /v1/assets/{id}`; future detail endpoints
    (accounts, etc.) will reuse the same fetcher
  - **`runtime_enrichment::nft_token_uri`** — drives the detail-only
    `metadata` field on `GET /v1/nfts/:id` (task 0195 §2d). Per ADR 0043
    detail-only carve-out — the `nfts.metadata` JSONB column was dropped
    in migration `20260507120000_drop_nfts_metadata.up.sql`. Flow:
    Soroban RPC `simulateTransaction` of `token_uri(token_id)`
    (SEP-50 per-token form, falls back to zero-arg `token_uri()` form
    for SEP-39 contracts on `MismatchingParameterLen` — see audit 0197
    Bug #5), then IPFS gateway fetch + JSON parse. Built-in safeguards:
    3 s wall-clock timeout, 256 KB body cap, scheme/hostname validation
    (https / ipfs only), 24 h LRU (1024 entries). Fail-soft NULL on any
    error class. Code is shared with the Lambda 2 write-side worker
    (`crates/enrichment-shared::nft_token_uri`); only the persistence
    half differs (handler returns the JSON inline, worker writes
    `nfts.name` / `media_url` / `collection_name`).
- **Surrogate-key resolution** — every StrKey that enters a route parameter
  (`G...`, `C...`) is resolved to the `BIGINT` surrogate via the relevant
  `UNIQUE` index at the request boundary
  ([ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md));
  every StrKey in a response comes from a join back to `accounts.account_id`
  or `soroban_contracts.contract_id`. The public API shape is unchanged
- **ClickHouse read store (tasks 0243 / 0244)** — every read handler is served
  from ClickHouse ([ADR 0047](../../../lore/2-adrs/0047_clickhouse-primary-api-datastore.md));
  the per-module PG↔CH `DataSource` dispatch was removed with Postgres. Each
  module's `queries.rs` maps `clickhouse::Row` results to the same `domain::*` /
  DTO types, so the public response shape is store-agnostic.
  The `assets` table on ClickHouse has **no numeric surrogate** — it is keyed on
  the natural identity 4-tuple `(asset_type, asset_code, issuer_id, contract_id)`,
  which is why `/assets/:id` and the list cursor use the composite token /
  composite keyset rather than the dropped `assets.id`.
  **SAC is a facet, not a type** ([ADR 0051](../../../lore/2-adrs/0051_sac-as-facet-of-classic-credit.md),
  task 0339): `asset_type_name` / `filter[type]` no longer carry `sac`; the asset
  DTO surfaces `sac_contract_id` (the SAC's `C…`, **re-derived on read** from
  `code:issuer` via `derive_sac_strkey` — never stored) + `sac_deployed`, both
  read from the indexer-owned `asset_sac` side table (`AggregatingMergeTree`,
  `max`-merged) LEFT-JOINed at read — NOT columns on `assets` (which is
  re-written whole every ledger and would clobber them). The canonical `id` of a
  SAC-wrap is its `CODE-ISSUER`, and the "SAC" view is the property filter
  `filter[sac]=true` (`sac.sac_deployed` — deployed SACs only; reserved
  un-deployed addresses are excluded). `/assets/{C…}` deep-links
  resolve either a soroban contract OR a SAC — `fetch_by_contract_id` hashes the
  input `C…` to its surrogate and matches it against the (small, whole-table
  aggregated) `asset_sac` join.
  The `nfts` table on ClickHouse is likewise **surrogate-free** (keyed on
  `(contract_id, token_id)`): the wire `NftItem.id` is dropped, the list cursor
  keys on `(minted_at_ledger, contract_id, token_id)`, and the transfers
  timeline keys on `(contract_id, token_id)` directly (no `nft_id`). NFT
  `name` / `media_url` / `collection_name` are read from the `nft_enrichment`
  side table (`argMax(_, version)`), since the indexer-owned `nfts.*` copies are
  vestigial NULL on CH; full enrichment coverage is a prod-flip prerequisite,
  the same gate as `assets` ↔ task 0231.

### 4.2 What the Backend Must Not Do

The backend does **not**:

- perform live chain indexing
- call Horizon or any private chain API
- rely on a third-party explorer database

Backend dependencies at runtime: (1) the explorer's own ClickHouse store for every
partition-pruned read, (2) the public Stellar ledger archive for read-time
XDR expansion on E3 / E14.

### 4.3 Boundary with Other Applications

Responsibility split across the workspace should remain clear:

- `crates/indexer` and related workers own ingestion and persistence into the explorer DB
- `crates/api` owns query APIs, response shaping, search, and transport concerns
- `web` consumes the REST API and should not reconstruct backend behavior client-side
- `crates/domain` holds reusable explorer-domain types shared across backend crates

## 5. Module Design

The backend is best structured as resource-oriented axum route modules matching the public API
surface.

### 5.1 Primary Modules

- `Network` - chain-level aggregate stats and freshness information
- `Transactions` - list and detail queries, filter handling, advanced/raw payload support
- `Ledgers` - ledger list/detail access and linked transaction retrieval
- `Accounts` - account summary, balances, and account-related transaction history
- `Assets` - classic and Soroban-native asset listing and detail retrieval
- `Contracts` - contract metadata, interface, invocations, and events
- `NFTs` - NFT list/detail retrieval and transfer history access
- `Liquidity Pools` - pool listing, detail, transaction history, and chart data
- `Search` - exact match and grouped result resolution across entity types

### 5.2 Cross-Cutting Backend Concerns

In addition to resource modules, the backend will need shared internal capabilities:

- request validation and query parsing
- cursor-based pagination helpers
- response serialization and error mapping
- search-query classification and exact-match resolution
- raw XDR passthrough for advanced transaction sections (no server-side decode)
- caching and freshness metadata

These are backend concerns even when their outputs are consumed by frontend pages.

## 6. API Surface

### 6.1 Base URL

**Base URL:** `https://api.soroban-explorer.com/v1`

### 6.2 Identifier form — StrKey only

**Every entity identifier on the wire is its SEP-23 StrKey, and only that.** Hex
is not accepted as input and never appears in a response: transactions use the
64-char hash, accounts `G…`, contracts `C…`, liquidity pools `L…`, and an NFT is
the composite `(contract StrKey, token_id)`.

Decided in task 0264 ("strkey canonical everywhere — strkey-only, no legacy hex
compat"): the project was pre-deploy, so there were no hex bookmarks worth a
compatibility path, and the Stellar ecosystem addresses pools by StrKey (CAP-38
/ SEP-23). `common::path::pool_id_strkey` enforces it on the detail route and
says so in its 400 body — _"hex form is no longer accepted"_. Internal storage
stays hex (`pool_id` is 32 raw bytes); the conversion happens at the boundary,
never in the contract.

**Two known exceptions, both recorded as debt rather than precedent:**

- `/v1/search` still classifies a 64-hex string as a pool or transaction id.
  0264 deferred the search endpoint explicitly (its Phases 3, 9 and 10) — it is
  unfinished work, not a second standard. Do not copy it.
- Cursors are opaque base64 payloads and carry internal surrogate ids; they are
  not identifiers and are never parsed by clients.

When adding a filter or route that takes an identifier, take the StrKey. Task
0470 reached for the search classifier as a model, accepted hex on a new pools
filter, and had to be corrected — the exception looked like the rule.

### 6.3 Endpoint Inventory

| Resource        | Endpoint(s)                                                                                                                                                                                                         |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Network         | `GET /network/stats`                                                                                                                                                                                                |
| Transactions    | `GET /transactions`, `GET /transactions/:hash`                                                                                                                                                                      |
| Ledgers         | `GET /ledgers`, `GET /ledgers/:sequence`                                                                                                                                                                            |
| Accounts        | `GET /accounts`, `GET /accounts/:account_id`, `GET /accounts/:account_id/transactions`                                                                                                                              |
| Assets          | `GET /assets`, `GET /assets/:id`, `GET /assets/:id/transactions`                                                                                                                                                    |
| Contracts       | `GET /contracts`, `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`, `GET /contracts/:contract_id/decompiled`, `GET /contracts/:contract_id/invocations`, `GET /contracts/:contract_id/events` |
| NFTs            | `GET /nfts`, `GET /nfts/:id`, `GET /nfts/:id/transfers`                                                                                                                                                             |
| Liquidity Pools | `GET /liquidity-pools`, `GET /liquidity-pools/:id`, `GET /liquidity-pools/:id/transactions`, `GET /liquidity-pools/:id/chart`, `GET /liquidity-pools/:id/participants`                                              |
| Search          | `GET /search?q=&type=transaction,contract,asset,account,nft,pool&limit=10`                                                                                                                                          |

### 6.4 Resource Details

#### Network

**`GET /network/stats`** - Chain overview: current ledger sequence, TPS, total accounts,
total contracts.

This endpoint exists to support top-level explorer summary views and should remain small,
fast, and cacheable with short TTLs.

#### Transactions

**`GET /transactions`** - Paginated list. Query params: `limit`, `cursor`,
`filter[source_account]`, `filter[contract_id]`, `filter[operation_type]`.

**`GET /transactions/:hash`** - Full detail for a single transaction (supports both normal
and advanced representations):

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap"
    }
  ],
  "operation_tree": [...],
  "events": [...],
  "envelope_xdr": "...",
  "result_xdr": "..."
}
```

Backend expectations for transactions:

- list responses must stay optimized for table-style browsing
- detail responses must support both human-readable and advanced/debugging use cases
- advanced/raw fields should be returned only where they are explicitly part of the detail contract
- transaction filters should remain backend-driven so clients do not need to post-process large result sets

#### Ledgers

**`GET /ledgers`** - Paginated list of ledgers.

**`GET /ledgers/:sequence`** - Ledger detail including transaction count and linked
transactions.

Ledger endpoints are primarily historical/browsing endpoints and should be highly cacheable
once the ledger is closed and no longer mutable.

#### Accounts

**`GET /accounts`** - Paginated list of indexed accounts ordered by last activity
(`last_seen_ledger`; `?order=` flips asc/desc — the only sortable, indexed dimension).
Each row: `account_id`, native `xlm_balance` (nullable), `first_seen_ledger`,
`last_seen_ledger`, `home_domain`. Filter: `filter[with_domain]` (known/anchor accounts).
Deliberately omits address search (exact lookup is the global-search redirect) and any
balance ranking / `xlm_supply_percent` (no index / no XLM-supply source — see task 0274).

**`GET /accounts/:account_id`** - Account detail: current balances, sequence number,
and first/last seen ledger.

**`GET /accounts/:account_id/transactions`** - Paginated transactions involving this
account.

The current account scope is intentionally limited to:

- account summary
- balances
- recent transaction history

This keeps account support aligned with the currently documented product scope and avoids
expanding the backend contract beyond what the frontend is expected to show.

#### Assets

**`GET /assets`** - Paginated list of assets (native XLM, classic credit assets, SACs, and Soroban-native assets).
Query params: `limit`, `cursor`, `filter[type]` (native/classic_credit/sac/soroban), `filter[code]`.

`filter[code]` is a case-insensitive **substring**, matched against three things: the
asset code, and the on-chain `name` and `symbol` from contract metadata (task 0370 —
Soroban-native assets carry an empty code, so a code-only match left them unfindable).
The code it matches is the **displayed** one, `if(asset_type = 0, 'XLM', asset_code)` —
native XLM is stored with an empty code, so matching the stored column returned the
credit assets minted under a code containing "XLM" and never the real one (task 0470).
Classic SEP-1 enrichment names are deliberately NOT matched: classic assets are findable
by code, and substring-matching their names adds noise.

**`GET /assets/:id`** - Asset detail: asset code, issuer/contract, type, supply, holder
count, metadata. The numeric surrogate was dropped (PR #175 / the PG→CH composite move),
so `:id` is a single canonical token in one of three forms: a contract StrKey
(`C…`, for SAC / Soroban / native XLM-SAC), a `CODE-ISSUER` composite (classic credit,
e.g. `USDC-GA…`), or the reserved literal `native` (the classic XLM singleton, which
carries no composite identity). The response `id` field echoes that same canonical token
(contract StrKey → else `CODE-ISSUER` → else `native`), so a client routes by echoing it
verbatim. A bare numeric is rejected with `400 invalid_id`.

The displayed `name`, `symbol`, and `decimals` are **read-composed from side
tables**, not from the `assets` row — `assets.name` has had no writer since task 0297. On the ClickHouse read path `name` resolves `asset_enrichment.name`
(classic/SAC enrichment, task 0231) → `soroban_contract_metadata.name` (on-chain
SEP-41 `METADATA`, task 0297) → `'Stellar Lumens'` for native; `symbol` /
`decimals` come from `soroban_contract_metadata` (decimals defaults to 7 for
classic/SAC). See `endpoint-queries-clickhouse/{08,09}_get_assets*.sql`.

**`GET /assets/:id/transactions`** - Paginated transactions involving this asset
(addressed by the same `:id` token forms).

The backend must preserve the distinction between native, classic credit, SAC, and
Soroban-native assets while still serving all through a unified explorer API.

#### Contracts

**`GET /contracts`** - Paginated list of Soroban contracts, newest-deployed first
(`id DESC`, no user sort). Each row: `contract_id`, `contract_type` (+ decoded name),
`is_sac`, `sac_asset`, `deployer`, `deployed_at_ledger`, and `recent_invocations` (a 7-day count over
the same window as the contract-detail stats). Filters: `filter[type]` (token | other |
nft | fungible) and `filter[q]` (full-text over name + contract_id).
`sac_asset` (task 0441) is the classic asset a SAC mirrors —
`{asset_code, issuer}`, both `null` for native XLM — resolved by the reverse
`asset_sac` lookup: ONE whole-table aggregation per page batching every SAC id
in a single `IN` list (the table is ordered by the asset side, so the lookup
scans; accepted at 7.79 MiB measured, bloom_filter skip index is the named
upgrade past ~5M rows), skipped entirely when the page holds no SAC. `null`
on non-SAC rows and on the rare SAC with no resolvable facet row (frontend
keeps the bare badge).

**`GET /contracts/:contract_id`** - Contract identity (id, contract_id, deployer, WASM hash, deployed_at_ledger), classification (`contract_type`, `is_sac`, `sac_asset` — the mirrored classic asset per the list-endpoint semantics above, task 0441), mutability (`upgradeable`), and per-contract activity stats. `upgradeable` (task 0327) is 3-state: `true` iff the contract's current WASM imports the `update_current_contract_wasm` host fn (a self-upgrade path), `false` if it does not (effectively immutable/frozen; a SAC has no WASM and is always `false`), and `null`/Unknown when the WASM interface has not been parsed with the flag yet (the frontend renders no chip). There is no on-ledger immutability flag — the import set is the only signal. Resolved in the contract-header query from a `LEFT JOIN wasm_interface_metadata` (`JSONExtractBool(metadata,'upgradeable')`); ClickHouse-only, the retired PG path returns `null`. Per ADR 0042 / task 0156 the response no longer carries a `metadata` field — the underlying `soroban_contracts.metadata JSONB` was replaced with a typed `name` column, historically consumed by the search query; the detail page previously returned `{}` for every row and lost no information when the field was dropped. That `name` column has had no writer since task 0297 (empty going forward; on-chain token metadata now lives in the `soroban_contract_metadata` side table and is surfaced via /assets, not /contracts). Post-0243 (CH cutover complete, PG retired) it has **no reader**: the CH global search resolves contract names from `soroban_contract_metadata` (`22_get_search.sql`), and task 0304 dropped the last reader — the contracts-LIST name-search fallback on `sc.name`. The dead column is pending `DROP COLUMN` (task 0310).

**`GET /contracts/:contract_id/interface`** - Public function signatures (names, parameter
types, return types).

**`GET /contracts/:contract_id/decompiled`** - On-demand decompilation of the contract's
WASM (task 0465, issue #374). No persistence: the handler resolves `wasm_hash`, fetches
the code bytes live from Soroban RPC (`getLedgerEntries`, pool from `SOROBAN_RPC_URLS`),
and runs the pinned `soroban-ret` crate on the blocking pool with a 10 s in-handler
timeout. `?format=rust` (default) returns reconstructed Rust with completeness markers
(`functions`, `todo_holes`, `unknown_vars` — counts, not percentages, per the
soroban-ret team's guidance); when Rust emission fails the same response degrades to
`representation: "wat"` with `rust_error` set. `?format=wat` returns the (lossless)
WAT directly. 404 for SAC / pre-upload contracts (no WASM by design) and for code no
longer live on the ledger. Output is immutable per (`wasm_hash`, decompiler version) —
responses carry the `LONG` cache header.

**`GET /contracts/:contract_id/invocations`** - Paginated list of contract invocations.

**`GET /contracts/:contract_id/events`** - Paginated list of contract events.

Contract endpoints are the most Soroban-specific part of the API and should remain the main
place where indexed contract metadata and decoded usage history are exposed.

#### NFTs

**`GET /nfts`** - Paginated list of NFTs. Query params: `limit`, `cursor`,
`filter[collection]` (exact match), `filter[contract_id]` (C-StrKey), `filter[name]`
(substring; rejects `%`/`_` literals — canonical SQL `15_get_nfts_list.sql`).

**`GET /nfts/:id`** - NFT detail: name, token ID, collection, contract, owner, metadata,
media URL.

**`GET /nfts/:id/transfers`** - Transfer history for a single NFT.

NFT responses should tolerate sparse metadata because the ecosystem and available metadata
quality may vary significantly.

#### Liquidity Pools

**`GET /liquidity-pools`** - Paginated list of pools. Query params: `limit`, `cursor`,
`filter[asset_code]` (case-insensitive **substring** of either leg, so `USD`
matches the `USDC` pools; `A/B` is a pair query where each needle claims its own
leg in either order; native legs match on `XLM` despite storing an empty code —
tasks 0246/0440. The same parameter also accepts a pool **identifier** in the
`L…` SEP-23 form, which selects that one pool instead of matching codes —
task 0470), `filter[pool_kind]` (`classic` | `soroban`; an unknown value is
**rejected with 400**, never ignored — a silently dropped filter returns a page
that contradicts the request), and `filter[min_tvl]` (decimal). The two filters
combine additively.

The four per-leg positional filters (`filter[asset_a_code]` / `[asset_a_issuer]`
and the same for `b`) were **removed in task 0374**: they named a leg by its
position in a pair, which a list of two to four legs has no equivalent for, and
no client held a key to use them.

Each `PoolItem` carries `legs` — the pool's assets in registration order, two
for a classic pool and two to four for a Soroban one, replacing the
`asset_a` / `asset_b` pair — plus `pool_kind`, `participant_count` (count of
active LP positions; task 0246), the snapshot fields, and a compute-at-read
USD `tvl` (task 0199 Phase A2 — one batched price lookup per page; `volume`
and `fee_revenue` stay `null` on the list, they are detail-only).

A leg names its asset family in the **same vocabulary `/v1/assets` uses**
(`native` | `classic_credit` | `soroban`). It used to speak the XDR `AssetType`
domain here (`credit_alphanum4` / `pool_share`) while the sibling endpoint spoke
the family one — one field name, two vocabularies, coinciding on the single word
`native`. The raw discriminant is no longer published beside the label: the
label is a pure function of it, and it has no honest value for a Soroban token.
`filter[min_tvl]` is **rejected with 400**: a value computed at read cannot
filter page membership, and the old SQL pre-filter read a snapshot column that
is never written, so it silently returned an empty page. Filter and projection
semantics in canonical SQL `18_get_liquidity_pools_list.sql`.

**`GET /liquidity-pools/:id`** - Pool detail: legs, kind, fee, reserves, total
shares, TVL, plus `participant_count` (task 0246). `reserve_a` / `reserve_b`
stay pair-shaped — the snapshot table is — so on a three- or four-leg Soroban
pool the later legs list without an amount rather than being dropped. Reserves / total shares come from
the latest snapshot row; clients that care about freshness read
`latest_snapshot_at` in the response. `participant_count` is independent of
snapshot freshness — populated even on stale pools. The money fields
(`tvl`, `volume`, `fee_revenue`) do NOT come from the snapshot row: they are
computed at read from the in-cluster `prices.*` views (task 0199,
[ADR 0053](../../../lore/2-adrs/0053_fast-change-offchain-compute-at-read.md))
and are `null` when a leg is unpriceable. A prices-side failure degrades those
three fields to `null` — it never fails the request.

**`GET /liquidity-pools/:id/transactions`** - Deposits, withdrawals, and trades for this
pool. Each row carries `amounts` (task 0279): **one entry per operation**, in
application order, each with `amount_a` / `amount_b` for the pool's canonical
legs as raw-stroop decimal **strings** (same reason as `reserve_a` — a JSON
number is a browser double and a big leg would lose digits), **signed from the
pool's side** — positive = the asset entered the pool. A trade reads `+/-`, a
deposit `+/+`, a withdrawal `-/-`, so the sign alone gives the direction and no
event-type field is needed.

Per operation rather than summed per transaction because **8.2% of (pool,
transaction) pairs run more than one operation against the same pool** (measured
on prod 2026-08-12 over 8.49M pairs): a sum across a bundled deposit + path
payment is smaller than the deposit and can even flip sign shape, so it would
sit under an Event chip that does not describe it. An empty list means no
figures — never zero — for history the backfill has not reached; the frontend
renders those rows blank.

**`GET /liquidity-pools/:id/chart`** - Time-series data for TVL, volume, and fee revenue.
Query params (all optional, sensible defaults): `interval` (`1h`/`1d`/`1w`,
default `1d`), `from` (ISO 8601, default `to` minus interval-appropriate
window — `1h→7d`, `1d→90d`, `1w→104w`), `to` (ISO 8601, default `now()`,
exclusive upper bound). `from < to` enforced; bucket count capped to keep
aggregation bounded. Bucket aggregation policy in canonical SQL
`21_get_liquidity_pools_chart.sql`.

**`GET /liquidity-pools/:id/participants`** - Paginated list of liquidity providers
with their share size, share percentage of the pool, first deposit ledger, and last
update ledger. Powers the "Pool participants" table on the LP detail page
(frontend §6.14). Backed by `lp_positions` (ADR 0037 §16). Added during task 0167
to close a doc-drift gap between the frontend page and the original endpoint
inventory.

These endpoints combine factual current-state reads with historical aggregate reads, so the
backend should keep raw pool state and chart-series generation concerns clearly separated.

**Sentinel placeholder pools.** During partial / mid-stream backfills, the persist
layer can emit placeholder rows in `liquidity_pools` to satisfy the
`lp_positions.pool_id` FK when the parent pool's `LedgerEntry` is not in the
indexed window — see [ADR 0041](../../../lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md)
and the database-schema overview §4.14 "Sentinel placeholder rows". Marker:
`created_at_ledger = 0` (no real Stellar pool can carry this value — pubnet
genesis seq is 1). Every pool-surfacing endpoint above hides sentinel rows at
two layers: the handler-level `pool_exists()` gate filters them (so per-pool
endpoints return 404), and each of the five canonical SQL queries carries its
own sentinel predicate (`18` / `19` inline `lp.created_at_ledger > 0`,
`20` / `21` / `23` an `EXISTS` guard) for defense-in-depth against callers that
bypass the handler. Task 0193 implements this filter.

#### Search

**`GET /search?q=&type=transaction,contract,asset,account,nft,pool&limit=10`** - Generic
search across all entity types. The classifier maps the raw `q` to two derived inputs
consumed by the canonical SQL: `hash_bytes` (32-byte BYTEA — drives `transaction` and
`pool` exact-match branches because pool ids are also 32-byte BYTEA) and `strkey_prefix`
(upper-cased StrKey or any `G…` / `C…` prefix — drives the `account` and `contract`
prefix branches). The raw `q` is also fed to the trigram / FTS branches (`assets`,
`nfts`, `soroban_contracts.search_vector`).

**Pools match on two shapes** (task 0470). A hash-shaped `q` is a point seek on
`pool_id`, the full ORDER BY key. Anything else is treated as an asset code and matched
with the SAME rule the pools list uses — case-insensitive substring against either leg,
`A/B` pair syntax where each needle claims its own leg in either order, and native XLM
resolved by `asset_type = 0` rather than by its (empty) stored code. The predicate is
defined once in `crates/api/src/common/pool_asset_codes.rs` and called by both
`/v1/search` and `/v1/liquidity-pools`, so the two surfaces cannot answer the same
question differently. Before this, a non-hash query matched no pool at all: `KALE`
returned 0 in search while the pools page returned 58.

Behaviour:

- when `q` is a fully-typed entity id (64-hex hash, full G-StrKey, full C-StrKey) **and**
  an exact row exists in `transaction_hash_index` / `liquidity_pools` / `accounts` /
  `soroban_contracts`, the response is `{ "type": "redirect", "entity_type", "entity_id" }`
  and the frontend navigates directly to the entity page.
- otherwise the response is `{ "type": "results", "groups": {...} }` with up to `limit`
  rows per entity bucket (default 10, hard ceiling 50). Each row carries the same
  columns regardless of bucket: `entity_type`, `identifier` (the human-shown id),
  `label`, and `route_token`. The frontend routes a hit on `route_token ?? identifier`:
  for transaction / account / contract / pool the display `identifier` IS the routable id
  (hash / StrKey / `L…`) so `route_token` is `null`; for `asset` the `identifier` is the
  non-routable asset code, so `route_token` carries the canonical `/assets/:id` token
  (contract StrKey | `CODE-ISSUER` | `native`, identical to the detail route);
  `nft` routes on the composite `(contract_id, token_id)` it also projects. The dropped
  numeric `surrogate_id` was replaced by `route_token` (task 0243) — search hits no longer
  emit a key the detail routes reject. `groups` includes only buckets that have at least
  one match — empty buckets are omitted from the response (the OpenAPI schema marks them
  optional); frontend treats absent and empty array identically.

Authoritative SQL:
[`22_get_search.sql`](../database-schema/endpoint-queries-clickhouse/22_get_search.sql) — UNION ALL
of six narrow CTEs, each `LIMIT $per_group_limit`-bounded, with `:include_*` BOOLEAN
flags resolved from the optional `?type=` filter (the planner removes branches whose
flag is FALSE).

No caching: `q` variability makes a TTL cache useless and the per-CTE `LIMIT` keeps each
query bounded.

### 6.5 Response Caching

Per task 0055, every public endpoint sets an explicit `Cache-Control` header.
Its consumer today is the **user's browser** (plus any future edge cache) —
the API Gateway stage cache was never enabled and stays off by decision
(2026-08-14, task 0455; rationale and return condition in
[`api-gateway-cache-spec.md`](./api-gateway-cache-spec.md)). Constants
live in [`crates/api/src/common/cache_control.rs`](../../../crates/api/src/common/cache_control.rs).

| Tier             | `Cache-Control`                      | Endpoints                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Long** (300s)  | `public, max-age=300`                | `GET /ledgers/:sequence` (closed), `GET /transactions/:hash` (heavy archive overlay available)                                                                                                                                                                                                                                                                                 |
| **Medium** (60s) | `public, max-age=60`                 | `GET /assets/:id`, `GET /contracts/:contract_id`, `GET /contracts/:contract_id/interface`, `GET /nfts/:id`, `GET /liquidity-pools/:pool_id/chart`                                                                                                                                                                                                                              |
| **Short** (10s)  | `public, max-age=10`                 | `GET /accounts/:account_id` and its sub-resource, head-ledger detail, `GET /transactions/:hash` (heavy unavailable), `GET /contracts/:contract_id/{invocations,events}`, non-live lists, etc.                                                                                                                                                                                  |
| **Live** (0s)    | `public, max-age=0, must-revalidate` | `GET /network/stats`, `GET /ledgers` (list), `GET /transactions` (list) — the per-ledger live polls. Any browser-cache TTL ≥ the ~5.8 s ledger cadence re-serves a stale payload to the adaptive poll and the feed batches 2-3 ledgers per visible update, so these endpoints opt out of HTTP caching entirely; request coalescing is the in-process moka layer's job instead. |
| **No-store**     | `no-store`                           | `GET /search` (variable `q`); also forced on every non-2xx response by tower middleware (`enforce_no_store_on_errors`) — error envelopes never reach the gateway cache                                                                                                                                                                                                         |

Two endpoints carry **conditional** logic:

- `GET /ledgers/:sequence` — Long when `next_sequence` is `Some` (closed
  ledger, immutable per Stellar consensus); Short when the requested ledger
  is the chain head and the indexer may still be settling.
- `GET /transactions/:hash` — Long when `heavy_fields_status = Ok` (full
  archive overlay merged); Short when archive fetch failed
  (`heavy_fields_status = Unavailable`) so a retry can pick up the archive
  sooner. The handler also **short-circuits the archive fetch entirely**
  for any row carrying `parse_error = true` in the DB (task 0190):
  re-fetching cannot make a degraded row whole, and serving the row through
  the unavailable-heavy path preserves the lore-0044 / lore-0046 contract
  (light slice always returned, heavy explicitly absent). The Short TTL
  applies in this case as well, so a fix that ever re-parses the row
  cleanly surfaces within one ledger cycle.

The 10s value matches the API Gateway `apiGatewayCacheTtlMutable` config in
`infra/envs/{staging,production}.json`. Lowering below 10s is wasted (gateway
clamps to its configured floor); raising above 10s would expose stale data
past one Stellar ledger cycle (~5s).

#### Conditional GET — `ETag` / `304` on the Live tier (task 0292)

The Live tier already carries `must-revalidate`, so every poll round-trips to
the origin. Task 0292 makes those round-trips cheap with a conditional GET
keyed on the chain head, implemented in
[`crates/api/src/common/conditional.rs`](../../../crates/api/src/common/conditional.rs):

- **`ETag` value is the chain head** (`latest_ledger_sequence`) from
  `crate::common::head` — the same cheap single-row probe the version-keyed
  cache uses (see §8.1). Emitted on `200` by `GET /network/stats` and the
  **live first page** of `GET /transactions` and `GET /ledgers`. Two validator
  strengths (`crate::common::conditional`):
  - **Lists → strong** (`"<head>"`). The list envelope is byte-stable for a
    given head, so a strong tag is correct. It is derived from the **returned
    body** (the newest row's `ledger_sequence`/`sequence`), not the pre-query
    head, so the validator always equals the bytes sent even if a ledger lands
    between the head probe and the query.
  - **`/network/stats` → weak** (`W/"<head>"`). The stats body carries a
    per-SELECT `generated_at` wall-clock, so two `200`s at the same head can
    differ byte-for-byte (also via a cache recompute or the last-good
    fallback); a strong validator would violate RFC 7232 §2.1. `If-None-Match`
    uses weak comparison regardless, so the `304` short-circuit is identical.
- **`If-None-Match` short-circuits to `304 Not Modified` _before_ the heavy
  query.** On a request whose `If-None-Match` already names the current head,
  the handler returns an empty-body `304` after only the cheap head probe — the
  35M-row list / stats statement never runs. This is the load-bearing
  condition: if the tag were computed by running the query, only egress would
  be saved, not the warehouse read. An idle poll therefore costs one head probe
  and nothing else, and external API clients (task 0277), whose polling we do
  not control, stop re-reading the warehouse every tick.
- **Scope — live first page only.** The head is a valid validator only for a
  response that is a pure function of the latest ledger: `GET /network/stats`,
  and the lists when there is **no cursor** (for `GET /ledgers`, additionally
  only newest-first — `?order=asc` returns the immutable oldest page). Cursored
  (historical) pages are head-independent, so a head-keyed `ETag` would just
  revalidate to `200` on every poll; they are excluded and keep their existing
  behaviour, and the extra head probe is paid only on the polled live request.
  Filtered first pages are included — a filter narrows the rows but they still
  change only when a new ledger lands, so head-keying is never stale.
- **`304` is not an error.** It carries the same `ETag` and the `LIVE`
  `Cache-Control` the matching `200` would, and is explicitly exempted from the
  `enforce_no_store_on_errors` middleware (which otherwise stamps `no-store` on
  every non-2xx) — stamping `no-store` on a `304` would break the
  conditional-GET contract.
- **Edge passthrough.** Cloudflare (edge auth, task 0277) and API Gateway
  (proxy integration) forward `If-None-Match` / `ETag` / `304` untouched — the
  edge only injects `X-Edge-Secret` and does not strip cache validators.
- **Shared caches never store the `304`.** The `304` carries
  `Cache-Control: public, max-age=0, must-revalidate`, so a compliant shared
  cache (CDN / API Gateway stage cache) never stores it, and an RFC 7234 cache
  never serves a stored `304` to a request lacking matching validators. This
  matters because the gateway cache key is path+query only (no `Vary` on
  `If-None-Match` / `Authorization`; see
  [`api-gateway-cache-spec.md`](./api-gateway-cache-spec.md)) — `max-age=0`
  plus the spec's defensive "cache-only-200" guidance keeps an empty-body
  `304` from ever being replayed to another client. Cross-tenant safety does
  not rely on this anyway: the list/stats bodies are chain-wide public data
  with no per-caller content, so `public` is data-correct even behind the auth
  gate. (A future change that puts any per-caller data in these bodies MUST
  drop `public` or add `Vary`.)
- **Not a replacement for 0290.** Conditional GET cuts _how often_ a client
  fetches, not the cost of one fetch: when a client must fetch (new ledger,
  first paint), the list statement still reads its rows. The load-bearing
  warehouse-query fix remains task 0290.

Cache-key requirements (consumed by CDK task 0097): full path + every query
parameter, including `cursor`. Different filter combinations produce
distinct cache entries. See
[`api-gateway-cache-spec.md`](./api-gateway-cache-spec.md) for the
infrastructure contract.

## 7. Data Access and Response Model

### 7.1 Source of Data

List endpoints and all partition-pruned reads come from the block explorer's own
ClickHouse store. Heavy-field detail endpoints (E3 `/transactions/:hash`,
E14 `/contracts/:id/events`) additionally fetch raw `.xdr.zst` from the **public
Stellar ledger archive** and re-parse it at request time per
[ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md).
Detail-only off-chain fields exempt from persistence per
[ADR 0043](../../../lore/2-adrs/0043_field-allocation-rule.md) (§4 `runtime_enrichment`
umbrella) are also fetched at request time — currently the SEP-1 issuer TOML
(asset `description` / `home_page`, task 0188) and the NFT `token_uri()` JSON
(`metadata`, task 0195). The API does not depend on Horizon or third-party
indexers for any response.

### 7.2 Response Shaping

The backend should expose read models designed for explorer use, not raw storage rows.
That means:

- flattening or restructuring nested data where that improves client usability
- attaching human-readable labels produced upstream during ingestion
- keeping raw protocol payloads available only for advanced/detail use cases
- preserving stable identifier fields needed for linking across pages

### 7.3 Pagination Semantics

Collection endpoints should use cursor-based pagination consistently.

API-level expectations:

- cursors are opaque to clients
- pagination should not depend on expensive total counts
- ordering should remain deterministic for stable browsing
- list filters must be applied in the backend query layer, not in the client

### 7.4 Normal vs Advanced Transaction Data

Transaction detail is the clearest example of a dual-mode backend contract:

- the normal view is centered on decoded operations and call trees
- the advanced view includes raw parameters, raw event payloads, and raw XDR where needed

The backend should treat these as two representations over the same transaction resource,
not as two unrelated endpoints with diverging semantics.

## 8. Caching and Performance

### 8.1 Caching Strategy

Caching operates at two levels:

- **API Gateway response caching** - responses for immutable data (historical
  transactions, closed ledgers) are cached with long TTLs at the API ingress layer. Mutable
  data (recent transactions, network stats) uses short TTLs (5-15 seconds). CloudFront is
  reserved for static frontend/document delivery in the initial topology.
- **Backend in-memory caching** - frequently accessed reference data is cached in the
  Lambda execution environment to reduce database round-trips. Reference data
  (contract metadata) uses TTLs of 30-60 seconds; live-polled data (network stats)
  is **version-keyed on the chain head** rather than a pure TTL, so a new ledger
  is visible on the first request after it is written (see below). All
  TTL-based in-process caches are built on the
  `moka` crate via the shared `crate::cache::ttl_cache` helper in
  `crates/api/src/cache.rs`, which fixes the TTL + `max_capacity` bound
  and yields lock-free reads, TinyLFU eviction and stampede protection
  out of the box (see task 0180). Concrete caches:
  - `ContractMetadataCache` (`crates/api/src/contracts/cache.rs`,
    45 s TTL, 10 000 entries) — keyed by contract StrKey, populated on
    `GET /v1/contracts/{contract_id}`.
  - `NetworkStatsCache` (`crates/api/src/network/cache.rs`) — **version-keyed
    on `latest_ledger_sequence`** (task 0291), not a pure TTL. Each request
    first reads the head cheaply via `crate::common::head` (a single-row read
    over the `ledgers` ordering key — PG `max(sequence)` over the PK, CH
    `ORDER BY sequence DESC LIMIT 1`; see §6.4 "Live" tier) and looks the cache
    up under that sequence: an unchanged head is a HIT, an advanced head misses
    and recomputes once. This eliminates the up-to-TTL window where the
    previous head was served, while `moka::future::Cache::try_get_with` still
    collapses concurrent misses on the same head down to a single DB query. The
    stats statement **pins** its latest-ledger row to the head it was keyed on
    (`WHERE sequence = head`), so the response's `latest_ledger_sequence`
    always equals the cache key (no TOCTOU / key-vs-body divergence) and the
    head is read once per miss, not twice. A generous backstop `time_to_live`
    (60 s) plus a small `max_capacity` only reclaim dead head keys — they are
    not the freshness mechanism. Because the head read is now a hard dependency
    in front of the cache (a warm HIT no longer means zero DB round-trips), a
    head-read failure falls back to the **last good snapshot** (`AppState
.network_last_good`, written on each miss) rather than a 500, preserving
    the old "warm cache survives a transient DB/CH blip" property. The same
    cheap head source is reused by the `ETag`/`304` conditional-GET layer
    (task 0292).
    Every cache is a field of `AppState` and shared across handler
    invocations on the same warm Lambda container; cold starts begin with
    empty caches and rebuild on demand.

### 8.2 Performance Expectations

The backend should optimize for explorer-style read traffic:

- many small GET requests from route-driven navigation
- repeated detail-page lookups for well-known identifiers
- list browsing with filters and cursor pagination
- bursty traffic on recently closed ledgers and popular contract/token pages

The API should avoid pushing expensive post-processing to the client when that would create
inconsistent results or duplicated logic across screens.

## 9. Fault Tolerance and Operational Notes

### 9.1 Fault Tolerance

- **Ingestion lag** - if the Galexie pipeline falls behind, the API continues serving
  data from the database with a freshness indicator showing the highest indexed ledger
  sequence. A CloudWatch alarm fires when no new ledger file lands in S3 for the
  configured lag window (5 min in production); see
  `docs/architecture/technical-design-general-overview.md` §3.7 for the full
  alarm set.
- **Lambda cold starts** - mitigated via Rust's fast startup on ARM/Graviton2 and provisioned concurrency
  at higher traffic tiers.
- **Connection handling** - the `clickhouse` HTTP client reuses a hyper connection
  pool per warm Lambda; there is no external connection proxy.

### 9.2 Operational Boundary

The backend should degrade gracefully when upstream ingestion is delayed. It should serve
what is already indexed and make freshness visible rather than failing simply because the
network tip has advanced.

It should also remain operationally simple:

- read-only with respect to chain data
- no dependence on external chain APIs for core functionality
- clean separation between transport concerns, query logic, and response shaping

## 10. Workspace Placement and Delivery Notes

The target workspace will provide the structural backend boundary (`crates/api`, per tasks 0094/0095) but the
Rust/axum runtime implementation is not yet in place. That is consistent with the repository README and
current bootstrap status.

Expected code placement:

- `crates/api` for application bootstrap, route wiring, axum modules, and runtime integrations
- `crates/domain` for reusable explorer-domain types and value objects shared across backend crates

This document should be treated as the detailed reference for future backend implementation
planning, with
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remaining
the primary source of truth.
