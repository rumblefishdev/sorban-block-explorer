//! Network module: `GET /v1/network/stats`.
//!
//! Top-level chain-overview endpoint consumed by the Home dashboard
//! (frontend §6.2). Read-only, no pagination, no filters. See task
//! 0045 for the full spec and ADR 0021 §E1 for the source-of-truth
//! query set.

pub mod cache;
pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the network sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::get_network_stats))
}
