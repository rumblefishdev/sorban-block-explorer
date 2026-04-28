//! Axum handlers for the assets endpoints. Pure DB — no read-time XDR.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use domain::TokenAssetType;

use crate::common::cursor;
use crate::common::cursor::TsIdCursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, finalize_ts_id_page, into_envelope};
use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::state::AppState;

use super::dto::{AssetDetailResponse, AssetItem, AssetTransactionItem, ListParams};
use super::queries::{
    AssetIdCursor, AssetIdentity, AssetRow, ResolvedListParams, asset_predicate_present,
    fetch_by_code_issuer, fetch_by_contract_id, fetch_by_id, fetch_list, fetch_transactions,
};

fn map_item(row: AssetRow) -> AssetItem {
    AssetItem {
        id: row.id,
        asset_type_name: row.asset_type_name,
        asset_type: row.asset_type,
        asset_code: row.asset_code,
        issuer: row.issuer,
        contract_id: row.contract_id,
        name: row.name,
        total_supply: row.total_supply,
        holder_count: row.holder_count,
        icon_url: row.icon_url,
    }
}

/// Three forms of `:id`. The first that parses cleanly drives the SQL.
enum AssetIdRef<'a> {
    Numeric(i32),
    Contract(&'a str),
    CodeIssuer(&'a str, &'a str),
}

fn parse_asset_id(raw: &str) -> Option<AssetIdRef<'_>> {
    if let Ok(n) = raw.parse::<i32>() {
        return Some(AssetIdRef::Numeric(n));
    }
    if is_strkey_shape(raw, 'C') {
        return Some(AssetIdRef::Contract(raw));
    }
    // Codes never contain `-`; split on the LAST one and validate the
    // issuer half as a G-StrKey to disambiguate from C-StrKeys with stray dashes.
    if let Some(idx) = raw.rfind('-')
        && idx > 0
        && idx < raw.len() - 1
    {
        let code = &raw[..idx];
        let issuer = &raw[idx + 1..];
        if is_strkey_shape(issuer, 'G') {
            return Some(AssetIdRef::CodeIssuer(code, issuer));
        }
    }
    None
}

fn is_strkey_shape(s: &str, prefix: char) -> bool {
    s.len() == 56
        && s.starts_with(prefix)
        && s.bytes().all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7'))
}

#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    params(
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated asset list",
         body = Paginated<AssetItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_assets(
    State(state): State<AppState>,
    pagination: Pagination<AssetIdCursor>,
    Query(params): Query<ListParams>,
) -> Response {
    let asset_type: Option<i16> = match filters::parse_enum_opt::<TokenAssetType>(
        params.filter_type.as_deref(),
        "type",
        Some("asset type"),
    ) {
        Ok(maybe) => maybe.map(|t| t as i16),
        Err(resp) => return resp,
    };

    // The query builder wraps `filter[code]` in `'%' || $1 || '%'` for the
    // trigram match — bare `%` / `_` from the caller would silently change
    // match semantics. Reject explicitly so a confused caller gets a 400
    // instead of an unexplained over-broad result set.
    if let Some(code) = params.filter_code.as_deref()
        && code.bytes().any(|b| b == b'%' || b == b'_')
    {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "filter[code] must not contain `%` or `_` (SQL wildcard literals)",
            serde_json::json!({ "filter": "code", "received": code }),
        );
    }

    let resolved = ResolvedListParams {
        limit: i64::from(pagination.limit),
        cursor: pagination.cursor,
        asset_type,
        asset_code: params.filter_code,
    };

    let mut rows: Vec<AssetRow> = match fetch_list(&state.db, &resolved).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_assets: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(&mut rows, pagination.limit, |r| {
        cursor::encode(&AssetIdCursor { id: r.id })
    });
    let data: Vec<AssetItem> = rows.into_iter().map(map_item).collect();

    Json(into_envelope(data, page)).into_response()
}

#[utoipa::path(
    get,
    path = "/assets/{id}",
    tag = "assets",
    params(
        ("id" = String, Path,
         description = "Numeric `assets.id`, contract StrKey (C…, 56 chars), or `code-issuer` composite."),
    ),
    responses(
        (status = 200, description = "Asset detail", body = AssetDetailResponse),
        (status = 400, description = "Invalid id format", body = ErrorEnvelope),
        (status = 404, description = "Asset not found",   body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_asset(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let parsed = match parse_asset_id(&id) {
        Some(p) => p,
        None => {
            return errors::bad_request_with_details(
                errors::INVALID_ID,
                "id must be a numeric assets.id, contract StrKey (C…, 56 chars), \
                 or `code-issuer` composite (e.g. USDC-GA…XYZ)",
                serde_json::json!({ "received": id }),
            );
        }
    };

    let row = match fetch_with(&state, parsed).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("asset not found"),
        Err(e) => {
            tracing::error!("DB error fetching asset {id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let deployed_at_ledger = row.deployed_at_ledger;
    let response = AssetDetailResponse {
        item: map_item(row),
        deployed_at_ledger,
        description: None,
        home_page: None,
    };
    Json(response).into_response()
}

async fn fetch_with(
    state: &AppState,
    parsed: AssetIdRef<'_>,
) -> Result<Option<AssetRow>, sqlx::Error> {
    match parsed {
        AssetIdRef::Numeric(n) => fetch_by_id(&state.db, n).await,
        AssetIdRef::Contract(c) => fetch_by_contract_id(&state.db, c).await,
        AssetIdRef::CodeIssuer(code, issuer) => fetch_by_code_issuer(&state.db, code, issuer).await,
    }
}

#[utoipa::path(
    get,
    path = "/assets/{id}/transactions",
    tag = "assets",
    params(
        ("id" = String, Path,
         description = "Numeric `assets.id`, contract StrKey (C…), or `code-issuer` composite."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated transactions involving the asset",
         body = Paginated<AssetTransactionItem>),
        (status = 400, description = "Invalid id format / pagination", body = ErrorEnvelope),
        (status = 404, description = "Asset not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_asset_transactions(
    State(state): State<AppState>,
    pagination: Pagination<TsIdCursor>,
    Path(id): Path<String>,
) -> Response {
    let parsed = match parse_asset_id(&id) {
        Some(p) => p,
        None => {
            return errors::bad_request_with_details(
                errors::INVALID_ID,
                "id must be a numeric assets.id, contract StrKey (C…, 56 chars), \
                 or `code-issuer` composite (e.g. USDC-GA…XYZ)",
                serde_json::json!({ "received": id }),
            );
        }
    };

    let row = match fetch_with(&state, parsed).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("asset not found"),
        Err(e) => {
            tracing::error!("DB error fetching asset {id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    if TokenAssetType::try_from(row.asset_type).is_err() {
        tracing::error!(
            "unknown asset_type discriminant {} for asset id={}",
            row.asset_type,
            row.id
        );
        return errors::internal_error(
            errors::DB_ERROR,
            "asset row carries an unknown asset_type discriminant",
        );
    }

    let identity = AssetIdentity {
        asset_code: row.asset_code.as_deref(),
        issuer: row.issuer.as_deref(),
        contract_id: row.contract_id.as_deref(),
    };

    // Native XLM has no DB-side identity referenced by ops — empty page
    // rather than emit `WHERE ()` SQL.
    if !asset_predicate_present(&identity) {
        let empty = into_envelope::<AssetTransactionItem>(
            Vec::new(),
            PageInfo {
                cursor: None,
                limit: pagination.limit,
                has_more: false,
            },
        );
        return Json(empty).into_response();
    }

    let mut rows = match fetch_transactions(
        &state.db,
        &identity,
        i64::from(pagination.limit),
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_asset_transactions: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_ts_id_page(&mut rows, pagination.limit, |r| r.created_at, |r| r.id);
    let data: Vec<AssetTransactionItem> = rows
        .into_iter()
        .map(|r| AssetTransactionItem {
            hash: r.hash,
            ledger_sequence: r.ledger_sequence,
            source_account: r.source_account,
            successful: r.successful,
            fee_charged: r.fee_charged,
            created_at: r.created_at,
            operation_count: r.operation_count,
            has_soroban: r.has_soroban,
            operation_types: r.operation_types,
        })
        .collect();

    Json(into_envelope(data, page)).into_response()
}
