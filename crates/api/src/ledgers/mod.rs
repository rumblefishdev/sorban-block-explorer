//! Ledgers API module — list (`GET /v1/ledgers`) and detail
//! (`GET /v1/ledgers/:sequence`) with embedded paginated transactions.
//!
//! Both endpoints are DB-only. The detail handler runs two statements:
//! a header lookup with prev/next navigation, and a partition-pruned
//! read of the `transactions` partition filtered by the requested
//! ledger. Memo / other heavy fields are exposed only by the
//! transaction detail endpoint; list rows stay slim.
//!
//! Canonical SQL refs:
//!   - `docs/architecture/database-schema/endpoint-queries/04_get_ledgers_list.sql`
//!   - `docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql`

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the ledgers sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_ledgers))
        .routes(routes!(handlers::get_ledger))
}
