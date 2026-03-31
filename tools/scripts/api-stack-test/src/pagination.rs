use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Cursor-based pagination query parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PaginationParams {
    /// Opaque base64-encoded cursor from a previous response. Omit for first page.
    #[param(example = "eyJiZWZvcmVfc2VxdWVuY2UiOjEwMH0")]
    pub cursor: Option<String>,

    /// Maximum number of items to return (default 20).
    /// Values outside [1, 100] are clamped (e.g., 0 → 1, 1000 → 100).
    #[param(example = 20, minimum = 1, maximum = 100)]
    pub limit: Option<i64>,
}

impl PaginationParams {
    /// Returns the effective page size, clamping to [1, 100], default 20.
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(20).clamp(1, 100)
    }
}

/// Generic paginated response envelope.
/// utoipa 5.x: register concrete types via turbofish in schemas(),
/// e.g. `PaginatedResponse::<Ledger>`.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedResponse<T: ToSchema> {
    /// The page of results.
    pub data: Vec<T>,

    /// Opaque cursor for the next page. Null if this is the last page.
    #[schema(example = "eyJiZWZvcmVfc2VxdWVuY2UiOjIwMH0", nullable)]
    pub next_cursor: Option<String>,

    /// Whether more results exist beyond this page.
    pub has_more: bool,
}
