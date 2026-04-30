//! Operational endpoints (health checks, diagnostics).

use axum::Json;
use serde_json::{Value, json};

/// Liveness probe consumed by AWS Lambda health checks and smoke tests.
#[utoipa::path(
    get,
    path = "/health",
    tag = "ops",
    responses(
        (status = 200, description = "Service is healthy", body = serde_json::Value),
    ),
)]
pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}
