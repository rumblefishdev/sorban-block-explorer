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

use crate::assets;
use crate::contracts;
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
        .nest("/v1", contracts::router())
        .nest("/v1", liquidity_pools::router())
        .nest("/v1", assets::router())
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

/// Locks the JOIN to `operations_appearances` — the no-filter list test
/// never hits this branch.
#[tokio::test]
async fn list_endpoint_filter_op_type_returns_envelope_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=3&filter%5Boperation_type%5D=PAYMENT")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
    assert!(json["data"].is_array(), "data not array: {json}");
    assert_eq!(json["page"]["limit"], 3, "page.limit not echoed: {json}");
    assert!(
        json["page"]["has_more"].is_boolean(),
        "page.has_more not bool: {json}"
    );
}

/// Locks `fetch_operations` against `operations_appearances` with
/// `ORDER BY o.id` — pre-existing detail tests only cover 404.
#[tokio::test]
async fn detail_endpoint_returns_200_for_known_hash_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT encode(hash, 'hex') FROM transaction_hash_index LIMIT 1")
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
    let Some((hash_hex,)) = row else {
        eprintln!("transaction_hash_index empty — skipping successful-detail test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/transactions/{hash_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
    assert!(
        json["operations"].is_array(),
        "detail.operations not array: {json}"
    );

    assert!(
        json["application_order"].is_number(),
        "application_order missing/non-number: {json}"
    );
    assert!(
        json["has_soroban"].is_boolean(),
        "has_soroban missing/non-bool: {json}"
    );
    assert!(
        json["operation_count"].is_number(),
        "operation_count missing/non-number: {json}"
    );
    assert!(
        json["inner_tx_hash"].is_string() || json["inner_tx_hash"].is_null(),
        "inner_tx_hash bad type: {json}"
    );

    if let Some(op) = json["operations"].as_array().and_then(|a| a.first()) {
        assert!(
            op["appearance_id"].is_number(),
            "operations[0].appearance_id missing/non-number: {op}"
        );
        assert!(
            op["type_name"].is_string(),
            "operations[0].type_name missing/non-string: {op}"
        );
        assert!(
            op["type"].is_number(),
            "operations[0].type missing/non-number: {op}"
        );
    }
}

#[tokio::test]
async fn detail_endpoint_projects_full_operation_columns_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> = sqlx::query_as(
        "SELECT encode(t.hash, 'hex') \
         FROM transactions t \
         JOIN operations_appearances oa \
              ON oa.transaction_id = t.id AND oa.created_at = t.created_at \
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some((hash_hex,)) = row else {
        eprintln!("no transactions with operations — skipping");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/transactions/{hash_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    let op = json["operations"]
        .as_array()
        .and_then(|a| a.first())
        .expect("at least one op (we joined to operations_appearances)");
    for field in [
        "source_account",
        "destination_account",
        "contract_id",
        "asset_code",
        "asset_issuer",
        "pool_id",
    ] {
        assert!(
            op.get(field).is_some(),
            "operations[0] missing key {field}: {op}"
        );
        assert!(
            op[field].is_string() || op[field].is_null(),
            "operations[0].{field} bad type: {op}"
        );
    }
    assert!(
        op["ledger_sequence"].is_number(),
        "operations[0].ledger_sequence not number: {op}"
    );
    assert!(
        op["created_at"].is_string(),
        "operations[0].created_at not ISO string: {op}"
    );
}

/// `build_app` uses fake S3 credentials so the archive fetch fails and
/// `heavy_fields_status = "unavailable"` exercises the DB fallback path.
#[tokio::test]
async fn detail_endpoint_falls_back_to_db_when_heavy_unavailable() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> = sqlx::query_as(
        "SELECT encode(t.hash, 'hex') \
         FROM transactions t \
         JOIN transaction_participants tp \
              ON tp.transaction_id = t.id AND tp.created_at = t.created_at \
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some((hash_hex,)) = row else {
        eprintln!("no tx with participants — skipping fallback test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/transactions/{hash_hex}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");

    assert_eq!(
        json["heavy_fields_status"], "unavailable",
        "expected heavy unavailable to exercise the fallback path: {json}"
    );
    assert!(
        json["participants"].is_array(),
        "fallback participants[] missing/non-array: {json}"
    );
    assert!(
        json["soroban_events"].is_array(),
        "fallback soroban_events[] missing/non-array: {json}"
    );
    assert!(
        json["soroban_invocations"].is_array(),
        "fallback soroban_invocations[] missing/non-array: {json}"
    );
    // Hash discovered via JOIN to transaction_participants, so fallback
    // must produce ≥1 row — proves the DB query fired.
    let participants = json["participants"].as_array().unwrap();
    assert!(
        !participants.is_empty(),
        "expected ≥1 participant on a tx joined via transaction_participants: {json}"
    );
}

/// Statement B path — broad contract match across the three appearance
/// tables. Asserts filter ↔ projection consistency on the matched contract.
#[tokio::test]
async fn list_endpoint_filter_contract_id_returns_envelope_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT contract_id FROM soroban_contracts ORDER BY id LIMIT 1")
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
    let Some((cid,)) = row else {
        eprintln!("soroban_contracts empty — skipping contract-filter test");
        return;
    };

    let router = build_app(pool);
    let uri = format!("/v1/transactions?limit=3&filter%5Bcontract_id%5D={cid}");
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
    assert!(json["data"].is_array(), "data not array: {json}");
    if let Some(arr) = json["data"].as_array() {
        for (i, row) in arr.iter().enumerate() {
            let ids = row["contract_ids"]
                .as_array()
                .unwrap_or_else(|| panic!("row[{i}].contract_ids not array: {row}"));
            assert!(
                ids.iter().any(|v| v.as_str() == Some(cid.as_str())),
                "row[{i}] missing filtered contract_id={cid} in contract_ids: {row}"
            );
        }
    }
}

/// Statement-B EXISTS post-filter branch (contract + op_type combined).
#[tokio::test]
async fn list_endpoint_filter_contract_id_and_op_type_returns_envelope_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT contract_id FROM soroban_contracts ORDER BY id LIMIT 1")
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
    let Some((cid,)) = row else {
        return;
    };

    let router = build_app(pool);
    let uri = format!(
        "/v1/transactions?limit=3&filter%5Bcontract_id%5D={cid}&filter%5Boperation_type%5D=INVOKE_HOST_FUNCTION"
    );
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
    assert!(json["data"].is_array(), "data not array: {json}");
}

#[tokio::test]
async fn list_endpoint_projects_canonical_columns_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/transactions?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");

    let Some(row) = json["data"].as_array().and_then(|a| a.first()) else {
        eprintln!("transactions empty — skipping projection-shape assertions");
        return;
    };

    assert!(
        row["application_order"].is_number(),
        "application_order missing/non-number: {row}"
    );
    assert!(
        row["has_soroban"].is_boolean(),
        "has_soroban missing/non-bool: {row}"
    );
    assert!(
        row["inner_tx_hash"].is_string() || row["inner_tx_hash"].is_null(),
        "inner_tx_hash bad type: {row}"
    );
    assert!(
        row["operation_types"].is_array(),
        "operation_types not array: {row}"
    );
    assert!(
        row["contract_ids"].is_array(),
        "contract_ids not array: {row}"
    );
}

// ---------------------------------------------------------------------------
// Assets endpoints (task 0049) — mirror the transactions coverage shape.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn assets_invalid_filter_type_returns_envelope_before_db() {
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/assets?filter%5Btype%5D=NOT_A_TYPE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_filter");
    assert_eq!(json["details"]["filter"], "type");
}

/// `filter[code]` must reject SQL wildcard literals (`%`, `_`) so a
/// confused caller can't silently change match semantics through the
/// trigram-substring path.
#[tokio::test]
async fn assets_filter_code_rejects_wildcard_literals() {
    for q in [
        "/v1/assets?filter%5Bcode%5D=USD%25", // %25 = `%`
        "/v1/assets?filter%5Bcode%5D=USD_",
    ] {
        let app = lazy_app();
        let resp = app
            .oneshot(Request::builder().uri(q).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (status, json) = body_json(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "uri={q} json={json}");
        assert_eq!(json["code"], "invalid_filter");
        assert_eq!(json["details"]["filter"], "code");
    }
}

#[tokio::test]
async fn assets_invalid_id_returns_400_envelope() {
    // Not numeric, not a 56-char StrKey, not a code-issuer composite — must
    // fail parsing in the handler before the DB is touched.
    let app = lazy_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/assets/not-an-asset-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_id");
    assert_eq!(json["details"]["received"], "not-an-asset-id");
}

#[tokio::test]
async fn assets_list_returns_paginated_envelope_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping assets list integration test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping");
            return;
        }
    };
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/assets?limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
    assert!(json["data"].is_array(), "data not array: {json}");
    assert_eq!(json["page"]["limit"], 5, "page.limit not echoed: {json}");
    assert!(
        json["page"]["has_more"].is_boolean(),
        "page.has_more not bool: {json}"
    );
}

/// `filter[type]=native` must return at most the singleton native row
/// (seeded by migration `20260428000000_seed_native_asset_singleton`).
#[tokio::test]
async fn assets_filter_type_native_returns_singleton_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/v1/assets?filter%5Btype%5D=native")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    let rows = json["data"].as_array().unwrap();
    // Allow zero (DB without seed) or one — never more than one native asset.
    assert!(rows.len() <= 1, "more than one native asset: {json}");
    if let Some(row) = rows.first() {
        // Canonical SQL projects BOTH the decoded label (asset_type_name)
        // and the raw SMALLINT (asset_type). Lock both contracts so a
        // future drift on either side surfaces here.
        assert_eq!(row["asset_type_name"], "native");
        assert_eq!(row["asset_type"], 0);
        assert!(
            row["asset_code"].is_null(),
            "native must have null asset_code"
        );
    }
}

/// Resolution by numeric `assets.id`. Skips cleanly if the table is
/// completely empty.
#[tokio::test]
async fn assets_detail_by_numeric_id_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    // Find any existing id (the singleton at id=1 always works after the
    // migration; guard regardless so the test stays robust).
    let row: Option<(i32,)> = sqlx::query_as("SELECT id FROM assets ORDER BY id LIMIT 1")
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
    let Some((id,)) = row else {
        eprintln!("assets table empty — skipping numeric-id resolution test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/assets/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    assert_eq!(json["id"], id, "id mismatch: {json}");
    assert!(
        json.get("description").is_some(),
        "detail response must carry the description slot (even if null): {json}"
    );
}

/// 404 path for a numeric id that does not exist.
#[tokio::test]
async fn assets_detail_unknown_id_returns_404_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };
    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                // Use a clearly-absent numeric id; SERIAL never reaches i32::MAX
                // in any realistic backfill.
                .uri("/v1/assets/2147483647")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {json}");
    assert_eq!(json["code"], "not_found");
}

/// `:id` resolution by contract StrKey. Skips when the DB has no SAC or
/// Soroban-native asset row with a non-NULL `contract_id`.
#[tokio::test]
async fn assets_detail_by_contract_strkey_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(i32, String)> = sqlx::query_as(
        "SELECT a.id, sc.contract_id \
         FROM assets a \
         JOIN soroban_contracts sc ON sc.id = a.contract_id \
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some((expected_id, contract_strkey)) = row else {
        eprintln!("no asset with contract_id — skipping contract-StrKey resolution test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/assets/{contract_strkey}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    assert_eq!(json["id"], expected_id, "wrong asset surfaced: {json}");
    assert_eq!(json["contract_id"], contract_strkey);
}

/// `:id` resolution by `code-issuer` composite. Skips when the DB has no
/// classic_credit / SAC-classic-wrap row.
#[tokio::test]
async fn assets_detail_by_code_issuer_composite_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(i32, String, String)> = sqlx::query_as(
        "SELECT a.id, a.asset_code, iss.account_id \
         FROM assets a \
         JOIN accounts iss ON iss.id = a.issuer_id \
         WHERE a.asset_code IS NOT NULL \
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some((expected_id, code, issuer)) = row else {
        eprintln!("no classic-identity asset — skipping code-issuer resolution test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/assets/{code}-{issuer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    assert_eq!(json["id"], expected_id);
    assert_eq!(json["asset_code"], code);
    assert_eq!(json["issuer"], issuer);
}

/// Non-native `/transactions` happy path — picks any non-native asset that
/// actually appears in `operations_appearances` and asserts the page
/// returns at least one tx (proving the per-asset_type predicate composer
/// resolves the right join branch on real data).
#[tokio::test]
async fn assets_transactions_returns_at_least_one_row_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    // Try classic identity first, fall back to contract identity.
    let by_classic: Option<(i32,)> = sqlx::query_as(
        "SELECT a.id FROM assets a \
         JOIN accounts iss ON iss.id = a.issuer_id \
         JOIN operations_appearances oa \
              ON oa.asset_code = a.asset_code AND oa.asset_issuer_id = iss.id \
         LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let by_contract: Option<(i32,)> = if by_classic.is_none() {
        sqlx::query_as(
            "SELECT a.id FROM assets a \
             JOIN soroban_contracts sc ON sc.id = a.contract_id \
             JOIN operations_appearances oa ON oa.contract_id = sc.id \
             LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    let Some((asset_id,)) = by_classic.or(by_contract) else {
        eprintln!(
            "no non-native asset references found in operations_appearances — \
             skipping happy-path /transactions assertion"
        );
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/assets/{asset_id}/transactions?limit=5"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    let data = json["data"].as_array().unwrap();
    assert!(
        !data.is_empty(),
        "asset {asset_id} appears in operations_appearances but \
         /transactions returned 0 rows: {json}"
    );
    // Lock the canonical-aligned response shape: every row must carry
    // `has_soroban` (bool) and `operation_types` (string[]) — these are
    // the §6.9 fields canonical 10_get_assets_transactions.sql projects.
    let first = &data[0];
    assert!(
        first["has_soroban"].is_boolean(),
        "has_soroban missing or not bool: {first}"
    );
    assert!(
        first["operation_types"].is_array(),
        "operation_types missing or not array: {first}"
    );
}

/// Native XLM has no DB-side identity referenced by `operations_appearances`
/// — the sub-resource short-circuits to an empty page rather than emit a
/// degenerate `WHERE ()` SQL. Lock the contract here.
#[tokio::test]
async fn assets_native_transactions_returns_empty_page_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    // Native singleton is asset_type=0; resolve its id rather than hard-coding.
    let row: Option<(i32,)> = sqlx::query_as("SELECT id FROM assets WHERE asset_type = 0 LIMIT 1")
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
    let Some((native_id,)) = row else {
        eprintln!("no native asset row — skipping");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/assets/{native_id}/transactions"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");
    assert_eq!(
        json["data"].as_array().unwrap().len(),
        0,
        "native asset must produce empty transactions page: {json}"
    );
    assert_eq!(json["page"]["has_more"], false);
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

// Contracts E10 detail (task 0172) — canonical shape lock per `11_*.sql`.
// ---------------------------------------------------------------------------

/// Asserts that `GET /v1/contracts/:id` returns every canonical-aligned
/// field name (post-task-0172): `wasm_uploaded_at_ledger`, `deployer` (not
/// `deployer_account`), `contract_type_name` + raw `contract_type` SMALLINT,
/// and the bounded-window `stats` trio (`recent_invocations`,
/// `recent_unique_callers`, `stats_window` echoed back).
///
/// Skips cleanly if the local DB has no soroban_contracts rows.
#[tokio::test]
async fn contracts_detail_returns_canonical_shape_against_real_db() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let Ok(pool) = PgPool::connect(&database_url).await else {
        return;
    };

    let row: Option<(String,)> =
        sqlx::query_as("SELECT contract_id FROM soroban_contracts ORDER BY id LIMIT 1")
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
    let Some((cid,)) = row else {
        eprintln!("no soroban_contracts rows — skipping contracts E10 shape test");
        return;
    };

    let router = build_app(pool);
    let resp = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/contracts/{cid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {json}");

    // Canonical field names — these would all fail on the pre-0172 shape.
    assert_eq!(json["contract_id"], cid);
    assert!(
        json.get("wasm_uploaded_at_ledger").is_some(),
        "missing wasm_uploaded_at_ledger: {json}"
    );
    assert!(
        json.get("deployer").is_some(),
        "missing `deployer` (post-rename from `deployer_account`): {json}"
    );
    assert!(
        json.get("contract_type_name").is_some(),
        "missing decoded `contract_type_name`: {json}"
    );
    assert!(
        json["contract_type"].is_i64() || json["contract_type"].is_null(),
        "`contract_type` must be raw SMALLINT (or null), got: {json}"
    );

    // Bounded-window stats trio. The window MUST be the API-side const
    // (`7 days`) so the frontend can render the label without guessing.
    let stats = &json["stats"];
    assert!(
        stats["recent_invocations"].is_i64(),
        "stats.recent_invocations not int: {json}"
    );
    assert!(
        stats["recent_unique_callers"].is_i64(),
        "stats.recent_unique_callers not int: {json}"
    );
    assert_eq!(
        stats["stats_window"], "7 days",
        "stats.stats_window must echo the API default: {json}"
    );

    // The pre-0172 shape would carry these — make sure they're gone.
    assert!(
        json.get("deployer_account").is_none(),
        "stale field deployer_account leaked: {json}"
    );
    assert!(
        stats.get("invocation_count").is_none(),
        "stale field stats.invocation_count leaked: {json}"
    );
    assert!(
        stats.get("event_count").is_none(),
        "stale field stats.event_count leaked: {json}"
    );
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
fn u32_try_from_invariants_relied_on_by_handlers() {
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
