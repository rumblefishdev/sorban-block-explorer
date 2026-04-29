//! Axum handler for `GET /v1/network/stats`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::common::errors;
use crate::openapi::schemas::ErrorEnvelope;
use crate::state::AppState;

use super::dto::NetworkStats;
use super::queries;

/// API-Gateway-facing cache hint. Matches the `apiGatewayCacheTtlMutable`
/// value (10s) in `infra/envs/{staging,production}.json` so that when
/// the gateway cache cluster is enabled it uses its configured TTL
/// rather than treating the response as cacheable indefinitely.
/// Worst-case user-perceived staleness is additive (~inner TTL + this
/// header), not bounded by the inner TTL — see `cache.rs` module docs.
/// Browsers / CDNs may also observe this header; `public` makes that
/// explicit.
const CACHE_CONTROL_VALUE: HeaderValue = HeaderValue::from_static("public, max-age=10");

/// Get top-level chain overview stats.
///
/// Reads the canonical single-statement network-stats query (latest
/// ledger row + `ledgers` 60s aggregate for TPS + `pg_class.reltuples`
/// estimates for accounts / contracts) and caches the assembled
/// response for 30s in process memory. See the task 0045 spec and
/// `docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`
/// for the full data-source mapping.
///
/// Concurrent cold-cache requests deduplicate via
/// `moka::sync::Cache::try_get_with` — the first thread runs the query
/// and the rest wait on its result instead of fanning out N Postgres
/// round-trips.
#[utoipa::path(
    get,
    path = "/network/stats",
    tag = "network",
    responses(
        (status = 200, description = "Chain overview stats", body = NetworkStats),
        (status = 500, description = "Database error",       body = ErrorEnvelope),
    ),
)]
pub async fn get_network_stats(State(state): State<AppState>) -> Response {
    // `try_get_with` deduplicates concurrent cold-cache requests: only
    // the first task runs the DB query, every other concurrent task on
    // the same key waits for that task's result and gets a clone of it
    // — even though our DB query is async, because we are using
    // `moka::future::Cache`. Errors are propagated as `Arc<sqlx::Error>`
    // so a single failed fetch is not cached and the next request
    // retries cleanly.
    let result: Result<Arc<NetworkStats>, Arc<sqlx::Error>> = state
        .network_cache
        .try_get_with((), async {
            queries::fetch_stats(&state.db).await.map(Arc::new)
        })
        .await;

    match result {
        Ok(stats) => ok_response(&stats),
        Err(e) => {
            tracing::error!("DB error in get_network_stats: {e}");
            errors::internal_error(errors::DB_ERROR, "Unable to retrieve network statistics.")
        }
    }
}

/// Build the 200 response with the canonical `Cache-Control` header.
/// Centralised so cache-hit and cache-miss paths cannot drift on the
/// header set.
fn ok_response(stats: &NetworkStats) -> Response {
    let mut resp = Json(stats.clone()).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, CACHE_CONTROL_VALUE);
    resp
}

#[cfg(test)]
mod tests {
    //! End-to-end shape check for `/v1/network/stats`.
    //!
    //! Mirrors the `DATABASE_URL`-gated pattern used by
    //! `crates/indexer/tests/persist_integration.rs` — runs only when
    //! the env var is set and reachable, skips cleanly otherwise so
    //! `cargo test` is green on a workstation without the compose
    //! stack up.
    //!
    //!   docker compose up -d
    //!   npm run db:migrate
    //!   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
    //!       cargo test -p api --bin api network -- --test-threads=1
    use axum::Router;
    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use utoipa_axum::router::OpenApiRouter;

    use crate::contracts::cache::new_contract_cache;
    use crate::network;
    use crate::network::cache::new_network_cache;
    use crate::state::AppState;
    use crate::stellar_archive::StellarArchiveFetcher;

    fn app(db: PgPool) -> Router {
        let aws_cfg = aws_sdk_s3::config::Builder::new()
            .region(aws_sdk_s3::config::Region::new("us-east-2"))
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .build();
        let s3 = aws_sdk_s3::Client::from_conf(aws_cfg);
        let fetcher = StellarArchiveFetcher::new(s3);
        let state = AppState {
            db,
            fetcher,
            contract_cache: new_contract_cache(),
            network_cache: new_network_cache(),
            network_id: xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE),
        };

        let (router, _spec) = OpenApiRouter::new()
            .nest("/v1", network::router())
            .with_state(state)
            .split_for_parts();
        router
    }

    /// Each test owns its own `AppState` (and therefore its own moka
    /// cache instance), so global serialisation is no longer required —
    /// parallel tests cannot trample each other's cache state.
    #[tokio::test]
    async fn stats_endpoint_returns_documented_shape_against_real_db() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL unset — skipping network stats integration test");
            return;
        };
        let pool = match PgPool::connect(&database_url).await {
            Ok(p) => p,
            Err(err) => {
                eprintln!("DATABASE_URL unreachable ({err}) — skipping network stats test");
                return;
            }
        };

        let resp = app(pool)
            .oneshot(
                Request::builder()
                    .uri("/v1/network/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let cc = resp
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(status, StatusCode::OK, "expected 200, got {status}: {json}");
        assert_eq!(
            cc.as_deref(),
            Some("public, max-age=10"),
            "Cache-Control header missing or wrong: {cc:?}"
        );

        // Shape asserted regardless of row counts — empty DB is fine.
        for key in [
            "tps_60s",
            "total_accounts",
            "total_contracts",
            "latest_ledger_sequence",
            "generated_at",
        ] {
            assert!(json.get(key).is_some(), "envelope missing `{key}`: {json}");
        }
        assert!(json["tps_60s"].is_number(), "tps_60s not number: {json}");
        assert!(
            json["total_accounts"].is_number(),
            "total_accounts not number: {json}"
        );
        assert!(
            json["total_contracts"].is_number(),
            "total_contracts not number: {json}"
        );
        assert!(
            json["latest_ledger_sequence"].is_number(),
            "latest_ledger_sequence not number: {json}"
        );
        // `latest_ledger_closed_at` may be `null` (empty DB) or an
        // ISO-8601 timestamp string serialised by chrono.
        if let Some(v) = json.get("latest_ledger_closed_at") {
            assert!(
                v.is_null() || v.is_string(),
                "latest_ledger_closed_at bad type: {json}"
            );
        }
        // `generated_at` is always present (DB `NOW()` on populated
        // cluster, `Utc::now()` fallback on empty cluster).
        assert!(
            json["generated_at"].is_string(),
            "generated_at not string: {json}"
        );
    }
}
