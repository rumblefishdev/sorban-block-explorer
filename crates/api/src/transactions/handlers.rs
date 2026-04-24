//! Axum handlers for the transactions endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use domain::OperationType;

use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::state::AppState;
use crate::stellar_archive::extractors::{extract_e3_heavy, extract_e3_memo};
use crate::stellar_archive::merge::merge_e3_response;

use super::cursor;
use super::dto::{ListParams, OperationItem, TransactionDetailLight, TransactionListItem};
use super::queries::{
    ResolvedListParams, fetch_detail, fetch_list, fetch_operations, lookup_hash_index,
    parse_op_type,
};

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

fn err(status: StatusCode, code: &str, msg: &str) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            code: code.to_string(),
            message: msg.to_string(),
            details: None,
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/transactions
// ---------------------------------------------------------------------------

/// List transactions with optional filters and cursor-based pagination.
#[utoipa::path(
    get,
    path = "/transactions",
    tag = "transactions",
    params(ListParams),
    responses(
        (status = 200, description = "Paginated transaction list",
         body = Paginated<TransactionListItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_transactions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    // Validate and clamp limit.
    let raw_limit = params.limit.unwrap_or(20);
    if raw_limit == 0 || raw_limit > 100 {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be between 1 and 100",
        );
    }

    // Decode optional cursor.
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(s) => match cursor::decode(s) {
            Ok(v) => Some(v),
            Err(_) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_cursor",
                    "cursor is malformed",
                );
            }
        },
    };

    // Validate and map filter[operation_type].
    let op_type = match params.filter_operation_type.as_deref() {
        None => None,
        Some(s) => match parse_op_type(s) {
            Ok(v) => Some(v),
            Err(_) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_filter",
                    "filter[operation_type] is not a recognized operation type",
                );
            }
        },
    };

    let resolved = ResolvedListParams {
        limit: i64::from(raw_limit),
        cursor,
        source_account: params.filter_source_account,
        contract_id: params.filter_contract_id,
        op_type,
    };

    // Fetch limit+1 rows (extra row used to determine has_more).
    let mut rows: Vec<super::queries::TxListRow> = match fetch_list(&state.db, &resolved).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_transactions: {e}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "db_error",
                "database error",
            );
        }
    };

    let has_more = rows.len() > raw_limit as usize;
    if has_more {
        rows.truncate(raw_limit as usize);
    }

    // Build next cursor from the last row on this page.
    let next_cursor = if has_more {
        rows.last().map(|r| cursor::encode(r.created_at, r.id))
    } else {
        None
    };

    // Collect unique ledger sequences and fetch XDR heavy fields concurrently
    // for memo_type / memo (ADR 0029). Failures degrade gracefully to null memo.
    let unique_seqs: Vec<u32> = {
        let mut seen = std::collections::HashSet::new();
        rows.iter()
            .filter_map(|r| {
                let seq = r.ledger_sequence as u32;
                seen.insert(seq).then_some(seq)
            })
            .collect()
    };

    let ledger_results = state.fetcher.fetch_ledgers(&unique_seqs).await;
    // Build a map: ledger_sequence → Option<LedgerCloseMeta>
    let ledger_map: std::collections::HashMap<u32, _> = unique_seqs
        .into_iter()
        .zip(ledger_results)
        .filter_map(|(seq, res)| match res {
            Ok(meta) => Some((seq, meta)),
            Err(e) => {
                tracing::warn!("failed to fetch ledger {seq} for memo extraction: {e}");
                None
            }
        })
        .collect();

    // Map DB rows → response items, merging XDR memo fields.
    let data: Vec<TransactionListItem> = rows
        .into_iter()
        .map(|row| {
            let (memo_type, memo) = ledger_map
                .get(&(row.ledger_sequence as u32))
                .and_then(|meta| extract_e3_memo(meta, &row.hash))
                .unwrap_or((None, None));

            TransactionListItem {
                hash: row.hash,
                ledger_sequence: row.ledger_sequence,
                source_account: row.source_account,
                successful: row.successful,
                fee_charged: row.fee_charged,
                created_at: row.created_at,
                operation_count: row.operation_count,
                memo_type,
                memo,
            }
        })
        .collect();

    Json(Paginated {
        data,
        page: PageInfo {
            cursor: next_cursor,
            limit: raw_limit,
            has_more,
        },
    })
    .into_response()
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
        ("hash" = String, Path, description = "Transaction hash (64-char lowercase hex)"),
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
        return err(
            StatusCode::BAD_REQUEST,
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
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", "transaction not found"),
        Err(e) => {
            tracing::error!("DB error looking up hash index: {e}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "db_error",
                "database error",
            );
        }
    };

    let tx = match fetch_detail(&state.db, &hash_bytes, index.created_at).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", "transaction not found"),
        Err(e) => {
            tracing::error!("DB error fetching transaction detail: {e}");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "db_error",
                "database error",
            );
        }
    };

    let op_rows: Vec<super::queries::OpRow> =
        match fetch_operations(&state.db, tx.id, tx.created_at).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB error fetching operations: {e}");
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "db_error",
                    "database error",
                );
            }
        };

    // ADR 0029 read path: fetch the parent ledger from the public Stellar
    // archive. On upstream failure → graceful degradation: heavy = None,
    // merge_e3_response sets heavy_fields_status = "unavailable" while still
    // returning the light slice from DB.
    let heavy = match state
        .fetcher
        .fetch_ledger(index.ledger_sequence as u32)
        .await
    {
        Ok(meta) => extract_e3_heavy(&meta, &hash),
        Err(e) => {
            tracing::warn!(
                "failed to fetch ledger {} for tx detail: {e}",
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
