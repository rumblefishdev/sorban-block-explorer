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

use crate::state::AppState;
use crate::stellar_archive::StellarArchiveFetcher;
use crate::{liquidity_pools, transactions};

/// Build a test app with the transactions and liquidity-pools routers
/// mounted at /v1.
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
        .nest("/v1", liquidity_pools::router())
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
// Task 0126 — liquidity-pool participants endpoint
//
// Validation tests run unconditionally (short-circuit before any SQL).
// The end-to-end test seeds a pool + accounts + LP positions, hits the
// endpoint, and tears down — gated on `DATABASE_URL` so it skips
// cleanly when no Postgres is available.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lp_participants_invalid_pool_id_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/liquidity-pools/not-hex/participants")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_pool_id");
}

#[tokio::test]
async fn lp_participants_invalid_limit_returns_envelope_before_db() {
    let app = lazy_app();
    // Well-formed pool_id (64 hex), bad limit — extractor short-circuits.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(
                    "/v1/liquidity-pools/\
                     0000000000000000000000000000000000000000000000000000000000000000/\
                     participants?limit=abc",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_limit");
}

#[tokio::test]
async fn lp_participants_invalid_cursor_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri(
                    "/v1/liquidity-pools/\
                     0000000000000000000000000000000000000000000000000000000000000000/\
                     participants?cursor=not!!base64",
                )
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
async fn lp_participants_404_for_missing_pool() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0126 missing-pool 404 test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0126 missing-pool 404 test");
            return;
        }
    };
    let app = build_app(pool);

    // Synthetic pool_id that won't exist on a clean DB.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(
                    "/v1/liquidity-pools/\
                     deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef/\
                     participants",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {json}");
    assert_eq!(json["code"], "not_found");
}

/// End-to-end: seed (pool, 3 accounts, 3 LP positions including one
/// zero-share row), call the endpoint twice for cursor round-trip, then
/// tear down. Asserts:
///
///   * 200 with `Paginated<ParticipantItem>` envelope
///   * Order: shares DESC
///   * Zero-share row filtered out
///   * Cursor round-trip yields disjoint pages
#[tokio::test]
async fn lp_participants_e2e_sort_filter_pagination() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0126 e2e test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0126 e2e test");
            return;
        }
    };

    // Distinct from any in-flight indexer test fixtures (TEST_POOL_ID
    // 3333…, SAC160_*) so the seed/teardown does not collide.
    const POOL_HEX: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const ACC_TOP: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA0126TOP";
    const ACC_MID: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA0126MID";
    const ACC_ZERO: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA0126ZRO";

    // Idempotent setup — clear any prior run leftovers first.
    teardown_lp_e2e_fixture(&pool, POOL_HEX, &[ACC_TOP, ACC_MID, ACC_ZERO]).await;
    setup_lp_e2e_fixture(&pool, POOL_HEX, ACC_TOP, ACC_MID, ACC_ZERO).await;

    let app = build_app(pool.clone());

    // -- Page 1: limit=1, expect ACC_TOP (highest shares = "100.0000000")
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/liquidity-pools/{POOL_HEX}/participants?limit=1"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, page1) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "page1 body: {page1}");
    let data1 = page1["data"].as_array().expect("data array").clone();
    assert_eq!(data1.len(), 1, "page1 should have exactly limit rows");
    assert_eq!(data1[0]["account"], ACC_TOP, "highest-shares account first");
    assert_eq!(data1[0]["shares"], "100.0000000");
    // share_percentage = 100 / 200 * 100 = 50.0 (snapshot total_shares=200).
    // PG NUMERIC division retains generous precision; assert by parsed
    // numeric rather than exact string to insulate against PG version
    // drift in the divisor's scale calculation.
    let pct_top: f64 = data1[0]["share_percentage"]
        .as_str()
        .expect("share_percentage present when snapshot is fresh")
        .parse()
        .expect("share_percentage parses as numeric");
    assert!(
        (pct_top - 50.0).abs() < 1e-9,
        "expected ~50.0%, got {pct_top}"
    );
    assert_eq!(
        page1["page"]["has_more"], true,
        "second page must exist (3rd row is filtered, 2nd remains)"
    );
    let cursor = page1["page"]["cursor"]
        .as_str()
        .expect("cursor present when has_more=true")
        .to_string();

    // -- Page 2: feed cursor, expect ACC_MID (50). ACC_ZERO must NOT appear.
    let app = build_app(pool.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/liquidity-pools/{POOL_HEX}/participants?limit=1&cursor={cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, page2) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "page2 body: {page2}");
    let data2 = page2["data"].as_array().expect("data array").clone();
    assert_eq!(data2.len(), 1);
    assert_eq!(data2[0]["account"], ACC_MID, "mid-shares account second");
    assert_eq!(data2[0]["shares"], "50.0000000");
    // share_percentage = 50 / 200 * 100 = 25.0
    let pct_mid: f64 = data2[0]["share_percentage"]
        .as_str()
        .expect("share_percentage present when snapshot is fresh")
        .parse()
        .expect("share_percentage parses as numeric");
    assert!(
        (pct_mid - 25.0).abs() < 1e-9,
        "expected ~25.0%, got {pct_mid}"
    );
    // Tail flag — third row is zero-shares, filtered out, so no page 3.
    assert_eq!(
        page2["page"]["has_more"], false,
        "zero-share row must be filtered → page2 is the tail"
    );

    // -- Confirm zero-shares account is never returned even when paged
    // through to the end without limit.
    let app = build_app(pool.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/liquidity-pools/{POOL_HEX}/participants?limit=100"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (_, all) = body_json(resp).await;
    let accounts: Vec<&str> = all["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["account"].as_str().unwrap())
        .collect();
    assert_eq!(accounts, vec![ACC_TOP, ACC_MID]);
    assert!(
        !accounts.contains(&ACC_ZERO),
        "zero-share row must be filtered: {accounts:?}"
    );

    teardown_lp_e2e_fixture(&pool, POOL_HEX, &[ACC_TOP, ACC_MID, ACC_ZERO]).await;
}

async fn setup_lp_e2e_fixture(
    pool: &PgPool,
    pool_hex: &str,
    acc_top: &str,
    acc_mid: &str,
    acc_zero: &str,
) {
    // Pool — minimal native↔credit shape, no FK to issuer (issuer_id NULL
    // for native means asset_a_type=0; asset_b is a synthetic credit).
    sqlx::query(
        r#"
        INSERT INTO liquidity_pools (
            pool_id, asset_a_type, asset_a_code, asset_a_issuer_id,
            asset_b_type, asset_b_code, asset_b_issuer_id,
            fee_bps, created_at_ledger
        ) VALUES (decode($1, 'hex'), 0, NULL, NULL, 1, '0126TKN', NULL, 30, 1)
        "#,
    )
    .bind(pool_hex)
    .execute(pool)
    .await
    .expect("insert pool");

    // Accounts (need surrogate ids for lp_positions FK).
    let acc_top_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number)
           VALUES ($1, 1, 1, 0) RETURNING id"#,
    )
    .bind(acc_top)
    .fetch_one(pool)
    .await
    .expect("insert acc_top");
    let acc_mid_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number)
           VALUES ($1, 1, 1, 0) RETURNING id"#,
    )
    .bind(acc_mid)
    .fetch_one(pool)
    .await
    .expect("insert acc_mid");
    let acc_zero_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number)
           VALUES ($1, 1, 1, 0) RETURNING id"#,
    )
    .bind(acc_zero)
    .fetch_one(pool)
    .await
    .expect("insert acc_zero");

    // LP positions: top=100, mid=50, zero=0 (must be filtered by API).
    sqlx::query(
        r#"
        INSERT INTO lp_positions (pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger)
        VALUES
            (decode($1, 'hex'), $2, 100.0::NUMERIC(28,7), 1, 1),
            (decode($1, 'hex'), $3,  50.0::NUMERIC(28,7), 1, 1),
            (decode($1, 'hex'), $4,   0.0::NUMERIC(28,7), 1, 1)
        "#,
    )
    .bind(pool_hex)
    .bind(acc_top_id)
    .bind(acc_mid_id)
    .bind(acc_zero_id)
    .execute(pool)
    .await
    .expect("insert lp_positions");

    // Snapshot row — total_shares = 200 so the canonical query's
    // `share_percentage` CTE has a fresh divisor. `created_at = NOW()`
    // lands in the live `_default` partition and is well within the
    // 7-day freshness window the spec uses.
    sqlx::query(
        r#"
        INSERT INTO liquidity_pool_snapshots (
            pool_id, ledger_sequence, reserve_a, reserve_b, total_shares, created_at
        )
        VALUES (decode($1, 'hex'), 1, 1000.0, 2000.0, 200.0, NOW())
        "#,
    )
    .bind(pool_hex)
    .execute(pool)
    .await
    .expect("insert liquidity_pool_snapshots");
}

async fn teardown_lp_e2e_fixture(pool: &PgPool, pool_hex: &str, accounts: &[&str]) {
    let _ = sqlx::query("DELETE FROM liquidity_pool_snapshots WHERE pool_id = decode($1, 'hex')")
        .bind(pool_hex)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM lp_positions WHERE pool_id = decode($1, 'hex')")
        .bind(pool_hex)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM liquidity_pools WHERE pool_id = decode($1, 'hex')")
        .bind(pool_hex)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = ANY($1)")
        .bind(accounts)
        .execute(pool)
        .await;
}
