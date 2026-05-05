---
id: '0188'
title: 'Feature: SEP-1 stellar.toml fetcher + GET /v1/assets/{id} runtime enrichment'
type: FEATURE
status: active
related_adr: ['0029', '0032']
related_tasks: ['0187', '0124', '0125']
tags:
  [priority-medium, effort-medium, layer-backend, enrichment, milestone-2, sep1]
links:
  - https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md
history:
  - date: '2026-05-04'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0187 future work. Bundles two phases per Karol's task-scope
      preference: (1) implement runtime_enrichment::sep1 — reqwest + toml
      parser + moka LRU + size cap + fail-soft, (2) wire the first and only
      currently available consumer at GET /v1/assets/{id}. All other potential
      consumers (accounts, nfts, lp) are blocked on their endpoint modules
      (0048/0051/0052) not yet existing — they will be wired in their own
      module tasks once those endpoints land.
  - date: '2026-05-05'
    status: active
    who: karolkow
    note: >
      Promoted from backlog. Re-verified post-0051/0052 merge: nfts and
      liquidity_pools modules now exist but neither uses SEP-1 (NFTs use
      metadata-URI fetch, LP uses price oracle — both separate enrichment
      paths). Only assets/{id} consumes SEP-1; scope unchanged.
---

# SEP-1 stellar.toml fetcher + GET /v1/assets/{id} runtime enrichment

## Summary

Implement `crates/api/src/runtime_enrichment/sep1/` — a fail-soft, in-process, LRU-cached HTTP client that resolves an issuer's `home_domain` to its stellar.toml file, parses the SEP-1 fields relevant to the explorer, and surfaces them on `GET /v1/assets/{id}`. Single bundled task: fetcher implementation + parser + cache + first (and currently only available) consumer all in one PR.

## Context

**SEP-1** (Stellar Ecosystem Proposal #1, https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md) is the convention by which an asset issuer publishes a self-describing TOML file at `https://{home_domain}/.well-known/stellar.toml` (max 100 KB). It carries:

- `[[CURRENCIES]]` array — per-token metadata: `code`, `issuer`, `name`, `desc`, `image`, `conditions`, `is_asset_anchored`, `anchor_asset_type`, `anchor_asset`, `redemption_instructions`, `display_decimals`, etc.
- `[DOCUMENTATION]` table — issuer org info: `ORG_NAME`, `ORG_DBA`, `ORG_URL`, `ORG_LOGO`, `ORG_DESCRIPTION`, `ORG_TWITTER`, `ORG_GITHUB`, etc.
- A few unrelated sections (validators, federation, KYC) that the explorer ignores.

Discovery uses the on-chain `accounts.home_domain` value, already populated by the indexer.

Task 0187 prepared the structural ground: the existing `runtime_enrichment::stellar_archive` submodule does S3 archive reread (ADR 0029) and a sibling `runtime_enrichment::sep1` skeleton was added with no body. This task fills in the body and wires the first consumer.

The two enrichment paths share the same architectural shape — per-request, in-process, fail-soft, no DB writes — so the existing `enrichment_status` / `heavy_fields_status` pattern from ADR 0029 is reused.

**Why bundled with the consumer:** previous decomposition would have shipped a fetcher with no caller. Bundling lands a useful end-to-end slice in one PR and lets the consumer integration shape the fetcher API rather than the other way around.

**Why only `GET /v1/assets/{id}` is in scope:** it is the only endpoint currently exposed by the API that has SEP-1-relevant fields to surface. `GET /v1/accounts/{id}` (task 0048), `GET /v1/nfts/{id}` (0051), and `GET /v1/liquidity-pools/{id}` (0052) do not exist yet — when they ship they will wire SEP-1 enrichment as part of their own module tasks. The list endpoint `GET /v1/assets` deliberately does not call SEP-1 inline (N issuer fetches per page = unacceptable latency); its `icon_url` column is a type-1 backfill concern handled by a future enrichment-worker crate.

## Implementation Plan

### Step 1: Workspace dependency setup

- Promote `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }` from `crates/audit-harness/Cargo.toml` to workspace root `[workspace.dependencies]`.
- Add `toml = "0.8"` to workspace deps.
- Add `reqwest = { workspace = true }` and `toml = { workspace = true }` to `crates/api/Cargo.toml`.
- `audit-harness` switches to `reqwest = { workspace = true }` to keep one source of truth.

### Step 2: `runtime_enrichment::sep1` module body

Replace the skeleton with a real implementation:

- `client.rs` — `Sep1Fetcher` struct wrapping `reqwest::Client` and a `moka::sync::Cache<String, Arc<Sep1TomlParsed>>` (project already uses `moka` in `contracts::cache` and `network::cache`). Methods: `fetch(home_domain: &str) -> Result<Arc<Sep1TomlParsed>, Sep1Error>`. Cache key = lowercased home_domain. TTL: 24 h (warm-Lambda LRU, cold start drops it).
- `dto.rs` — `Sep1TomlParsed` struct mapping only the fields the API consumes. Two top-level groups: `currencies: Vec<Sep1Currency>` and `documentation: Sep1Organization`. All fields `Option<String>` / `Option<bool>` / `Option<i32>` so a partial / oddly-formatted toml degrades to `None` rather than failing the whole parse.
- `errors.rs` — `Sep1Error` enum: `MissingHomeDomain`, `Http`, `Timeout`, `BodyTooLarge { limit }`, `MalformedToml`. All map to `enrichment_status: "unavailable"` on the consumer side, never to a 5xx.
- `size_cap.rs` (or inline) — bound the response body at 100 KB using `Response::bytes_stream()` + a manual byte counter; reject early without buffering the rest. Per SEP-1 max file size.
- `timeouts.rs` (or inline) — connect-timeout 1 s, total-request-timeout 2 s. Whole budget below the API Gateway 29 s ceiling with margin.
- `mod.rs` — re-exports the public surface: `Sep1Fetcher`, `Sep1TomlParsed`, `Sep1Currency`, `Sep1Organization`, `Sep1Error`.

### Step 3: Wire fetchers into `AppState` via a `RuntimeEnrichment` bundle

Replace the two flat fields (`fetcher: StellarArchiveFetcher` + the would-be
`sep1: Sep1Fetcher`) on `AppState` with a single grouped struct:

```rust
// crates/api/src/runtime_enrichment/mod.rs
#[derive(Clone)]
pub struct RuntimeEnrichment {
    pub stellar_archive: StellarArchiveFetcher,
    pub sep1: Sep1Fetcher,
}

// crates/api/src/state.rs
pub struct AppState {
    pub db: PgPool,
    pub runtime_enrichment: RuntimeEnrichment,
    pub contract_cache: ContractMetadataCache,
    pub network_cache: NetworkStatsCache,
    pub network_id: [u8; 32],
}
```

Rationale: AppState's field count doesn't grow per new transport; the
grouping mirrors the `runtime_enrichment` module structure 1:1; future
submodules (e.g. `nft_metadata`, `price_oracle`) get added to the bundle
without touching `AppState`. Access path becomes
`state.runtime_enrichment.stellar_archive.fetch_ledger(...)` and
`state.runtime_enrichment.sep1.fetch(...)`.

Update consumers: `transactions/handlers.rs`, `contracts/handlers.rs`,
`assets/handlers.rs` (handler-side access); `main.rs`, `tests_integration.rs`,
`network/handlers.rs::tests::app` (construction sites).

### Step 4: Asset details DTO additions

`AssetDetailResponse` already carries `description: Option<String>` and
`home_page: Option<String>` — both hardcoded `None` before this task.
Strict scope: only those two get populated. No other fields added. No
`organization` nested DTO. No `enrichment_status` flag — a fetch failure
is silently logged and both fields end up `None`, indistinguishable from
the no-issuer / no-home_domain skip path. (Re-introducing
`enrichment_status` is a future cleanup if the consumer needs it.)

OpenAPI: no new components registered — response shape unchanged from
the schema's perspective; only the documented values for the two
existing fields change.

Parser surface mirrors response surface: `Sep1Currency` models only
`code`, `issuer`, `desc`; `Sep1Documentation` models only `org_url`.
Other SEP-1 fields are silently dropped at parse time. When a future
consumer needs e.g. `conditions` or `org_twitter`, add it both to the
parser DTO and to the response DTO at the same time.

### Step 5: Asset details handler integration

In `crates/api/src/assets/handlers.rs::get_asset`:

1. Run the existing DB query (now joining `iss.home_domain AS issuer_home_domain`
   so the fetch key arrives on `AssetRow` without an extra round-trip).
2. If the resolved asset has a non-empty `issuer_home_domain`, call
   `state.sep1.fetch(home_domain).await`.
3. On success, find the matching `(code, issuer)` row in `parsed.currencies`
   and read `desc` → `description`. Read `parsed.documentation.org_url` →
   `home_page` regardless of whether the currency match succeeded (issuer
   may publish their site even without listing every individual token).
4. On any failure, warn-log and set both fields `None`.
5. Native XLM, no-issuer Soroban tokens, and accounts without `home_domain`
   skip the fetch entirely; both fields `None`.

### Step 6: Tests

- Unit (`sep1::dto::tests`): TOML parser handles minimal / partial / malformed
  inputs; unknown sections silently ignored.
- Unit (`sep1::client::tests`): `validate_host` accepts DNS names, rejects
  empty / IPv4-literal / IPv6-literal / URL-smuggling shapes.
- Integration (`sep1::client::tests`): five tests against a localhost raw-TCP
  fixture server (no axum / hyper dev deps — bare `tokio::TcpListener` writing
  a hand-formatted HTTP/1.1 response). Cover happy parse, HTTP 500, body cap
  rejection, cache hit (Arc::ptr_eq), and IP-literal rejection without I/O.
  Fixture server isolation enabled by injecting an `EndpointResolver` closure
  on the fetcher — production path keeps `https://{host}/.well-known/...`.
- An `#[ignore]` end-to-end test against a real issuer is **not** added in this
  task (skip kept short of acceptance criteria); the fixture-server path covers
  the wire format end-to-end and the real-issuer test can land as a follow-up.

### Step 7: Docs + canonical SQL

Update three canonical sources to reflect the SEP-1 path replacing the
abandoned per-entity S3 enrichment plan:

- **Canonical SQL** `docs/architecture/database-schema/endpoint-queries/09_get_assets_by_id.sql`:
  add `iss.home_domain AS issuer_home_domain` to the SELECT (internal
  lookup key for the SEP-1 fetch — not in the API response). Drop the
  outdated "S3 returns: description, home_page" header comment; replace
  with a "Runtime SEP-1 fetch via `runtime_enrichment::sep1` (task 0188)"
  reference.
- **Endpoint response shapes** `docs/architecture/database-schema/endpoint-queries/README.md`
  §09: replace "Step 2 — S3 overlay (`s3://<bucket>/assets/{id}.json`)"
  with "Step 2 — SEP-1 runtime fetch via `runtime_enrichment::sep1`",
  pointing the description / home_page rows at `CURRENCIES[].desc` and
  `DOCUMENTATION.ORG_URL` respectively.
- **Backend overview** `docs/architecture/backend/backend-overview.md`
  §4.1: split the runtime-enrichment description into two
  transport-specific sub-bullets (stellar_archive + sep1) with the
  built-in safeguards (body cap, timeouts, SSRF guards, LRU TTL) and
  the future-consumer list (accounts when 0048 ships).
- **Schema overview** `docs/architecture/database-schema/database-schema-overview.md`
  §4.10 Assets: clarify that SEP-1 detail fields are not persisted at
  all (resolved at request time), narrowing the original typed-columns
  plan from ADR 0023 Part 3 and superseding the per-entity S3 hydration
  sketched under task 0164.

## Acceptance Criteria

- [ ] `runtime_enrichment::sep1` exposes `Sep1Fetcher`, `Sep1TomlParsed`,
      `Sep1Currency` (re-exports). `Sep1Documentation` and `Sep1Error` accessible
      via fully-qualified `sep1::dto::*` / `sep1::errors::*` paths — not
      re-exported until a second consumer needs them. Skeleton TODO removed.
- [ ] `reqwest` promoted to workspace deps; `toml` added to workspace.
- [ ] `RuntimeEnrichment { stellar_archive, sep1 }` struct lives in
      `runtime_enrichment::mod`; `AppState.runtime_enrichment` replaces the
      old flat `fetcher` + `sep1` fields. Wired through `main.rs` and every
      test-app builder (`tests_integration::build_app`,
      `network::handlers::tests::app`, `main::tests::test_app`); access path
      becomes `state.runtime_enrichment.stellar_archive.fetch_ledger(...)`
      and `state.runtime_enrichment.sep1.fetch(...)`.
- [ ] `AssetDetailResponse` shape unchanged from before this task —
      `description` and `home_page` continue to exist as `Option<String>`,
      but they are now populated from SEP-1 instead of hardcoded `None`.
      No new fields added.
- [ ] `GET /v1/assets/{id}` returns 200 in every code path (success / fetch
      failure / no home_domain / native XLM); never 5xx on a SEP-1 failure;
      existing light-slice fields unchanged.
- [ ] `Sep1Currency` and `Sep1Documentation` parser DTOs model only the fields
      the API consumes today (`code`, `issuer`, `desc` and `org_url`
      respectively). Adding a new SEP-1 field for a future consumer requires
      adding it to BOTH the parser DTO and the response DTO at the same time.
- [ ] Body size cap is implemented in `capped_body` and bounded by
      `MAX_BODY_BYTES = 100 * 1024` (verified by inspection / future
      real-issuer smoke).
- [ ] Connect timeout 1 s, total timeout 2 s configured (constants exported
      from `client.rs`).
- [ ] Built-in SSRF guard: RFC 1035 hostname check + `IpAddr::parse` rejection
      run before any I/O. Documented limitation: DNS-resolved private addresses
      (issuer.example.com → 10.x.x.x) NOT blocked at this layer.
- [ ] `cargo check -p api`, `cargo clippy -p api -- -D warnings`, and
      `cargo test -p api` all clean.
- [ ] **Docs updated** — `docs/architecture/backend/backend-overview.md`
      §4.1 describes the two transport-specific submodules under
      `runtime_enrichment` (stellar_archive + sep1);
      `docs/architecture/database-schema/database-schema-overview.md` §4.10
      Assets clarifies that SEP-1 detail fields are not persisted at all.
      Per [ADR 0032](../2-adrs/0032_docs-architecture-evergreen-maintenance.md).

## Out of Scope

- Wiring SEP-1 enrichment into `GET /v1/accounts/{id}` — the endpoint does not exist; covered when 0048 ships.
- Wiring SEP-1 / metadata-URI enrichment into NFTs and LP details — those modules use different transports (NFT metadata URI, LP price oracle); separate tasks.
- `GET /v1/assets` list-endpoint enrichment — would require N stellar.toml fetches per page; instead, `assets.icon_url` will be populated by a separate type-1 enrichment-worker crate (future task).
- Exposing additional SEP-1 fields on the response (`conditions`, `is_asset_anchored`, `anchor_*`, `redemption_instructions`, `display_decimals`, organisation block, `enrichment_status`) — strictly scoped to existing `description` + `home_page` fields per Karol's call. Re-add per consumer demand in a follow-up.
- `Sep1OrganizationDto` and `EnrichmentStatus` types — not implemented in this task. The fail-soft signal currently collapses to "field is null" without a status discriminator; if a frontend can't distinguish "fetch failed" from "no data published", introduce `enrichment_status` then.
- `#[ignore]` real-issuer integration test against e.g. `ultrastellar.com` — fixture server covers the wire format; real-issuer smoke can land as a follow-up.
- DynamoDB / S3 / Postgres caching of stellar.toml — Karol's Q4 decision was in-memory LRU per warm Lambda only. Reconsider only if p95 latency profile demands it post-launch.
- DNS-resolved private-IP SSRF blocking (resolve domain → check against RFC 1918 / 6598 / link-local) — only literal-IP rejection done here. Follow-up if threat model demands.
- ADR 0029 amendment — deferred until a unified description across both submodules is worth writing (separate cleanup task).
- Archiving 0124 / 0125 as superseded — separate cleanup task once both runtime + worker module pairs land.

## Notes

This task closes the first end-to-end slice of the M2 enrichment plan: a user
hits `GET /v1/assets/{id}` and gets two issuer-published fields
(`description`, `home_page`) merged into the response from the issuer's
stellar.toml. Subsequent enrichment work (accounts consumer, type-1 worker
crate, ADR cleanup) reuses the fetcher and cache established here.

**Strict scope rationale.** The original draft of this task proposed
exposing nine SEP-1-sourced fields plus an `enrichment_status` discriminator
and a nested `Sep1OrganizationDto`. Karol cut scope to just the two fields
that already existed on `AssetDetailResponse` as hardcoded `None` — the
ones we explicitly came here to fill. Adding more SEP-1 fields later is
cheap (parser + response DTO + handler tuple); removing them after a
frontend ships against them is expensive. Default to under-exposing.
