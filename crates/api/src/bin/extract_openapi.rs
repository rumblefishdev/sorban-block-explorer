//! Build-time OpenAPI spec extractor.
//!
//! Prints the current API spec to stdout so callers can redirect it to a file:
//! `cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json`

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

fn main() {
    let (_, spec) = OpenApiRouter::with_openapi(api::openapi::ApiDoc::openapi())
        .routes(routes!(api::ops::health))
        .nest("/v1", api::network::router())
        .nest("/v1", api::transactions::router())
        .nest("/v1", api::contracts::router())
        .nest("/v1", api::liquidity_pools::router())
        .nest("/v1", api::assets::router())
        .split_for_parts();

    println!(
        "{}",
        spec.to_pretty_json()
            .expect("failed to serialize OpenAPI spec as pretty JSON")
    );
}
