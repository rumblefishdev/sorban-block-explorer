//! Contracts API module: detail, interface, invocations, events.
//!
//! Wire shapes mirror canonical SQL `endpoint-queries/{11..14}_*.sql`
//! (task 0167). Pagination, error envelopes, cursor codec, and StrKey
//! validation come from `crate::common::*` (task 0043). Contract metadata
//! is small and gets a 45 s per-Lambda cache (`cache::ContractMetadataCache`).

pub mod cache;
pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the contracts sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::get_contract))
        .routes(routes!(handlers::get_interface))
        .routes(routes!(handlers::list_invocations))
        .routes(routes!(handlers::list_events))
}
