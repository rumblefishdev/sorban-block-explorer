//! REST API Lambda handler for the Soroban block explorer.

mod config;
mod openapi;
// Public-archive XDR fetch helper. Used by E3 and E14 endpoint handlers
// (added in a follow-up task). Exposed as module so future handlers
// can call the extractors without further wiring.
#[allow(dead_code)]
mod stellar_archive;

use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiSpec;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::AppConfig;
use crate::openapi::ApiDoc;

/// Liveness probe consumed by AWS Lambda health checks and smoke tests.
#[utoipa::path(
    get,
    path = "/health",
    tag = "ops",
    responses(
        (status = 200, description = "Service is healthy", body = serde_json::Value),
    ),
)]
async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Build the application router from an explicit [`AppConfig`].
///
/// Kept pure (no `std::env` reads) so tests can construct their own
/// config without mutating process state.
fn app(config: &AppConfig) -> Router {
    // `routes!(handler)` registers the #[utoipa::path] annotation so
    // the spec returned by `split_for_parts` carries the live paths.
    // We then stamp the runtime `servers` block (resolved from
    // AppConfig.base_url) onto the registered spec.
    let (router, mut spec) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
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

    let config = AppConfig::from_env();
    let app = app(&config);
    lambda_http::run(app).await.expect("failed to run Lambda");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_config() -> AppConfig {
        AppConfig {
            base_url: "http://localhost:9000".to_string(),
        }
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = app(&test_config());
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
        let app = app(&test_config());
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
        let app = app(&test_config());
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

    #[cfg(feature = "swagger-ui")]
    #[tokio::test]
    async fn swagger_ui_mounted_when_feature_enabled() {
        let app = app(&test_config());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api-docs/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // SwaggerUi redirects the index path or serves HTML directly.
        assert!(
            response.status().is_success() || response.status().is_redirection(),
            "expected 2xx/3xx for /api-docs/, got {}",
            response.status()
        );
    }

    #[cfg(not(feature = "swagger-ui"))]
    #[tokio::test]
    async fn swagger_ui_absent_without_feature() {
        let app = app(&test_config());
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
