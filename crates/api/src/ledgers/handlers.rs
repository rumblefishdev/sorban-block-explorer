//! Axum handlers for the ledgers endpoints.
//!
//! Both endpoints are pure DB-only — list reads `ledgers` directly,
//! detail runs a header query plus a partition-pruned read of the
//! `transactions` partition for the embedded transactions[]. No archive
//! XDR fetch on either endpoint: list rows do not carry memo / heavy
//! fields (those live on the transaction detail endpoint instead).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::common::cursor::TsIdCursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::pagination::{finalize_ts_id_page, into_envelope};
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;
use crate::transactions::dto::TransactionListItem;

use super::dto::{LedgerDetailResponse, LedgerListItem};
use super::queries::{LedgerTxRow, fetch_by_sequence, fetch_list, fetch_transactions};

/// Short-TTL hint (10s) — used by the list endpoint and by the head
/// ledger's detail response. The value is pinned by the API Gateway
/// `apiGatewayCacheTtlMutable: 10` config in
/// `infra/envs/{staging,production}.json`, NOT by the ~5s Stellar
/// ledger close cadence. Lowering the header below 10s is wasted: the
/// gateway will still cache the response for its configured 10s window.
/// Raising it above 10s would expose stale data past one ledger cycle.
const CACHE_CONTROL_SHORT: HeaderValue = HeaderValue::from_static("public, max-age=10");

/// Long-TTL hint (300s) — used by closed (non-head) ledger detail
/// responses. Closed ledgers are immutable per Stellar consensus, so a
/// 5-minute browser / CDN cache is safe. The "is closed" decision uses
/// `LedgerDetailRow.next_sequence`: when a later ledger exists in DB
/// the requested one cannot still be settling. Note: an indexer
/// reindex of historical rows (e.g. tasks 0168/0169/0170) can change
/// `transaction_count` on otherwise-immutable ledger rows; cache purge
/// is the operational mitigation in that case.
const CACHE_CONTROL_LONG: HeaderValue = HeaderValue::from_static("public, max-age=300");

// ---------------------------------------------------------------------------
// GET /v1/ledgers
// ---------------------------------------------------------------------------

/// List ledgers ordered by `(closed_at DESC, sequence DESC)` with cursor
/// pagination.
#[utoipa::path(
    get,
    path = "/ledgers",
    tag = "ledgers",
    params(
        ("limit"  = Option<u32>,    Query, description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated ledger list",
         body = Paginated<LedgerListItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_ledgers(
    State(state): State<AppState>,
    pagination: Pagination<TsIdCursor>,
) -> Response {
    // Fetch limit+1 rows — the extra row drives `has_more` detection in
    // `finalize_ts_id_page` below.
    let mut rows: Vec<LedgerListItem> = match fetch_list(
        &state.db,
        i64::from(pagination.limit) + 1,
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_ledgers: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Cursor maps closed_at → ts, sequence → id. Field names are opaque
    // to the client (cursor wire format is base64(JSON), per ADR 0008).
    let page = finalize_ts_id_page(&mut rows, pagination.limit, |r| r.closed_at, |r| r.sequence);

    let mut resp = Json(into_envelope(rows, page)).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, CACHE_CONTROL_SHORT);
    resp
}

// ---------------------------------------------------------------------------
// GET /v1/ledgers/:sequence
// ---------------------------------------------------------------------------

/// Get ledger detail by sequence — header + prev/next navigation +
/// embedded paginated transactions.
///
/// Two phases, both DB-only:
///
/// 1. **DB header.** Resolve `:sequence` against `ledgers` + LATERAL
///    prev/next on `idx_ledgers_closed_at`. 404 on miss.
/// 2. **DB transactions.** Keyset-paginated read of the `transactions`
///    partition with full equality partition prune
///    (`created_at = $closed_at`).
///
/// The detail endpoint reuses the standard `?limit=` / `?cursor=` query
/// parameters to drive embedded transactions pagination. Detail itself
/// is a single resource, so no naming collision arises — the params
/// page the embedded list directly.
#[utoipa::path(
    get,
    path = "/ledgers/{sequence}",
    tag = "ledgers",
    params(
        ("sequence" = i64,            Path,  description = "Ledger sequence number"),
        ("limit"    = Option<u32>,    Query, description = "Embedded transactions page size (1–100, default 20)."),
        ("cursor"   = Option<String>, Query, description = "Embedded transactions opaque pagination cursor."),
    ),
    responses(
        (status = 200, description = "Ledger detail with embedded transactions",
         body = LedgerDetailResponse),
        (status = 400, description = "Invalid sequence format or pagination parameter", body = ErrorEnvelope),
        (status = 404, description = "Ledger not found",        body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn get_ledger(
    State(state): State<AppState>,
    Path(sequence_raw): Path<String>,
    pagination: Pagination<TsIdCursor>,
) -> Response {
    // Path param shape-validate. Path<i64> would auto-reject non-numeric
    // input with a 400, but the framework's default body is opaque — we
    // want our canonical `ErrorEnvelope` with `code = "invalid_id"`.
    let sequence: i64 = match sequence_raw.parse() {
        Ok(n) if n >= 0 => n,
        _ => {
            return errors::bad_request(
                errors::INVALID_ID,
                "sequence must be a non-negative integer",
            );
        }
    };

    // Phase 1 — DB header.
    let header_row = match fetch_by_sequence(&state.db, sequence).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found(format!("ledger with sequence {sequence} not found")),
        Err(e) => {
            tracing::error!("DB error in get_ledger header: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Phase 2 — DB embedded transactions, keyset-paginated by
    // `?limit=` / `?cursor=` query params validated above.
    let mut tx_rows: Vec<LedgerTxRow> = match fetch_transactions(
        &state.db,
        header_row.sequence,
        header_row.closed_at,
        pagination.cursor.as_ref(),
        i64::from(pagination.limit) + 1,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in get_ledger transactions: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let tx_page = finalize_ts_id_page(&mut tx_rows, pagination.limit, |r| r.created_at, |r| r.id);

    let tx_data: Vec<TransactionListItem> = tx_rows.into_iter().map(Into::into).collect();

    let body = LedgerDetailResponse {
        sequence: header_row.sequence,
        hash: header_row.hash,
        closed_at: header_row.closed_at,
        protocol_version: header_row.protocol_version,
        transaction_count: header_row.transaction_count,
        base_fee: header_row.base_fee,
        prev_sequence: header_row.prev_sequence,
        next_sequence: header_row.next_sequence,
        transactions: into_envelope(tx_data, tx_page),
    };

    // Cache-Control by chain position. `next_sequence` from LATERAL is
    // null exactly when no later ledger has closed yet — i.e. this row
    // is the chain head and the indexer may still be settling. Anything
    // older is immutable and safe to cache long.
    let cache_value = if body.next_sequence.is_none() {
        CACHE_CONTROL_SHORT
    } else {
        CACHE_CONTROL_LONG
    };

    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, cache_value);
    resp
}
