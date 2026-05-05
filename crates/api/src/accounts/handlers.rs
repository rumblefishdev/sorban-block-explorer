//! Axum handlers for the accounts endpoints. Pure DB-only.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::common::cursor::TsIdCursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::pagination::{finalize_ts_id_page, into_envelope};
use crate::common::path;
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{AccountBalance, AccountDetailResponse, AccountTransactionItem};
use super::queries::{fetch_account, fetch_balances, fetch_transactions};

/// 10s matches the API Gateway `apiGatewayCacheTtlMutable` config.
const CACHE_CONTROL_SHORT: HeaderValue = HeaderValue::from_static("public, max-age=10");

// ---------------------------------------------------------------------------
// GET /v1/accounts/:account_id
// ---------------------------------------------------------------------------

/// Account detail — header from `accounts` + balances from
/// `account_balances_current` (canonical 06 statements A + B).
#[utoipa::path(
    get,
    path = "/accounts/{account_id}",
    tag = "accounts",
    params(
        ("account_id" = String, Path, description = "Stellar account StrKey (G…, 56 chars)"),
    ),
    responses(
        (status = 200, description = "Account detail with current balances",
         body = AccountDetailResponse),
        (status = 400, description = "Invalid account_id",        body = ErrorEnvelope),
        (status = 404, description = "Account not found",         body = ErrorEnvelope),
        (status = 500, description = "Internal server error",     body = ErrorEnvelope),
    ),
)]
pub async fn get_account(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&account_id, 'G', "account_id") {
        return resp;
    }

    let header = match fetch_account(&state.db, &account_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found(format!("account '{account_id}' not found")),
        Err(e) => {
            tracing::error!("DB error fetching account {account_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let balances = match fetch_balances(&state.db, header.id).await {
        Ok(rows) => rows
            .into_iter()
            .map(|r| AccountBalance {
                asset_type_name: r.asset_type_name,
                asset_type: r.asset_type,
                asset_code: r.asset_code,
                asset_issuer: r.asset_issuer,
                balance: r.balance,
                last_updated_ledger: r.last_updated_ledger,
            })
            .collect(),
        Err(e) => {
            tracing::error!("DB error fetching balances for {account_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let body = AccountDetailResponse {
        account_id: header.account_id,
        sequence_number: header.sequence_number,
        balances,
        home_domain: header.home_domain,
        first_seen_ledger: header.first_seen_ledger,
        last_seen_ledger: header.last_seen_ledger,
    };

    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, CACHE_CONTROL_SHORT);
    resp
}

// ---------------------------------------------------------------------------
// GET /v1/accounts/:account_id/transactions
// ---------------------------------------------------------------------------

/// Paginated transactions involving the account (source or participant).
/// 404 when the StrKey is unknown — distinct from "indexed account, no
/// transactions yet" (matches assets/contracts sub-resource pattern).
#[utoipa::path(
    get,
    path = "/accounts/{account_id}/transactions",
    tag = "accounts",
    params(
        ("account_id" = String, Path, description = "Stellar account StrKey (G…, 56 chars)"),
        ("limit"  = Option<u32>,    Query, description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated transactions involving the account",
         body = Paginated<AccountTransactionItem>),
        (status = 400, description = "Invalid account_id / pagination", body = ErrorEnvelope),
        (status = 404, description = "Account not found",   body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_account_transactions(
    State(state): State<AppState>,
    pagination: Pagination<TsIdCursor>,
    Path(account_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&account_id, 'G', "account_id") {
        return resp;
    }

    let account_known = match fetch_account(&state.db, &account_id).await {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("DB error resolving account {account_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };
    if !account_known {
        return errors::not_found(format!("account '{account_id}' not found"));
    }

    let mut rows = match fetch_transactions(
        &state.db,
        &account_id,
        i64::from(pagination.limit) + 1,
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_account_transactions: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_ts_id_page(&mut rows, pagination.limit, |r| r.created_at, |r| r.id);
    let data: Vec<AccountTransactionItem> = rows
        .into_iter()
        .map(|r| AccountTransactionItem {
            hash: r.hash,
            ledger_sequence: r.ledger_sequence,
            application_order: r.application_order,
            source_account: r.source_account,
            fee_charged: r.fee_charged,
            successful: r.successful,
            operation_count: r.operation_count,
            has_soroban: r.has_soroban,
            operation_types: r.operation_types,
            created_at: r.created_at,
        })
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, CACHE_CONTROL_SHORT);
    resp
}
