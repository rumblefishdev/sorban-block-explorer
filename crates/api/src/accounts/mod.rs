//! Accounts API module — detail (`GET /v1/accounts/:account_id`) and
//! account-scoped transaction history
//! (`GET /v1/accounts/:account_id/transactions`).
//!
//! Pure DB — no read-time XDR. Account scope is intentionally limited to
//! summary + balances + transactions per ADR 0025 / task 0048.
//!
//! Canonical SQL refs:
//!   - `docs/architecture/database-schema/endpoint-queries/06_get_accounts_by_id.sql`
//!   - `docs/architecture/database-schema/endpoint-queries/07_get_accounts_transactions.sql`

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Build the accounts sub-router (mounted under `/v1` in `main::app`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::get_account))
        .routes(routes!(handlers::list_account_transactions))
}
