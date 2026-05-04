//! Search API module: `GET /v1/search`.
//!
//! Spec sources:
//!   * lore task 0053
//!   * `docs/architecture/database-schema/endpoint-queries/22_get_search.sql`
//!     (authoritative SQL — six narrow CTEs unioned, `:include_*` flags
//!     per entity bucket, `:per_group_limit` cap)
//!   * `docs/architecture/backend/backend-overview.md §6.3 Search`
//!
//! No caching (per task 0053): variable `q` makes a TTL cache useless
//! and the per-CTE `LIMIT` keeps each query bounded.

mod classifier;
pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the search sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::get_search))
}
