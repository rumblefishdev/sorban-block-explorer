//! Axum handlers for the NFT endpoints. Pure DB — no read-time XDR.

#![allow(clippy::result_large_err)]

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};

use crate::common::cursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{ListParams, NftIdCursor, NftItem, NftTransferCursor, NftTransferItem};
use super::queries::{ResolvedListParams, fetch_by_id, fetch_list, fetch_transfers, nft_exists};

/// Parse `:id` path parameter as a positive `i32` (NFT surrogate id).
///
/// Returns the canonical `INVALID_ID` envelope on parse failure, zero, or
/// negative. Centralised here because both `get_nft` and
/// `list_nft_transfers` accept the same `:id` shape.
fn parse_nft_id(raw: &str) -> Result<i32, axum::response::Response> {
    match raw.parse::<i32>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(errors::bad_request_with_details(
            errors::INVALID_ID,
            "id must be a positive integer (NFT surrogate id)",
            serde_json::json!({ "param": "id", "received": raw }),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/nfts",
    tag = "nfts",
    params(
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated NFT list",
         body = Paginated<NftItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_nfts(
    State(state): State<AppState>,
    pagination: Pagination<NftIdCursor>,
    Query(params): Query<ListParams>,
) -> Response {
    if let Err(resp) = filters::strkey_opt(params.filter_contract_id.as_deref(), 'C', "contract_id")
    {
        return resp;
    }

    if let Err(resp) = filters::reject_sql_wildcards_opt(params.filter_name.as_deref(), "name") {
        return resp;
    }

    let resolved = ResolvedListParams {
        limit: i64::from(pagination.limit) + 1,
        cursor: pagination.cursor,
        filter_collection: params.filter_collection,
        filter_contract_id: params.filter_contract_id,
        filter_name: params.filter_name,
    };

    let mut rows = match fetch_list(&state.db, &resolved).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_nfts: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(&mut rows, pagination.limit, |r| {
        cursor::encode(&NftIdCursor { id: r.id })
    });

    Json(into_envelope(rows, page)).into_response()
}

#[utoipa::path(
    get,
    path = "/nfts/{id}",
    tag = "nfts",
    params(
        ("id" = i32, Path, description = "Internal NFT surrogate id (`nfts.id`)."),
    ),
    responses(
        (status = 200, description = "NFT detail", body = NftItem),
        (status = 400, description = "Invalid id format", body = ErrorEnvelope),
        (status = 404, description = "NFT not found",   body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_nft(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match parse_nft_id(&id) {
        Ok(n) => n,
        Err(resp) => return resp,
    };

    let row = match fetch_by_id(&state.db, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("nft not found"),
        Err(e) => {
            tracing::error!("DB error fetching nft {id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    Json(row).into_response()
}

#[utoipa::path(
    get,
    path = "/nfts/{id}/transfers",
    tag = "nfts",
    params(
        ("id" = i32, Path, description = "Internal NFT surrogate id (`nfts.id`)."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated NFT transfer history",
         body = Paginated<NftTransferItem>),
        (status = 400, description = "Invalid id / pagination", body = ErrorEnvelope),
        (status = 404, description = "NFT not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_nft_transfers(
    State(state): State<AppState>,
    pagination: Pagination<NftTransferCursor>,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_nft_id(&id) {
        Ok(n) => n,
        Err(resp) => return resp,
    };

    match nft_exists(&state.db, id).await {
        Ok(true) => {}
        Ok(false) => return errors::not_found("nft not found"),
        Err(e) => {
            tracing::error!("DB error in nft_exists({id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    let fetch_limit = i64::from(pagination.limit) + 1;
    let mut rows =
        match fetch_transfers(&state.db, id, pagination.cursor.as_ref(), fetch_limit).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB error in fetch_transfers({id}): {e}");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    let page = finalize_page(&mut rows, pagination.limit, |last| {
        cursor::encode(&NftTransferCursor {
            created_at: last.created_at,
            ledger_sequence: last.ledger_sequence,
            event_order: last.event_order,
        })
    });

    Json(into_envelope(rows, page)).into_response()
}
