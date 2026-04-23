//! Axum handlers for the transactions endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use domain::OperationType;

use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::state::AppState;
use crate::stellar_archive::dto::XdrOperationDto;
use crate::stellar_archive::extractors::extract_e3_heavy;

use super::cursor;
use super::dto::{
    DetailParams, EventItem, ListParams, OperationItem, TransactionDetailLight, TransactionListItem,
};
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
        .zip(ledger_results.into_iter())
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
                .and_then(|meta| extract_e3_heavy(meta, &row.hash))
                .map(|h| (h.memo_type, h.memo))
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
/// Both views fetch the parent ledger from the public Stellar archive
/// (per ADR 0029) so the response always includes `memo`, `result_code`,
/// `operation_tree`, `events`, and per-op `function_name`. Add
/// `?view=advanced` to additionally include `envelope_xdr`, `result_xdr`,
/// and per-op `raw_parameters`. On upstream failure all XDR-sourced
/// fields degrade gracefully to `null` / empty.
#[utoipa::path(
    get,
    path = "/transactions/{hash}",
    tag = "transactions",
    params(
        ("hash" = String, Path, description = "Transaction hash (64-char lowercase hex)"),
        DetailParams,
    ),
    responses(
        (status = 200, description = "Transaction detail (normal or advanced view)",
         body = TransactionDetailLight),
        (status = 400, description = "Invalid hash or view parameter", body = ErrorEnvelope),
        (status = 404, description = "Transaction not found",          body = ErrorEnvelope),
        (status = 500, description = "Internal server error",          body = ErrorEnvelope),
    ),
)]
pub async fn get_transaction(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    Query(params): Query<DetailParams>,
) -> Response {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid_hash",
            "hash must be a 64-character lowercase hexadecimal string",
        );
    }
    let hash_bytes = hex::decode(&hash).expect("validated above");

    let is_advanced = match params.view.as_deref() {
        None | Some("") => false,
        Some("advanced") => true,
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_view_param",
                "view must be 'advanced' or absent",
            );
        }
    };

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

    // ADR 0029 read path: both normal and advanced views fetch the parent
    // ledger from the public Stellar archive. Heavy fields (memo, result_code,
    // operation_tree, events, per-op function_name) come from XDR. On upstream
    // failure → graceful degradation: all XDR-sourced fields null / empty.
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

    let xdr_ops: &[XdrOperationDto] = heavy
        .as_ref()
        .map(|h| h.operations.as_slice())
        .unwrap_or(&[]);

    let events: Vec<EventItem> = heavy
        .as_ref()
        .map(|h| {
            h.contract_events
                .iter()
                .map(|e| EventItem {
                    event_type: e.event_type.clone(),
                    contract_id: e.contract_id.clone(),
                    topics: e.topics.clone(),
                    data: e.data.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let (envelope_xdr, result_xdr) = if is_advanced {
        heavy
            .as_ref()
            .map(|h| (h.envelope_xdr.clone(), h.result_xdr.clone()))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };

    let detail = TransactionDetailLight {
        hash: tx.hash,
        ledger_sequence: tx.ledger_sequence,
        source_account: tx.source_account,
        successful: tx.successful,
        fee_charged: tx.fee_charged,
        created_at: tx.created_at,
        parse_error: tx.parse_error,
        memo_type: heavy.as_ref().and_then(|h| h.memo_type.clone()),
        memo: heavy.as_ref().and_then(|h| h.memo.clone()),
        result_code: heavy.as_ref().and_then(|h| h.result_code.clone()),
        operations: build_operations(&op_rows, xdr_ops, is_advanced),
        operation_tree: heavy.as_ref().and_then(|h| h.operation_tree.clone()),
        events,
        envelope_xdr,
        result_xdr,
    };

    Json(detail).into_response()
}

/// Build the per-op response items. `function_name` is included whenever
/// XDR is available (both views); `raw_parameters` is gated on `is_advanced`.
fn build_operations(
    op_rows: &[super::queries::OpRow],
    xdr_ops: &[XdrOperationDto],
    is_advanced: bool,
) -> Vec<OperationItem> {
    op_rows
        .iter()
        .map(|op| {
            let xdr_op = xdr_ops
                .iter()
                .find(|x| x.application_order == op.application_order);
            OperationItem {
                op_type: OperationType::try_from(op.op_type)
                    .map(|t: OperationType| t.to_string())
                    .unwrap_or_else(|_| "unknown".to_string()),
                contract_id: op.contract_id.clone(),
                function_name: xdr_op
                    .and_then(|x| x.details["functionName"].as_str())
                    .map(str::to_string),
                raw_parameters: if is_advanced {
                    xdr_op.map(|x| x.details.clone())
                } else {
                    None
                },
            }
        })
        .collect()
}
