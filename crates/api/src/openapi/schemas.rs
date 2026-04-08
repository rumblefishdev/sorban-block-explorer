//! Reusable OpenAPI schema components.
//!
//! These shapes are load-bearing: every endpoint handler that ships in
//! M2 (tasks 0043–0053) is expected to return either `ErrorEnvelope`
//! on failure or `Paginated<T>` / a plain domain object on success.
//! Their structural decisions are captured in ADR 0008.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Canonical error response body.
///
/// Intentionally simpler than RFC 7807 (`problem+json`) — see ADR 0008.
/// Machine-readable `code` drives client behaviour; `message` is a
/// human-friendly description; `details` carries optional structured
/// context (field-level validation errors, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorEnvelope {
    /// Stable machine-readable error code, e.g. `"invalid_cursor"`.
    pub code: String,
    /// Human-readable error description.
    pub message: String,
    /// Optional structured details. Shape is error-specific.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Pagination metadata attached to list responses.
///
/// Cursor-based pagination (not offset) — see ADR 0008. Aligns with
/// Stellar Horizon conventions and produces stable listings even when
/// the underlying ledger stream advances between requests.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PageInfo {
    /// Opaque cursor identifying the next page, absent when the
    /// client has reached the end of the stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Page size that produced `data`. Echoes the client's requested
    /// limit (clamped server-side).
    pub limit: u32,
    /// `true` when further pages exist, `false` on the final page.
    pub has_more: bool,
}

/// Canonical envelope for paginated list responses.
///
/// Generic over the item type `T` so every endpoint can reuse a single
/// shape. Concrete instantiations (e.g. `Paginated<Transaction>`) are
/// picked up automatically by utoipa-axum via the handler return type
/// when M2 endpoint modules are wired in. Unused in M1 — kept as
/// infrastructure that M2 endpoints will consume.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Paginated<T>
where
    T: ToSchema,
{
    pub data: Vec<T>,
    pub page: PageInfo,
}
