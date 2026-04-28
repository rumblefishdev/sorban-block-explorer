//! End-to-end integration tests for the task 0043 shared helpers.
//!
//! Exercises `GET /v1/transactions` through the real app router with the
//! shared `Pagination<TsIdCursor>` extractor, `filters::strkey` /
//! `filters::parse_enum` validators, `finalize_ts_id_page` + `into_envelope`
//! wire assembly, and the `errors::*` envelope builders. DB-touching tests
//! skip cleanly when `DATABASE_URL` is unset or unreachable — validation
//! tests run unconditionally because they short-circuit before any SQL
//! executes.
//!
//! Run locally against the compose stack:
//!
//!   docker compose up -d
//!   npm run db:migrate
//!   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
//!       cargo test -p api --bin api tests_integration -- --test-threads=1

use axum::Router;
use axum::body::{self, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use utoipa_axum::router::OpenApiRouter;

use crate::contracts;
use crate::state::AppState;
use crate::stellar_archive::StellarArchiveFetcher;
use crate::transactions;

/// Build a test app with the transactions router mounted at /v1.
///
/// Caller supplies the `PgPool`. Validation tests that never touch the DB
/// pass `connect_lazy("...")` (free until first query), DB-gated tests
/// pass a real `PgPool::connect(...)` result.
fn build_app(db: PgPool) -> Router {
    let aws_cfg = aws_sdk_s3::config::Builder::new()
        .region(aws_sdk_s3::config::Region::new("us-east-2"))
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(aws_sdk_s3::config::Credentials::new(
            "test-access-key",
            "test-secret-key",
            None,
            None,
            "tests_integration",
        ))
        .timeout_config(crate::stellar_archive::default_timeout_config())
        .build();
    let s3 = aws_sdk_s3::Client::from_conf(aws_cfg);
    let fetcher = StellarArchiveFetcher::new(s3);
    let contract_cache = crate::contracts::cache::ContractMetadataCache::new();
    let state = AppState {
        db,
        fetcher,
        contract_cache,
        network_id: xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE),
    };

    let (router, _spec) = OpenApiRouter::new()
        .nest("/v1", transactions::router())
        .nest("/v1", contracts::router())
        .with_state(state)
        .split_for_parts();
    router
}

/// Convenience wrapper for validation tests that never hit the DB.
fn lazy_app() -> Router {
    let db = sqlx::PgPool::connect_lazy("postgres://localhost/test_unused")
        .expect("connect_lazy never fails");
    build_app(db)
}

async fn body_json(resp: axum::response::Response) -> (StatusCode, Value) {
    let status = resp.status();
    let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// ---------------------------------------------------------------------------
// Validation tests — no DB contact, run unconditionally.
//
// These prove that the shared `Pagination` extractor, `filters::strkey`,
// and `filters::parse_enum` short-circuit before any SQL executes, and
// return the canonical `ErrorEnvelope` for each failure code. They are
// the end-to-end counterpart to the unit tests in `common::*::tests` —
// the unit tests cover the helpers in isolation; these prove they fire
// through the real axum request plumbing when wired into the
// transactions handler.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_limit_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_limit");
    assert_eq!(json["details"]["received"], "abc");
}

#[tokio::test]
async fn invalid_cursor_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?cursor=not!!base64")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_cursor");
}

#[tokio::test]
async fn invalid_strkey_filter_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?filter%5Bsource_account%5D=BAD")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_filter");
    assert_eq!(json["details"]["filter"], "source_account");
}

#[tokio::test]
async fn invalid_operation_type_filter_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?filter%5Boperation_type%5D=NOT_A_TYPE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_filter");
    assert_eq!(json["details"]["filter"], "operation_type");
}

// ---------------------------------------------------------------------------
// DB-touching test — gated on DATABASE_URL.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_endpoint_returns_paginated_envelope_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping list envelope integration test");
        return;
    };

    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping list envelope integration test");
            return;
        }
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");

    // Envelope shape asserted regardless of row count — empty DB is fine.
    assert!(
        json.get("data").is_some(),
        "envelope missing `data`: {json}"
    );
    assert!(json["data"].is_array(), "data not array: {json}");
    let page = &json["page"];
    assert_eq!(page["limit"], 3, "page.limit not echoed: {json}");
    assert!(
        page["has_more"].is_boolean(),
        "page.has_more not bool: {json}"
    );
    // `cursor` is `Option<String>` with `skip_serializing_if = Option::is_none`
    // on the empty-page case — either a string or absent is valid.
    if let Some(c) = page.get("cursor") {
        assert!(c.is_string() || c.is_null(), "page.cursor bad type: {json}");
    }
}

/// Full request → response → next cursor → request chain.
///
/// Asserts that page 2 returned by feeding the page-1 cursor back into the
/// extractor:
///   * has no overlap with page 1 (different `hash` set), and
///   * is correctly bounded — `has_more` flips to false at the tail, or the
///     cursor advances monotonically when more pages remain.
///
/// Skips cleanly when DB is unavailable or has fewer than 2 rows (cannot
/// validate continuation on an empty / single-row table).
#[tokio::test]
async fn cursor_round_trip_no_overlap_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping cursor round-trip test");
        return;
    };

    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping cursor round-trip test");
            return;
        }
    };

    // Page 1: limit=1 to maximise the chance of has_more=true on small DBs.
    let router = build_app(pool.clone());
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, page1) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "page1 status: {status} body {page1}"
    );

    let data1 = page1["data"].as_array().expect("data array").clone();
    if data1.is_empty() || page1["page"]["has_more"] != true {
        eprintln!("DB has <2 transactions — skipping continuation assertions");
        return;
    }
    let cursor = page1["page"]["cursor"]
        .as_str()
        .expect("page.cursor present when has_more=true")
        .to_string();
    let hash1 = data1[0]["hash"].as_str().unwrap().to_string();

    // Page 2: feed cursor back. Cursor is base64url *unpadded* (URL-safe alphabet, no `=`),
    // so raw interpolation into the query string is safe — no percent-encoding needed.
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/transactions?limit=1&cursor={cursor}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, page2) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "page2 status: {status} body {page2}"
    );

    let data2 = page2["data"].as_array().expect("data array").clone();
    if let Some(first) = data2.first() {
        let hash2 = first["hash"].as_str().unwrap();
        assert_ne!(
            hash1, hash2,
            "page2 first row overlaps page1 — cursor predicate broken"
        );
    }
    // page2.cursor either advances to a new value or is absent on tail.
    if let Some(next) = page2["page"]["cursor"].as_str() {
        assert_ne!(
            next, cursor,
            "page2 cursor identical to page1 — no progress"
        );
    }
}

// ---------------------------------------------------------------------------
// Graceful-degradation tests (task 0044 §6).
//
// Lock the wire-level invariant that no endpoint returns 5xx purely because
// ingestion is behind the network tip. Concretely:
//
//   * Missing-resource lookups (hash not yet indexed, contract not yet
//     indexed) must surface as 404 with a `not_found` envelope, never 500.
//   * Upstream public-archive (S3) outages must degrade XDR-derived fields
//     to null with the parent response still 200; the endpoint must not
//     surface the underlying error to the client.
//   * Malformed input that short-circuits before the DB still maps to 400
//     with the canonical envelope code (no panic, no 500).
//
// These complement the per-record degradation tests in 0046's S3-gated
// suite (`extract_e3_*`) by exercising the full handler chain end-to-end.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn detail_invalid_hash_format_returns_400_before_db() {
    // Short / non-hex hash short-circuits before any DB or S3 call. Locks in
    // the pre-DB validation branch so a future refactor cannot start
    // forwarding malformed hashes into `lookup_hash_index` and 500-ing on
    // the SQL bind.
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/transactions/notahash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_hash");
}

#[tokio::test]
async fn detail_unknown_hash_returns_404_not_500() {
    // The "ledger 60M+1 not yet indexed" scenario — well-formed hash, no row
    // in `transactions`. The handler must surface this as 404 with the
    // `not_found` envelope, never 500. This is the literal invariant
    // documented in ADR 0008 + spec §"Graceful Degradation": missing recent
    // data is normal, not an error condition.
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping detail-unknown-hash test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping detail-unknown-hash test");
            return;
        }
    };

    // 64 hex chars, all zeros — guaranteed to not match any real ledger.
    let unknown_hash = "0".repeat(64);

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/transactions/{unknown_hash}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404, got {status}: {json}"
    );
    assert_eq!(json["code"], "not_found");
    assert!(
        json.get("error").is_none(),
        "envelope must be flat (ADR 0008): {json}"
    );
}

#[tokio::test]
async fn list_with_unreachable_s3_returns_200_with_degraded_memo() {
    // The fake AWS credentials in `build_app` mean every public-archive
    // fetch fails. The list handler must still return 200 with degraded
    // memo fields (None) for any rows that happen to be in the DB —
    // never 500. Skips when the DB is empty (cannot prove degradation
    // without rows to enrich).
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping list-degraded-memo test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping list-degraded-memo test");
            return;
        }
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "list must stay 200 even when S3 is unreachable: {status} {json}"
    );

    let data = json["data"].as_array().expect("data array").clone();
    if data.is_empty() {
        eprintln!("DB empty — cannot assert per-row memo degradation, skipping");
        return;
    }

    // Every row must serialise; memo / memo_type are allowed to be null
    // (degraded) but the row itself must be present and well-formed.
    for row in &data {
        assert!(row["hash"].is_string(), "row missing hash: {row}");
        // memo_type / memo are Option<String> with skip_serializing_if on the
        // None branch — either absent or null is valid for the degraded path.
        if let Some(mt) = row.get("memo_type") {
            assert!(mt.is_null() || mt.is_string(), "memo_type bad shape: {row}");
        }
    }
}

// ---------------------------------------------------------------------------
// Contracts handlers — graceful-degradation regression coverage (task 0044 §6).
//
// Mirror the transactions tests for /v1/contracts/:id{,/interface,/invocations,
// /events}. The contracts module ships its own ListParams parser + S3
// stop-and-retry expansion; these tests lock that no path returns 5xx for
// missing-resource or malformed-input scenarios. A future refactor that, e.g.,
// flips `Ok(None) => not_found` to `internal_error` or starts forwarding bad
// StrKey paths into the SQL bind will fail one of these tests.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contract_invalid_id_returns_400_before_db() {
    // Malformed StrKey (lowercase, wrong length) short-circuits before any DB
    // hit. Locks the pre-DB validation branch in `get_contract`.
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/contracts/notavalidstrkey")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_contract_id");
    assert_eq!(json["details"]["param"], "contract_id");
    assert_eq!(json["details"]["expected_prefix"], "C");
}

#[tokio::test]
async fn contract_invocations_invalid_id_returns_400_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/contracts/notavalidstrkey/invocations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_contract_id");
}

#[tokio::test]
async fn contract_events_invalid_id_returns_400_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/contracts/notavalidstrkey/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_contract_id");
}

#[tokio::test]
async fn contract_unknown_id_returns_404_not_500() {
    // Well-formed StrKey, no row in `soroban_contracts`. Equivalent of
    // `detail_unknown_hash_returns_404_not_500` for the contracts route.
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping contract-unknown-id test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping contract-unknown-id test");
            return;
        }
    };

    // Synthetic 56-char StrKey (no CRC) guaranteed not to exist.
    let unknown_contract = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/contracts/{unknown_contract}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404, got {status}: {json}"
    );
    assert_eq!(json["code"], "not_found");
}

#[tokio::test]
async fn contract_interface_unknown_returns_404() {
    // No `wasm_interface_metadata` row for the contract → 404.
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping interface-unknown test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping interface-unknown test");
            return;
        }
    };

    let unknown_contract = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/contracts/{unknown_contract}/interface"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404, got {status}: {json}"
    );
    assert_eq!(json["code"], "not_found");
}

// ---------------------------------------------------------------------------
// Out-of-u32-range `ledger_sequence` — pure logic test (no fixture row needed).
//
// Stellar `LedgerHeader.ledgerSeq` is `uint32` so any DB row with
// `ledger_sequence > u32::MAX` indicates corrupted ingestion or a
// hypothetical schema drift. The handler responds by skipping the row from
// memo enrichment / heavy fetch and logging a `warn`, never panicking.
// Seeding such a row in PG is unrealistic (would require a deliberate
// out-of-bound BIGINT), so we lock the conversion behaviour at the type
// boundary instead.
// ---------------------------------------------------------------------------

#[test]
fn out_of_u32_range_ledger_sequence_fails_conversion_safely() {
    // Inputs the handler converts via `u32::try_from(i64)`:
    assert!(
        u32::try_from(i64::MAX).is_err(),
        "i64::MAX must overflow u32"
    );
    assert!(
        u32::try_from(i64::from(u32::MAX) + 1).is_err(),
        "u32::MAX + 1 must overflow"
    );
    assert!(u32::try_from(-1_i64).is_err(), "negative must fail");

    // Boundary: u32::MAX itself fits.
    assert_eq!(u32::try_from(i64::from(u32::MAX)).unwrap(), u32::MAX);

    // The handler's pattern: failed conversion → warn + skip / heavy=None,
    // not panic. Verified by the call sites in
    // `transactions/handlers.rs::list_transactions` (memo enrichment loop),
    // `transactions/handlers.rs::get_transaction` (heavy fetch),
    // and `contracts/handlers.rs::expand_invocations` / `expand_events`
    // (per-row stop-and-retry).
}
