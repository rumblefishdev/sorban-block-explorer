use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::cursor;
use crate::error::AppError;
use crate::pagination::{PaginatedResponse, PaginationParams};

/// Rust type alias — lets utoipa resolve the type in `body = ...` without `<>`
type PaginatedLedgers = PaginatedResponse<Ledger>;
use crate::AppState;

// --- Model (matches DB schema) ---

/// A closed ledger on the Stellar network.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Ledger {
    /// Ledger sequence number (unique identifier).
    #[schema(example = 51_000_000)]
    pub sequence: i64,

    /// Hash of this ledger's header (hex, 64 chars).
    #[schema(example = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2")]
    pub hash: String,

    /// When this ledger closed.
    pub closed_at: DateTime<Utc>,

    /// Stellar protocol version at this ledger.
    #[schema(example = 25)]
    pub protocol_version: i32,

    /// Number of transactions in this ledger.
    #[schema(example = 42)]
    pub transaction_count: i32,

    /// Base fee in stroops.
    #[schema(example = 100)]
    pub base_fee: i64,
}

// --- Handlers ---

/// List ledgers (cursor-based pagination, descending by sequence).
#[utoipa::path(
    get,
    path = "/ledgers",
    tag = "Ledgers",
    params(PaginationParams),
    responses(
        (status = 200, description = "Paginated list of ledgers", body = inline(PaginatedLedgers)),
        (status = 400, description = "Invalid cursor", body = crate::error::ErrorBody),
    )
)]
pub async fn list_ledgers(
    Query(params): Query<PaginationParams>,
    State(state): State<AppState>,
) -> Result<Json<PaginatedResponse<Ledger>>, AppError> {
    let limit = params.limit();

    let before_seq = match &params.cursor {
        Some(c) => cursor::decode(c)?,
        None => i64::MAX,
    };

    // Fetch limit+1 to detect has_more
    let rows: Vec<Ledger> = sqlx::query_as(
        "SELECT sequence, hash, closed_at, protocol_version, transaction_count, base_fee \
         FROM ledgers WHERE sequence < $1 ORDER BY sequence DESC LIMIT $2",
    )
    .bind(before_seq)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let data: Vec<Ledger> = rows.into_iter().take(limit as usize).collect();
    let next_cursor = if has_more {
        data.last().map(|l| cursor::encode(l.sequence))
    } else {
        None
    };

    Ok(Json(PaginatedResponse {
        data,
        next_cursor,
        has_more,
    }))
}

/// Get a single ledger by sequence number.
#[utoipa::path(
    get,
    path = "/ledgers/{sequence}",
    tag = "Ledgers",
    params(("sequence" = i64, Path, description = "Ledger sequence number")),
    responses(
        (status = 200, description = "Ledger found", body = Ledger),
        (status = 404, description = "Ledger not found", body = crate::error::ErrorBody),
    )
)]
pub async fn get_ledger(
    Path(sequence): Path<i64>,
    State(state): State<AppState>,
) -> Result<Json<Ledger>, AppError> {
    let ledger: Ledger = sqlx::query_as(
        "SELECT sequence, hash, closed_at, protocol_version, transaction_count, base_fee \
         FROM ledgers WHERE sequence = $1",
    )
    .bind(sequence)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("ledger {} not found", sequence)))?;

    Ok(Json(ledger))
}

// --- Router ---

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ledgers", get(list_ledgers))
        .route("/ledgers/{sequence}", get(get_ledger))
}
