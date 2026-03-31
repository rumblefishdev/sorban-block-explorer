use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// Error response body returned by all error status codes.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Human-readable error message.
    #[schema(example = "ledger 999 not found")]
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("database error")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Database(err) => {
                tracing::error!(?err, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
