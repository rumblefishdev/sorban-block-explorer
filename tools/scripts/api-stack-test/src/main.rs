//! PoC: Rust API stack for Soroban Block Explorer (task 0092)
//!
//! Demonstrates the full recommended stack:
//! - axum 0.8 + lambda_http 1.1 (Lambda handler)
//! - sqlx 0.8 (PostgreSQL, typed JSONB, FromRow)
//! - utoipa 5.4 + utoipa-axum 0.2 (OpenAPI generation)
//! - tower-http (CORS, tracing)
//! - Cursor-based pagination (opaque base64)
//!
//! Run locally: cargo lambda watch
//! Build for Lambda: cargo lambda build --release --arm64

mod cursor;
mod error;
mod ledgers;
mod pagination;

use axum::Router;
use lambda_http::{run, tracing, Error};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Soroban Block Explorer API", version = "0.1.0"),
    paths(
        ledgers::list_ledgers,
        ledgers::get_ledger,
    ),
    components(schemas(
        ledgers::Ledger,
        pagination::PaginatedResponse::<ledgers::Ledger>,
        error::ErrorBody,
    )),
    tags(
        (name = "Ledgers", description = "Ledger endpoints"),
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::init_default_subscriber();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/explorer".into());

    // Lambda-optimized pool: lazy connect defers DB handshake until first query,
    // avoiding a roundtrip during cold start if the invocation doesn't need DB.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Some(Duration::from_secs(600)))
        .test_before_acquire(true)
        .connect_lazy(&database_url)
        .expect("DB pool creation failed");

    let state = AppState { db: pool };

    let app = Router::new()
        .merge(ledgers::router())
        .merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", ApiDoc::openapi()),
        )
        .layer(CorsLayer::permissive()) // PoC only — production should scope to known origins
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    run(app).await
}
