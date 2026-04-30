//! Handlers for the liquidity-pool participants endpoint.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};

use crate::common::cursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{ParticipantItem, SharesCursor};
use super::queries::{fetch_participants, pool_exists};

/// Pool ID is a 32-byte hash rendered as 64 lowercase hex chars in the
/// URL path. Anything else is rejected before we touch the DB.
fn is_valid_pool_id_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/participants",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — 64-char lowercase hex (BYTEA) per ADR 0024."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated participants list",
         body = Paginated<ParticipantItem>),
        (status = 400, description = "Invalid pool_id, limit, or cursor", body = ErrorEnvelope),
        (status = 404, description = "Pool not found",  body = ErrorEnvelope),
        (status = 500, description = "Database error",  body = ErrorEnvelope),
    )
)]
pub async fn list_participants(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    pagination: Pagination<SharesCursor>,
) -> Response {
    if !is_valid_pool_id_hex(&pool_id) {
        return errors::bad_request(
            "invalid_pool_id",
            "pool_id must be a 64-character lowercase hex string",
        );
    }

    // 404 vs 200-empty disambiguation: a missing pool gets 404 so the
    // frontend can route to a "pool not found" page. An existing pool
    // with no current participants returns 200 with `data: []`.
    match pool_exists(&state.db, &pool_id).await {
        Ok(true) => {}
        Ok(false) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!("DB error in pool_exists({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    // Fetch limit + 1 so `finalize_page` can detect a next page without
    // a separate count query.
    let fetch_limit = i64::from(pagination.limit) + 1;
    let mut rows = match fetch_participants(
        &state.db,
        &pool_id,
        pagination.cursor.as_ref(),
        fetch_limit,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in fetch_participants({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Cursor builder gets the kept tail row directly — both the wire
    // `shares` (NUMERIC string) and the internal `account_id_surrogate`
    // BIGINT travel inside the opaque payload, never on the wire.
    let page = finalize_page(&mut rows, pagination.limit, |last| {
        cursor::encode(&SharesCursor {
            shares: last.shares.clone(),
            account_id: last.account_id_surrogate,
        })
    });

    let data: Vec<ParticipantItem> = rows
        .into_iter()
        .map(|r| ParticipantItem {
            account: r.account,
            shares: r.shares,
            share_percentage: r.share_percentage,
            first_deposit_ledger: r.first_deposit_ledger,
            last_updated_ledger: r.last_updated_ledger,
        })
        .collect();

    Json(into_envelope(data, page)).into_response()
}
