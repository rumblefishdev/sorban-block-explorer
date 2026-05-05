//! Liquidity pools API module.
//!
//! Endpoints (canonical SQL `endpoint-queries/{18..21,23}_*.sql`):
//!   - `GET /v1/liquidity-pools`                          (task 0052)
//!   - `GET /v1/liquidity-pools/{pool_id}`                (task 0052)
//!   - `GET /v1/liquidity-pools/{pool_id}/transactions`   (task 0052)
//!   - `GET /v1/liquidity-pools/{pool_id}/chart`          (task 0052)
//!   - `GET /v1/liquidity-pools/{pool_id}/participants`   (task 0126)
//!
//! Pagination, error envelopes, cursor codec, and StrKey validation come
//! from `crate::common::*` (task 0043).

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Sub-router mounted under `/v1` in `main::app`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_pools))
        .routes(routes!(handlers::get_pool))
        .routes(routes!(handlers::list_pool_transactions))
        .routes(routes!(handlers::get_pool_chart))
        .routes(routes!(handlers::list_participants))
}
