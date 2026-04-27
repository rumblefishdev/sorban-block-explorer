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
use crate::transactions;

/// Build a minimal test app with the transactions router mounted at /v1.
/// Uses `connect_lazy` so tests that never hit the DB pay no connection cost.
fn lazy_app() -> Router {
    let db = sqlx::PgPool::connect_lazy("postgres://localhost/test_unused")
        .expect("connect_lazy never fails");
    let aws_cfg = aws_sdk_s3::config::Builder::new()
        .region(aws_sdk_s3::config::Region::new("us-east-2"))
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .build();
    let s3 = aws_sdk_s3::Client::from_conf(aws_cfg);
    let fetcher = StellarArchiveFetcher::new(s3);
    let state = AppState { db, fetcher };

    let (router, _spec) = OpenApiRouter::new()
        .nest("/v1", transactions::router())
        .with_state(state)
        .split_for_parts();
    router
}

fn real_app(db: PgPool) -> Router {
    let aws_cfg = aws_sdk_s3::config::Builder::new()
        .region(aws_sdk_s3::config::Region::new("us-east-2"))
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .build();
    let s3 = aws_sdk_s3::Client::from_conf(aws_cfg);
    let fetcher = StellarArchiveFetcher::new(s3);
    let state = AppState { db, fetcher };

    let (router, _spec) = OpenApiRouter::new()
        .nest("/v1", transactions::router())
        .with_state(state)
        .split_for_parts();
    router
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

    let app = real_app(pool);
    let resp = app
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
    let app = real_app(pool.clone());
    let resp = app
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

    // Page 2: feed cursor back. URL-encode the `=` padding-free base64url.
    let app = real_app(pool);
    let resp = app
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
