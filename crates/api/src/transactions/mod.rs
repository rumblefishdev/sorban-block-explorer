//! Transactions API module: GET /v1/transactions and GET /v1/transactions/:hash.

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the transactions sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_transactions))
        .routes(routes!(handlers::get_transaction))
}
