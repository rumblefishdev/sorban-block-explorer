//! Contracts API module: detail, interface, invocations, events.
//!
//! Per ADRs 0033 / 0034 the events and invocations responses are assembled
//! from a DB appearance index plus read-time XDR fetched from the public
//! Stellar archive (ADR 0029). Contract metadata is small and gets a 45 s
//! per-Lambda cache (`cache::ContractMetadataCache`).

pub mod cache;
pub mod cursor;
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
