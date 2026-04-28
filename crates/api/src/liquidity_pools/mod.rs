//! Liquidity pools API module: pool participants list (task 0126).
//!
//! Today exposes a single endpoint —
//! `GET /v1/liquidity-pools/{pool_id}/participants` — sourced from the
//! `lp_positions` table (populated by task 0162). Pool metadata / detail /
//! list endpoints will arrive in follow-up tasks; the module is shaped
//! so additional handlers can be registered without restructuring.

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Sub-router mounted under `/v1` in `main::app`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::list_participants))
}
