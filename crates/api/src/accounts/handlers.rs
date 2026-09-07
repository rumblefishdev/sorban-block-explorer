//! Axum handlers for the accounts endpoints. DB reads are served from
//! ClickHouse (`queries`); PG was retired (task 0244).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};

use crate::common::cache_control;
use crate::common::cursor::{self, Direction, SortOrder, parse_sort_order};
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::common::path;
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;
use crate::transactions::dto::TxListCursor;

use super::dto::{
    AccountBalance, AccountDetailResponse, AccountListItem, AccountSigner, AccountSigning,
    AccountTransactionItem, AccountTxListParams, AccountsListCursor, AccountsListParams,
};
use super::queries::{
    self, AccountBalanceRow, AccountEntryStateRow, AccountHeaderRow, AccountListRow, AccountTxRow,
    ResolvedListParams,
};

// ---------------------------------------------------------------------------
// GET /v1/accounts (list)
// ---------------------------------------------------------------------------

/// List accounts ordered by `last_seen_ledger` (the only indexed sort) —
/// newest-active first by default, oldest-first with `?order=asc`. The order
/// is sticky across pages; cursor pagination walks within it.
/// `filter[with_domain]` keeps only accounts that set a home_domain. No
/// address search — exact lookup is the global search's redirect path. Same
/// shape as the other list endpoints.
#[utoipa::path(
    get,
    path = "/accounts",
    tag = "accounts",
    params(
        ("limit"  = Option<u32>,    Query, description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from a previous response."),
        AccountsListParams,
    ),
    responses(
        (status = 200, description = "Paginated account list",
         body = Paginated<AccountListItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    pagination: Pagination<AccountsListCursor>,
    Query(params): Query<AccountsListParams>,
) -> Response {
    let sort = match parse_sort_order(params.order.as_deref()) {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    let direction = pagination.direction;
    let has_predecessor = pagination.has_predecessor();
    let resolved = ResolvedListParams {
        limit: pagination.fetch_limit(),
        cursor: pagination.cursor,
        with_domain: params.filter_with_domain.unwrap_or(false),
    };

    let mut rows: Vec<AccountListRow> =
        match fetch_list_for_source(&state, &resolved, sort, direction).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "DB error in list_accounts");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    let page = finalize_page(
        &mut rows,
        pagination.limit,
        direction,
        has_predecessor,
        |dir, r| {
            cursor::encode(
                &AccountsListCursor {
                    last_seen_ledger: r.last_seen_ledger,
                    id: r.id,
                },
                dir,
            )
        },
    );
    let data: Vec<AccountListItem> = rows.into_iter().map(map_item).collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

fn map_item(r: AccountListRow) -> AccountListItem {
    AccountListItem {
        account_id: r.account_id,
        xlm_balance: r.xlm_balance,
        last_seen_ledger: r.last_seen_ledger,
        first_seen_ledger: r.first_seen_ledger,
        home_domain: r.home_domain,
    }
}

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

    let header = match fetch_account_for_source(&state, &account_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found(format!("account '{account_id}' not found")),
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error fetching account");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Balances and deleted-status both key off `header.id` alone — neither
    // consumes the other, so they go out together (task 0446). Kept as two
    // `match` arms so each keeps its own error log line.
    let (balances_res, deleted_res, signing_res) = tokio::join!(
        fetch_account_balances(&state, header.id),
        fetch_deleted_for_source(&state, header.id),
        fetch_entry_state_for_source(&state, header.id),
    );

    let balances = match balances_res {
        Ok(rows) => rows
            .into_iter()
            .map(|r| AccountBalance {
                asset_type_name: r.asset_type_name,
                asset_type: r.asset_type,
                asset_code: r.asset_code,
                asset_issuer: r.asset_issuer,
                contract_id: r.contract_id,
                name: r.name,
                symbol: r.symbol,
                balance: r.balance,
                decimals: r.decimals,
                last_updated_ledger: r.last_updated_ledger,
                sac_deployed: r.sac_deployed,
            })
            .collect(),
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error fetching account balances");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let deleted = match deleted_res {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error deriving account deleted status");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let signing = match signing_res {
        Ok(row) => row.map(|r| AccountSigning {
            // Zip the three parallel arrays the table stores. They cannot be
            // ragged — the writer pushes to all three adjacently, after every
            // `continue` (`stage.rs`), and the whole table measures 0 ragged
            // rows out of 10.9M — so `zip` truncating to the shortest is a
            // safe default for a shape that does not occur, not a decision
            // about one that does.
            signers: r
                .signer_keys
                .into_iter()
                .zip(r.signer_weights)
                .zip(r.signer_types)
                .map(|((key, weight), signer_type)| AccountSigner {
                    key,
                    weight,
                    signer_type,
                })
                .collect(),
            master_weight: u32::from(r.master_weight),
            threshold_low: u32::from(r.threshold_low),
            threshold_med: u32::from(r.threshold_med),
            threshold_high: u32::from(r.threshold_high),
            last_updated_ledger: r.last_updated_ledger,
        }),
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error fetching account entry state");
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
        deleted,
        signing,
    };

    let mut resp = Json(body).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
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
        AccountTxListParams,
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
    pagination: Pagination<TxListCursor>,
    Path(account_id): Path<String>,
    Query(params): Query<AccountTxListParams>,
) -> Response {
    if let Err(resp) = path::strkey(&account_id, 'G', "account_id") {
        return resp;
    }

    let sort = match parse_sort_order(params.order.as_deref()) {
        Ok(s) => s,
        Err(err) => return err.into_response(),
    };

    // Reject a stale cursor minted under the retired PG backend. Its keyset is
    // meaningless under CH, so per ADR 0008 fail with `invalid_cursor` instead
    // of silently mis-paginating. A legacy/untagged cursor already fails decode
    // upstream in the extractor; this guards the decodes-but-wrong-intent case.
    if let Some(cursor) = &pagination.cursor
        && !cursor_matches_source(cursor)
    {
        return errors::bad_request(errors::INVALID_CURSOR, "cursor is malformed or expired");
    }

    let header = match fetch_account_for_source(&state, &account_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found(format!("account '{account_id}' not found")),
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error resolving account");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let direction = pagination.direction;
    let has_predecessor = pagination.has_predecessor();

    let mut rows = match fetch_account_tx_for_source(
        &state,
        header.id,
        pagination.fetch_limit(),
        pagination.cursor.as_ref(),
        sort,
        direction,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(account_id = %account_id, error = %e, "DB error in list_account_transactions");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(
        &mut rows,
        pagination.limit,
        direction,
        has_predecessor,
        |dir, r| cursor::encode(&account_tx_cursor_for(r), dir),
    );
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
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

// ---------------------------------------------------------------------------
// Per-source dispatch helpers
// ---------------------------------------------------------------------------

async fn fetch_list_for_source(
    state: &AppState,
    params: &ResolvedListParams,
    sort: SortOrder,
    direction: Direction,
) -> Result<Vec<AccountListRow>, clickhouse::error::Error> {
    queries::fetch_list(&state.ch(), params, sort, direction).await
}

async fn fetch_account_for_source(
    state: &AppState,
    account_strkey: &str,
) -> Result<Option<AccountHeaderRow>, clickhouse::error::Error> {
    queries::fetch_account(&state.ch(), account_strkey).await
}

/// `deleted` status off the native holding's lifecycle column (ADR 0055).
/// See `queries::fetch_deleted_status`.
async fn fetch_deleted_for_source(
    state: &AppState,
    account_surrogate_id: i64,
) -> Result<bool, clickhouse::error::Error> {
    queries::fetch_deleted_status(&state.ch(), account_surrogate_id).await
}

/// See `queries::fetch_entry_state` — `None` means "never observed", not
/// "no extra signers".
async fn fetch_entry_state_for_source(
    state: &AppState,
    account_id: i64,
) -> Result<Option<AccountEntryStateRow>, clickhouse::error::Error> {
    queries::fetch_entry_state(&state.ch(), account_id).await
}

async fn fetch_account_balances(
    state: &AppState,
    account_id: i64,
) -> Result<Vec<AccountBalanceRow>, clickhouse::error::Error> {
    // Balances are ClickHouse-only — the unified `balances` model is CH (task 0331).
    queries::fetch_balances(&state.ch(), account_id).await
}

async fn fetch_account_tx_for_source(
    state: &AppState,
    account_id: i64,
    limit: i64,
    cursor: Option<&TxListCursor>,
    sort: SortOrder,
    direction: Direction,
) -> Result<Vec<AccountTxRow>, clickhouse::error::Error> {
    queries::fetch_transactions(&state.ch(), account_id, limit, cursor, sort, direction).await
}

/// Build the opaque account-transactions cursor for a boundary row. CH keys on
/// `(ledger_sequence, id)` (the `transaction_participants` / `transactions`
/// keyset).
fn account_tx_cursor_for(r: &AccountTxRow) -> TxListCursor {
    TxListCursor::Ch {
        ledger_sequence: r.ledger_sequence,
        tiebreak: r.id,
    }
}

/// True when the decoded cursor is a current (CH) cursor. A stale cursor minted
/// under the retired PG backend is rejected (ADR 0008 fail-clean).
fn cursor_matches_source(cursor: &TxListCursor) -> bool {
    matches!(cursor, TxListCursor::Ch { .. })
}
