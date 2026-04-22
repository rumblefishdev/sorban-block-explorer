---
id: '0153'
title: 'API: wire E3 and E14 endpoints using stellar_archive library (follow-up to 0150)'
type: FEATURE
status: backlog
related_adr: ['0027', '0029', '0030', '0031']
related_tasks: ['0150', '0149', '0151', '0152']
blocked_by: []
tags: [layer-api, priority-high, effort-medium, follow-up]
milestone: 1
links:
  - lore/1-tasks/archive/0150_FEATURE_api-xdr-fetch-read-path.md
  - lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
history:
  - date: '2026-04-22'
    status: backlog
    who: FilipDz
    note: >
      Spawned from 0150. Library pieces under `crates/api/src/stellar_archive/`
      (StellarArchiveFetcher, extractors, DTOs, merge functions) are done and
      verified against live public archive. This task wires them into axum
      handlers for E3 and E14 with DB queries, cursor pagination, shared
      state, and response DTOs. Completes AC #4 of 0150 (staging golden
      fixture integration test).
---

# API: wire E3 and E14 endpoints using stellar_archive library

## Summary

Task 0150 delivered the library layer in `crates/api/src/stellar_archive/`
— `StellarArchiveFetcher`, heavy-field DTOs, extractors, merge functions
with graceful-degradation primitives. Only `/health` is currently exposed
by `crates/api/`. This task adds the two HTTP endpoints that depend on XDR
fetch per ADR 0029:

- **E3** `GET /transactions/:hash`
- **E14** `GET /contracts/:id/events`

## Context — what's already in place

Delivered by 0150 (do not rebuild):

- `stellar_archive::StellarArchiveFetcher` with `fetch_ledger(seq) -> Result<LedgerCloseMeta, FetchError>` and `fetch_ledgers(&[seq]) -> Vec<Result<..>>`.
- `stellar_archive::extractors::{extract_e3_heavy, extract_e14_heavy}`.
- `stellar_archive::dto::{E3HeavyFields, E14HeavyEventFields, E3Response<T>, E14EventResponse<E>, HeavyFieldsStatus}`.
- `stellar_archive::merge::{merge_e3_response, merge_e14_event_response, merge_e14_events}`.
- `stellar_archive::default_timeout_config()` (5s per public-archive S3 op).
- 6 `#[ignore]` integration tests validating end-to-end against `aws-public-blockchain` (us-east-2).

## Scope

### 1. Shared state — `AppState`

Create `crates/api/src/state.rs` with:

- `AppState { db_pool: PgPool, stellar_archive: StellarArchiveFetcher }`
- `AppState::from_env() -> Result<Self, ...>` builds both at cold start.
- DB pool via `db::pool::create_pool(&database_url)`. DATABASE_URL resolution
  mirrors `crates/indexer/src/main.rs:22-33` (env var fallback to Secrets Manager
  via `db::secrets::resolve_database_url` when `SECRET_ARN`+`RDS_PROXY_ENDPOINT` set).
- S3 client: `aws_config::defaults(BehaviorVersion::latest()).no_credentials().region("us-east-2").timeout_config(default_timeout_config()).load().await` — **region must be us-east-2** (verified during 0150).
- Add `db = { path = "../db", features = ["aws-secrets"] }` + `sqlx` to `crates/api/Cargo.toml`.

Inject into axum router via `Router::with_state(app_state)`. Keep existing
`/health` path stateless by declaring `State<AppState>` only on new handlers.
Tests: use `connect_lazy` for DB so unit tests don't need a live Postgres.

### 2. E3 handler — `GET /transactions/:hash`

File: `crates/api/src/handlers/transactions.rs` (new module).

Flow:

1. Hex-decode `:hash` path param (64 chars → 32 bytes). Error → `ErrorEnvelope { code: "invalid_hash" }` (400).
2. `SELECT hash, ledger_sequence, created_at FROM transaction_hash_index WHERE hash = $1` (PK lookup, no partition scan).
3. If not found → `ErrorEnvelope { code: "not_found" }` (404).
4. Query `transactions` + JOINs to fill the light view:
   - `transactions.id, application_order, fee_charged, inner_tx_hash, successful, operation_count, has_soroban, parse_error`.
   - JOIN `accounts` on `source_id` → StrKey `account_id`.
   - For operations/events/invocations: nested queries with JOINs to `accounts` (source_id, destination_id, asset_issuer_id, transfer_from_id, transfer_to_id, caller_id) and `soroban_contracts` (contract_id surrogate → StrKey — per ADR 0030).
5. Call `state.stellar_archive.fetch_ledger(ledger_sequence).await`.
6. On `Ok(meta)`: `extract_e3_heavy(&meta, &tx_hash_hex)` → `Some(heavy)`; then `merge_e3_response(light, Some(heavy))`.
7. On `Err(FetchError::*)`: `merge_e3_response(light, None)` — graceful degradation (200 with `heavy_fields_status: "unavailable"`).
8. `#[utoipa::path]` annotation on handler; register via `routes!(handler)`.

### 3. E14 handler — `GET /contracts/:id/events`

File: `crates/api/src/handlers/contracts.rs` (new module).

Flow:

1. Path param `:id` validation (56-char C… StrKey). Error → `invalid_contract_id`.
2. `SELECT id FROM soroban_contracts WHERE contract_id = $1` (StrKey → surrogate BIGINT).
3. If not found → 404.
4. Query params: `limit` (u32, default 25, clamp ≤100), `cursor` (opaque string encoding `(created_at, id)` tuple).
5. Page `soroban_events` by composite key `(created_at DESC, id DESC)` WHERE `contract_id = $surrogate` + cursor filter. JOIN `accounts` for `transfer_from_id`/`transfer_to_id` → StrKey.
6. Collect unique `ledger_sequence` from the page → `state.stellar_archive.fetch_ledgers(&unique_seqs).await`.
7. For each successful ledger: `extract_e14_heavy(&meta, &contract_id_strkey)` → `Vec<E14HeavyEventFields>`. Flatten all successful extractions.
8. `merge_e14_events(db_light, all_heavy, |e| (e.transaction_hash.clone(), e.event_index))`. Events whose ledger fetch failed get `heavy_fields_status: "unavailable"`.
9. Return `{ data: [...merged], page: { cursor, limit, has_more } }`.

### 4. Response DTOs (OpenAPI schemas)

In `crates/api/src/openapi/schemas.rs` (existing module):

- `TransactionLight` — DB light view (hash hex, ledger_sequence, created_at, application_order, source_account StrKey, fee_charged, inner_tx_hash, successful, operation_count, has_soroban, parse_error, operations[], soroban_events[], soroban_invocations[]).
- `OperationLight`, `SorobanEventLight`, `SorobanInvocationLight` — nested light rows with resolved StrKeys.
- Compose into `TransactionDetail = E3Response<TransactionLight>` for OpenAPI docs (flatten via `#[serde(flatten)]` already in `E3Response`).
- `ContractEventLight` + `ContractEventsPage = { data: Vec<E14EventResponse<ContractEventLight>>, page: PageInfo }`.

All derive `utoipa::ToSchema` so they render in OpenAPI JSON.

### 5. Error responses

Use existing `ErrorEnvelope` + `PageInfo` schemas (already in openapi). Cases:

- 400 `invalid_hash`, `invalid_contract_id`, `invalid_cursor`
- 404 `not_found`
- 200 with `heavy_fields_status: "unavailable"` on upstream timeout/404 from public archive

### 6. Golden-fixture integration test (completes 0150 AC #4)

`crates/api/tests/e3_e14_golden.rs`:

- `#[ignore]` test hitting a local instance of the router with a known fixture tx hash + contract id present in a pinned ledger.
- Assert response JSON shape + key fields (heavy_fields_status == "ok", non-empty signatures/events, etc.).
- Use `connect_lazy` DB + a deployed staging DB snapshot OR tower service mocking.

## Out of scope

- Cache layer (deferred; spawn dedicated task only if measured p95 unacceptable)
- CloudWatch metric emission (tracing spans already in 0150 library; metrics pipeline is a separate concern)
- Additional endpoints (E1, E2, E4-E13, E15-E22) — DB-only, separate tasks
- Enum SMALLINT migration (0152 is independent; if it lands first, handlers bind the enum type directly instead of VARCHAR)

## Dependencies / ordering

- **Requires**: 0150 (library) — ✅ done
- **Requires**: 0149 (write path populates DB) — ✅ done
- **Independent of**: 0151 (contracts surrogate) — ✅ landed; handlers use StrKey↔surrogate resolver
- **Orthogonal to**: 0152 (enum SMALLINT) — if 0152 lands first, swap VARCHAR binds for enum binds; no blocker either way

## Acceptance Criteria

- [ ] `AppState` built at cold start with `PgPool` + `StellarArchiveFetcher`; passed to router via `with_state`.
- [ ] `GET /transactions/:hash` returns merged DB light + XDR heavy, with graceful degradation on upstream failure.
- [ ] `GET /contracts/:id/events` returns paginated merged events with full `topics` + `data`; partial-failure events degrade per-row.
- [ ] StrKey ↔ surrogate BIGINT resolution works for both `accounts.id` and `soroban_contracts.id`.
- [ ] Cursor pagination on E14 — composite key `(created_at DESC, id DESC)` with opaque cursor round-trip.
- [ ] `ErrorEnvelope` used for all 4xx paths; never 500 on upstream archive failures.
- [ ] OpenAPI spec exposes `/transactions/{hash}` and `/contracts/{id}/events` with full schemas.
- [ ] Golden-fixture integration test passes against a known tx hash and contract (completes 0150 AC #4).
- [ ] `npx nx run rust:build`, `rust:test`, `rust:lint` pass.

## Open questions

1. **DB pool size**: indexer uses `max_connections(1)` (RDS Proxy constraint). Does API need the same, or can it run more connections per Lambda instance?
2. **Cursor format**: base64-encoded JSON `{"created_at":"...","id":N}` or a signed HMAC to prevent tampering?
3. **Contract-surrogate cache**: if many requests hit the same contract, cache `StrKey → id` in-memory to skip the resolver query? Benchmark first.
