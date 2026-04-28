//! Axum handler for `GET /v1/network/stats`.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::common::errors;
use crate::openapi::schemas::ErrorEnvelope;
use crate::state::AppState;

use super::cache;
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
    if let Some(stats) = cache::get() {
        return ok_response(stats);
    }

    match queries::fetch_stats(&state.db).await {
        Ok(stats) => {
            cache::put(stats.clone());
            ok_response(stats)
        }
        Err(e) => {
            tracing::error!("DB error in get_network_stats: {e}");
            errors::internal_error(errors::DB_ERROR, "Unable to retrieve network statistics.")
        }
    }
}

/// Build the 200 response with the canonical `Cache-Control` header.
/// Centralised so cache-hit and cache-miss paths cannot drift on the
/// header set.
fn ok_response(stats: NetworkStats) -> Response {
    let mut resp = Json(stats).into_response();
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

    use crate::contracts::cache::ContractMetadataCache;
    use crate::network;
    use crate::state::AppState;
    use crate::stellar_archive::StellarArchiveFetcher;

    use super::cache;

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
            contract_cache: ContractMetadataCache::new(),
        };

        let (router, _spec) = OpenApiRouter::new()
            .nest("/v1", network::router())
            .with_state(state)
            .split_for_parts();
        router
    }

    // The std `MutexGuard` is held across `.await` deliberately — its
    // sole purpose is to serialise this test against the unit tests in
    // `network/cache.rs` that touch the same global static. There is
    // no real cross-task contention to deadlock on.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn stats_endpoint_returns_documented_shape_against_real_db() {
        let _guard = cache::TEST_CACHE_MUTEX
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        cache::clear();

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
            "tps",
            "total_accounts",
            "total_contracts",
            "highest_indexed_ledger",
        ] {
            assert!(json.get(key).is_some(), "envelope missing `{key}`: {json}");
        }
        assert!(json["tps"].is_number(), "tps not number: {json}");
        assert!(
            json["total_accounts"].is_number(),
            "total_accounts not number: {json}"
        );
        assert!(
            json["total_contracts"].is_number(),
            "total_contracts not number: {json}"
        );
        assert!(
            json["highest_indexed_ledger"].is_number(),
            "highest_indexed_ledger not number: {json}"
        );
        // `ingestion_lag_seconds` may be `null` (empty DB) or a number.
        if let Some(v) = json.get("ingestion_lag_seconds") {
            assert!(
                v.is_null() || v.is_number(),
                "ingestion_lag_seconds bad type: {json}"
            );
        }
    }
}
