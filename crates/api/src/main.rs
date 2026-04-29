//! REST API Lambda handler for the Soroban block explorer.

mod assets;
mod common;
mod config;
mod contracts;
mod liquidity_pools;
mod network;
mod openapi;
mod ops;
mod state;
#[cfg(test)]
mod tests_integration;
mod transactions;
// Public-archive XDR fetch helper. Used by E3, E13 and E14 endpoint handlers.
// Exposed as module so future handlers can call the extractors without
// further wiring.
mod stellar_archive;

use axum::{Json, Router, routing::get};
use utoipa::openapi::OpenApi as OpenApiSpec;

use crate::config::AppConfig;
use crate::state::AppState;
use crate::stellar_archive::StellarArchiveFetcher;

/// Build the application router from an explicit [`AppConfig`] and [`AppState`].
///
/// Kept pure (no `std::env` reads) so tests can construct their own
/// config and state without mutating process state.
fn app(config: &AppConfig, state: AppState) -> Router {
    // Shared `openapi::register_routes` builds the same chain that the
    // `extract_openapi` build-time binary uses, so the codegen spec and
    // the live router cannot advertise different endpoints. We then stamp
    // the runtime `servers` block (resolved from AppConfig.base_url) onto
    // the registered spec.
    let (router, mut spec) = openapi::register_routes()
        .with_state(state)
        .split_for_parts();
    spec.servers = Some(vec![utoipa::openapi::server::Server::new(&config.base_url)]);

    // Share the spec behind an Arc so `/api-docs-json` only clones
    // a reference count per request instead of the full document.
    let spec_arc = std::sync::Arc::new(spec);
    let spec_for_json = spec_arc.clone();
    let router = router.route(
        "/api-docs-json",
        get(move || {
            let spec = spec_for_json.clone();
            async move { Json(spec) }
        }),
    );

    mount_swagger_ui(router, spec_arc.as_ref())
}

#[cfg(feature = "swagger-ui")]
fn mount_swagger_ui(router: Router, spec: &OpenApiSpec) -> Router {
    use utoipa_swagger_ui::SwaggerUi;
    // `SwaggerUi::url` mounts its own handler for the spec JSON under
    // the passed path, so we give it a dedicated internal path to
    // avoid colliding with the always-on `/api-docs-json` public
    // endpoint registered above.
    router.merge(SwaggerUi::new("/api-docs").url("/api-docs/openapi.json", spec.clone()))
}

#[cfg(not(feature = "swagger-ui"))]
fn mount_swagger_ui(router: Router, _spec: &OpenApiSpec) -> Router {
    router
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    tracing::info!("api cold start — resolving database credentials");

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let secret_arn =
                std::env::var("SECRET_ARN").expect("either DATABASE_URL or SECRET_ARN must be set");
            let rds_endpoint = std::env::var("RDS_PROXY_ENDPOINT")
                .expect("RDS_PROXY_ENDPOINT must be set when using SECRET_ARN");
            db::secrets::resolve_database_url(&secret_arn, &rds_endpoint)
                .await
                .expect("failed to resolve database URL from Secrets Manager")
        }
    };

    let db = db::pool::create_pool(&database_url).expect("failed to create DB pool");
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .no_credentials()
        .region(aws_sdk_s3::config::Region::new("us-east-2"))
        .timeout_config(stellar_archive::default_timeout_config())
        .load()
        .await;
    let s3_client = aws_sdk_s3::Client::new(&aws_config);
    let fetcher = StellarArchiveFetcher::new(s3_client);

    let config = AppConfig::from_env();
    let passphrase = std::env::var("STELLAR_NETWORK_PASSPHRASE").unwrap_or_else(|_| {
        panic!(
            "STELLAR_NETWORK_PASSPHRASE env not set; required to align tx_set \
             envelopes with apply-order tx_processing when re-extracting \
             heavy fields. Expected the full Stellar passphrase string \
             (e.g. \"Public Global Stellar Network ; September 2015\")."
        )
    });
    let network_id = xdr_parser::network_id(&passphrase);
    let state = AppState {
        db,
        fetcher,
        contract_cache: contracts::cache::ContractMetadataCache::new(),
        network_id,
    };
    let app = app(&config, state);

    lambda_http::run(app).await.expect("failed to run Lambda");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    fn test_config() -> AppConfig {
        AppConfig {
            base_url: "http://localhost:9000".to_string(),
        }
    }

    /// Build a test app. Uses `connect_lazy` so no real DB connection is opened.
    fn test_app() -> Router {
        let db = sqlx::PgPool::connect_lazy("postgres://localhost/test_unused")
            .expect("connect_lazy never fails");
        // Build a minimal StellarArchiveFetcher using a stub AWS config.
        // The S3 client will not be called during spec/health tests.
        let aws_cfg = aws_sdk_s3::config::Builder::new()
            .region(aws_sdk_s3::config::Region::new("us-east-2"))
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .build();
        let s3 = aws_sdk_s3::Client::from_conf(aws_cfg);
        let fetcher = StellarArchiveFetcher::new(s3);
        app(
            &test_config(),
            AppState {
                db,
                fetcher,
                contract_cache: contracts::cache::ContractMetadataCache::new(),
                network_id: xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE),
            },
        )
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn api_docs_json_contains_health_path() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(spec["info"]["title"], "Soroban Block Explorer API");
        assert_eq!(spec["info"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(
            spec["paths"]["/health"].is_object(),
            "spec missing /health path: {spec}"
        );
        assert_eq!(spec["servers"][0]["url"], "http://localhost:9000");
    }

    #[tokio::test]
    async fn api_docs_json_has_error_envelope_component() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            spec["components"]["schemas"]["ErrorEnvelope"].is_object(),
            "spec missing ErrorEnvelope component: {spec}"
        );
        assert!(
            spec["components"]["schemas"]["PageInfo"].is_object(),
            "spec missing PageInfo component: {spec}"
        );
    }

    #[tokio::test]
    async fn api_docs_json_contains_contracts_paths() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();
        for path in [
            "/v1/contracts/{contract_id}",
            "/v1/contracts/{contract_id}/interface",
            "/v1/contracts/{contract_id}/invocations",
            "/v1/contracts/{contract_id}/events",
        ] {
            assert!(
                spec["paths"][path].is_object(),
                "spec missing {path} path: {spec}"
            );
        }
    }

    #[tokio::test]
    async fn api_docs_json_contains_assets_paths() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();
        for path in [
            "/v1/assets",
            "/v1/assets/{id}",
            "/v1/assets/{id}/transactions",
        ] {
            assert!(
                spec["paths"][path].is_object(),
                "spec missing {path} path: {spec}"
            );
        }
    }

    #[tokio::test]
    async fn api_docs_json_contains_transactions_paths() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            spec["paths"]["/v1/transactions"].is_object(),
            "spec missing /v1/transactions path: {spec}"
        );
        assert!(
            spec["paths"]["/v1/transactions/{hash}"].is_object(),
            "spec missing /v1/transactions/{{hash}} path: {spec}"
        );
    }

    #[tokio::test]
    async fn api_docs_json_contains_network_stats_path() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs-json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let spec: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            spec["paths"]["/v1/network/stats"].is_object(),
            "spec missing /v1/network/stats path: {spec}"
        );
        assert!(
            spec["components"]["schemas"]["NetworkStats"].is_object(),
            "spec missing NetworkStats component: {spec}"
        );
    }

    #[cfg(feature = "swagger-ui")]
    #[tokio::test]
    async fn swagger_ui_mounted_when_feature_enabled() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            response.status().is_success() || response.status().is_redirection(),
            "expected 2xx/3xx for /api-docs/, got {}",
            response.status()
        );
    }

    #[cfg(not(feature = "swagger-ui"))]
    #[tokio::test]
    async fn swagger_ui_absent_without_feature() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
