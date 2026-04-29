//! Operational endpoints (health checks, diagnostics).

use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Response body for the liveness probe.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` when the service is reachable.
    pub status: String,
}

/// Liveness probe consumed by AWS Lambda health checks and smoke tests.
#[utoipa::path(
    get,
    path = "/health",
    tag = "ops",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
    ),
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}
