//! Assets API module — list / detail / transactions sub-resource.
//!
//! `:id` is multi-form (numeric / contract StrKey / `code-issuer` composite);
//! resolution lives in `handlers::parse_asset_id`.

pub mod dto;
mod handlers;
mod queries;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list_assets))
        .routes(routes!(handlers::get_asset))
        .routes(routes!(handlers::list_asset_transactions))
}
