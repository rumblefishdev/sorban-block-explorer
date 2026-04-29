//! Axum handlers for the transactions endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use domain::OperationType;

use crate::common::cursor::TsIdCursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_ts_id_page, into_envelope};
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;
use crate::stellar_archive::extractors::extract_e3_heavy;
use crate::stellar_archive::merge::merge_e3_response;

use super::dto::{ListParams, OperationItem, TransactionDetailLight, TransactionListItem};
use super::queries::{
    ResolvedListParams, fetch_detail, fetch_list, fetch_operations, lookup_hash_index,
};

// ---------------------------------------------------------------------------
// GET /v1/transactions
// ---------------------------------------------------------------------------

/// List transactions with optional filters and cursor-based pagination.
#[utoipa::path(
    get,
    path = "/transactions",
    tag = "transactions",
    params(
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated transaction list",
         body = Paginated<TransactionListItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_transactions(
    State(state): State<AppState>,
    pagination: Pagination<TsIdCursor>,
    Query(params): Query<ListParams>,
) -> Response {
    // Shape-validate filters before touching DB. Without these checks an
    // invalid StrKey would silently produce an empty result set, and an
    // unknown operation_type would 404 the SQL bind — both bad UX. Helpers
    // return the canonical 400 envelope on failure.
    let op_type: Option<i16> = match filters::parse_enum_opt::<OperationType>(
        params.filter_operation_type.as_deref(),
        "operation_type",
        Some("operation type"),
    ) {
        Ok(maybe) => maybe.map(|t| t as i16),
        Err(resp) => return resp,
    };
    if let Err(resp) = filters::strkey_opt(
        params.filter_source_account.as_deref(),
        'G',
        "source_account",
    ) {
        return resp;
    }
    if let Err(resp) = filters::strkey_opt(params.filter_contract_id.as_deref(), 'C', "contract_id")
    {
        return resp;
    }

    let resolved = ResolvedListParams {
        limit: i64::from(pagination.limit),
        cursor: pagination.cursor,
        source_account: params.filter_source_account,
        contract_id: params.filter_contract_id,
        op_type,
    };

    // Fetch limit+1 rows (extra row used to determine has_more).
    let mut rows: Vec<super::queries::TxListRow> = match fetch_list(&state.db, &resolved).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_transactions: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Trim limit+1 → limit, derive page info with cursor built from last row.
    let page = finalize_ts_id_page(&mut rows, pagination.limit, |r| r.created_at, |r| r.id);

    // Pure DB-only mapping — no archive XDR fetch. Memo / heavy fields
    // belong on the transaction detail endpoint (E3) inside the E3 heavy
    // block, not in the list response. Keeping the list path archive-free
    // matches canonical SQL 02's `Data sources: DB-only` contract and
    // avoids an N-fan-out fetch per page.
    let data: Vec<TransactionListItem> = rows
        .into_iter()
        .map(|row| TransactionListItem {
            hash: row.hash,
            ledger_sequence: row.ledger_sequence,
            source_account: row.source_account,
            successful: row.successful,
            fee_charged: row.fee_charged,
            created_at: row.created_at,
            operation_count: row.operation_count,
        })
        .collect();

    Json(into_envelope(data, page)).into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/transactions/:hash
// ---------------------------------------------------------------------------

/// Get a single transaction by hash.
///
/// Returns the wrapped E3 response from task 0150: the DB-sourced
/// `TransactionDetailLight` (flattened to the top level) plus a `heavy` block
/// carrying every XDR-sourced field — memo, result_code, signatures, fee-bump
/// source, envelope/result XDR, contract + diagnostic events, per-operation
/// decoded details, and the nested `operation_tree`. `heavy_fields_status` is
/// `"ok"` when the public-archive fetch succeeded and `"unavailable"` when it
/// failed (graceful degradation per ADR 0029 — the light slice is always
/// returned). Per ADR 0033 there is no separate "advanced" view; the wrapper
/// always carries the full heavy payload when available.
#[utoipa::path(
    get,
    path = "/transactions/{hash}",
    tag = "transactions",
    params(
        ("hash" = String, Path, description = "Transaction hash (64-char hex; uppercase or lowercase accepted, normalised server-side)"),
    ),
    responses(
        (status = 200, description = "Transaction detail (light + heavy block)",
         body = crate::stellar_archive::dto::E3Response<TransactionDetailLight>),
        (status = 400, description = "Invalid hash",          body = ErrorEnvelope),
        (status = 404, description = "Transaction not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_transaction(State(state): State<AppState>, Path(hash): Path<String>) -> Response {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return errors::bad_request(
            "invalid_hash",
            "hash must be a 64-character hexadecimal string",
        );
    }
    // Normalize to lowercase: extract_e3_heavy does case-sensitive matching
    // against ExtractedTransaction.hash (always lowercase hex), so an
    // uppercase request would otherwise degrade silently to heavy = None.
    let hash = hash.to_ascii_lowercase();
    let hash_bytes = hex::decode(&hash).expect("validated above");

    let index = match lookup_hash_index(&state.db, &hash_bytes).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("transaction not found"),
        Err(e) => {
            tracing::error!("DB error looking up hash index: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let tx = match fetch_detail(&state.db, &hash_bytes, index.created_at).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("transaction not found"),
        Err(e) => {
            tracing::error!("DB error fetching transaction detail: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let op_rows: Vec<super::queries::OpRow> =
        match fetch_operations(&state.db, tx.id, tx.created_at).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB error fetching operations: {e}");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    // ADR 0029 read path: fetch the parent ledger from the public Stellar
    // archive. On upstream failure → graceful degradation: heavy = None,
    // merge_e3_response sets heavy_fields_status = "unavailable" while still
    // returning the light slice from DB. Out-of-range BIGINT → u32 also
    // degrades to heavy = None rather than wrapping silently.
    let heavy = match u32::try_from(index.ledger_sequence) {
        Ok(seq) => match state.fetcher.fetch_ledger(seq).await {
            Ok(meta) => extract_e3_heavy(&meta, &hash, &state.network_id),
            Err(e) => {
                tracing::warn!("failed to fetch ledger {seq} for tx detail: {e}");
                None
            }
        },
        Err(_) => {
            tracing::warn!(
                "out-of-u32-range ledger_sequence {} for tx detail; degrading to heavy = unavailable",
                index.ledger_sequence
            );
            None
        }
    };

    let light = TransactionDetailLight {
        hash: tx.hash,
        ledger_sequence: tx.ledger_sequence,
        source_account: tx.source_account,
        successful: tx.successful,
        fee_charged: tx.fee_charged,
        created_at: tx.created_at,
        parse_error: tx.parse_error,
        operations: db_operations(&op_rows),
    };

    Json(merge_e3_response(light, heavy)).into_response()
}

/// Project DB-side operation rows onto the light `OperationItem` slice
/// (type tag + contract_id only). XDR-decoded per-op details live in
/// `heavy.operations[]`.
fn db_operations(op_rows: &[super::queries::OpRow]) -> Vec<OperationItem> {
    op_rows
        .iter()
        .map(|op| OperationItem {
            op_type: OperationType::try_from(op.op_type)
                .map(|t: OperationType| t.to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            contract_id: op.contract_id.clone(),
        })
        .collect()
}
