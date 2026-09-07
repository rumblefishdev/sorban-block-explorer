//! Axum handlers for the transactions endpoints. DB reads are served from
//! ClickHouse (`queries`); PG was retired (task 0244). The archive XDR
//! enrichment path (ADR 0029) sits on top of the resolved header.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use domain::OperationType;

use crate::common::cache_control;
use crate::common::conditional;
use crate::common::cursor::{self, Direction};
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::head;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::common::path;
use crate::common::strkey::pool_id_hex_to_strkey;
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::runtime_enrichment::stellar_archive::dto::{E3HeavyFields, HeavyFieldsStatus};
use crate::runtime_enrichment::stellar_archive::extractors::extract_e3_heavy;
use crate::runtime_enrichment::stellar_archive::merge::merge_e3_response;
use crate::state::AppState;

use super::dto::{
    EventAppearanceItem, InvocationAppearanceItem, ListParams, OperationItem,
    TransactionDetailLight, TransactionListItem, TxListCursor,
};
use super::queries::{
    self, EventAppearanceRow, InvocationAppearanceRow, OpRow, ResolvedListParams, TxDetailRow,
    TxListRow,
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
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated transaction list",
         body = Paginated<TransactionListItem>),
        (status = 304, description = "Not Modified — `If-None-Match` matched the current chain head (live first page only)"),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_transactions(
    State(state): State<AppState>,
    pagination: Pagination<TxListCursor>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
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

    // Reject a stale cursor minted under the retired PG backend. Its keyset
    // values are meaningless under CH, so per ADR 0008 we fail with
    // `invalid_cursor` instead of silently mis-paginating. A legacy/untagged
    // cursor already fails to decode upstream in the extractor; this guards the
    // decodes-but-wrong-intent case.
    if let Some(cursor) = &pagination.cursor
        && !cursor_matches_source(cursor)
    {
        return errors::bad_request(errors::INVALID_CURSOR, "cursor is malformed or expired");
    }

    // Conditional GET on the LIVE first page only (task 0292): the list is
    // always newest-first, so with no cursor its content is a pure function of
    // the chain head → the head is a valid `ETag`. Filtered first pages are
    // included (a filter narrows the rows but they still only change when a new
    // ledger lands, so head-keying is correct, never stale). Cursored
    // (historical) pages are excluded — head-independent. The head probe is
    // therefore paid only on the polled live request.
    let live_head = if pagination.cursor.is_none() {
        head::current_head_opt(&state).await
    } else {
        None
    };
    if let Some(head) = live_head
        && conditional::if_none_match_satisfied(&headers, head)
    {
        return conditional::not_modified(head);
    }

    let direction = pagination.direction;
    let has_predecessor = pagination.has_predecessor();
    let resolved = ResolvedListParams {
        limit: i64::from(pagination.limit),
        cursor: pagination.cursor,
        source_account: params.filter_source_account,
        contract_id: params.filter_contract_id,
        op_type,
    };

    // Fetch limit+1 rows — extra peek drives forward-continuation detection.
    let mut rows: Vec<TxListRow> =
        match fetch_list_for_source(&state, &resolved, direction, live_head).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "DB error in list_transactions");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    // Trim limit+1 → limit, derive page info with cursor built from the
    // boundary rows. The cursor payload differs by datasource (PG keys on
    // `(created_at, id)`; CH on `(ledger_sequence, id)`), but the wire
    // format stays opaque — see `TxListCursor`.
    let page = finalize_page(
        &mut rows,
        pagination.limit,
        direction,
        has_predecessor,
        |dir, r| cursor::encode(&list_cursor_for(&resolved, r), dir),
    );

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
            application_order: row.application_order,
            source_account: row.source_account,
            fee_charged: row.fee_charged,
            inner_tx_hash: row.inner_tx_hash,
            successful: row.successful,
            operation_count: row.operation_count,
            has_soroban: row.has_soroban,
            operation_types: row.operation_types,
            created_at: row.created_at,
        })
        .collect();

    // ETag on the live first page so the next poll can revalidate to `304`
    // (task 0292). Derive it from the BODY (the newest returned row's
    // `ledger_sequence`), not the pre-query `live_head`: if a ledger landed
    // between the head probe and the query, the page reflects that newer head,
    // and a strong validator must equal the bytes sent. Falls back to
    // `live_head` for an empty page. The CH path already caps the scan at
    // `live_head`, so there body_head == live_head. (Rows are newest-first on
    // the live page, so `first()` is the max sequence.)
    let body_head = data.first().map(|r| r.ledger_sequence);

    let mut resp = Json(into_envelope(data, page)).into_response();
    // LIVE (max-age=0): the home feed polls this list once per ledger; any
    // browser-cache TTL ≥ the ~5.8s cadence would batch 2-3 ledgers per
    // visible update (see common::cache_control).
    cache_control::attach(&mut resp, cache_control::LIVE);
    if let Some(h) = live_head {
        conditional::attach_etag(&mut resp, body_head.unwrap_or(h));
    }
    resp
}

/// Build the opaque list cursor for a boundary row. PG keys the list scan on
/// `(created_at, id)`. CH keys on `(ledger_sequence, <tie-break>)`, where the
/// tie-break depends on which list statement served the page — the cursor must
/// anchor the *same* keyset the next page's query will use:
///
/// - **Statement A** (no filter, the polled hot path) reads `transactions` in
///   primary-key order `(ledger_sequence, application_order)` with FINAL
///   dropped (the `read_rows` quota fix — see `queries::fetch_list`), so its
///   tie-break is `application_order`.
/// - **Statements B/C** (contract / op_type filter) drive off
///   `operations_appearances` and key on the `transactions.id` surrogate, so
///   their tie-break is `id`.
///
/// The emitted variant is tagged with the active datasource so a later request
/// can reject a cursor minted for the other backend (see `list_transactions`).
/// A cursor is not tagged with its statement: switching filters mid-pagination
/// resets the page in practice, and per ADR 0008 a stale opaque cursor that
/// anchors the wrong keyset degrades to a re-aligned page, never a hard error.
fn list_cursor_for(params: &ResolvedListParams, r: &TxListRow) -> TxListCursor {
    TxListCursor::Ch {
        ledger_sequence: r.ledger_sequence,
        tiebreak: if params.contract_id.is_none() && params.op_type.is_none() {
            i64::from(r.application_order)
        } else {
            r.id
        },
    }
}

/// True when the decoded cursor is a current (CH) cursor. A stale cursor minted
/// under the retired PG backend decodes but lacks the current `ch` intent, so
/// it is rejected with `invalid_cursor` rather than silently mis-paginating
/// (ADR 0008 fail-clean, HTTP 400).
fn cursor_matches_source(cursor: &TxListCursor) -> bool {
    matches!(cursor, TxListCursor::Ch { .. })
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
         body = crate::runtime_enrichment::stellar_archive::dto::E3Response<TransactionDetailLight>),
        (status = 400, description = "Invalid hash",          body = ErrorEnvelope),
        (status = 404, description = "Transaction not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_transaction(State(state): State<AppState>, Path(hash): Path<String>) -> Response {
    let hash = match path::parse_hash(&hash) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    // Resolve hash → transaction header via the CH two-step (hash index →
    // detail keyed by ledger_sequence); a miss at either step surfaces as 404.
    let tx = match lookup_detail_for_source(&state, &hash).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("transaction not found"),
        Err(e) => {
            tracing::error!(tx_hash = %hash, error = %e, "DB error looking up transaction detail");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Overlap the DB operations query with the heavy archive fetch (task 0330).
    // Both depend only on `tx` (resolved above) and are independent of each
    // other; the heavy path is the dominant cost (cross-region archive fetch +
    // XDR parse), so running the ops query concurrently hides its round-trip
    // under the archive latency instead of paying both serially.
    // Key the archive fetch on the resolved header hash, not the queried one:
    // on a fee-bump inner-hash lookup the queried hash never matches the
    // applied (outer) tx in the ledger meta, so the extractor would blank the
    // heavy block. `tx.hash` is always the outer hash (task 0375).
    let (op_rows_res, heavy) = tokio::join!(
        fetch_operations_for_source(&state, &tx),
        compute_heavy(&state, &tx.hash, &tx),
    );
    let op_rows: Vec<OpRow> = match op_rows_res {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(tx_hash = %tx.hash, error = %e, "DB error fetching operations");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // When the archive is unavailable, fall back to the DB-side appearance
    // index so the response stays useful. Sub-fetch errors degrade that
    // one array to `[]` rather than failing the whole detail call.
    let (participants, soroban_events, soroban_invocations) = if heavy.is_none() {
        let (p_res, e_res, i_res) = tokio::join!(
            fetch_participants_for_source(&state, &tx),
            fetch_events_for_source(&state, &tx),
            fetch_invocations_for_source(&state, &tx),
        );
        let participants = p_res.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DB fallback: fetch_participants failed");
            Vec::new()
        });
        let events = e_res
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "DB fallback: fetch_event_appearances failed");
                Vec::new()
            })
            .into_iter()
            .map(|r| EventAppearanceItem {
                contract_id: r.contract_id,
                ledger_sequence: r.ledger_sequence,
                created_at: r.created_at,
            })
            .collect();
        let invocations = i_res
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "DB fallback: fetch_invocation_appearances failed");
                Vec::new()
            })
            .into_iter()
            .map(|r| InvocationAppearanceItem {
                contract_id: r.contract_id,
                caller_account: r.caller_account,
                ledger_sequence: r.ledger_sequence,
                created_at: r.created_at,
            })
            .collect();
        (participants, events, invocations)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };

    let light = TransactionDetailLight {
        hash: tx.hash,
        ledger_sequence: tx.ledger_sequence,
        application_order: tx.application_order,
        source_account: tx.source_account,
        source_account_home_domain: tx.source_account_home_domain,
        fee_charged: tx.fee_charged,
        inner_tx_hash: tx.inner_tx_hash,
        successful: tx.successful,
        operation_count: tx.operation_count,
        has_soroban: tx.has_soroban,
        created_at: tx.created_at,
        parse_error: tx.parse_error,
        operations: db_operations(&op_rows),
        participants,
        soroban_events,
        soroban_invocations,
    };

    // Finalized tx + full heavy overlay → immutable per Stellar consensus.
    // Degraded response (heavy unavailable) gets short TTL so a retry can
    // pick up the archive sooner.
    let body = merge_e3_response(light, heavy);
    let cache_value = if body.heavy_fields_status == HeavyFieldsStatus::Ok {
        cache_control::LONG
    } else {
        cache_control::SHORT
    };
    let mut resp = Json(body).into_response();
    cache_control::attach(&mut resp, cache_value);
    resp
}

/// Resolve the E3 heavy block for `tx` via the ADR 0029 read path: fetch the
/// parent ledger from the public Stellar archive, then `extract_e3_heavy`. Kept
/// as its own future so the handler can `tokio::join!` it with the DB ops query
/// (the overlap hides the ops round-trip under the archive latency).
///
/// Returns `None` (→ `heavy_fields_status: "unavailable"`, graceful
/// degradation per ADR 0029) for:
///   - `tx.parse_error` rows: the indexer already failed to parse this tx's
///     XDR (lore-0190); re-fetching could mask the historical flag or emit an
///     `ok` status with NULL fields, violating the lore-0046/0044 contract
///     that such rows serve `heavy: null`. Skipping it also saves the S3
///     round-trip;
///   - an out-of-u32-range `ledger_sequence` (cannot address the archive);
///   - an archive fetch failure or a ledger that does not contain the tx.
async fn compute_heavy(state: &AppState, hash: &str, tx: &TxDetailRow) -> Option<E3HeavyFields> {
    if tx.parse_error {
        tracing::debug!(
            tx_hash = %hash,
            "skipping archive fetch for parse_error transaction; \
             surfacing heavy_fields_status = unavailable per lore-0046 contract"
        );
        return None;
    }
    let seq = match u32::try_from(tx.ledger_sequence) {
        Ok(seq) => seq,
        Err(_) => {
            tracing::warn!(
                ledger_sequence = tx.ledger_sequence,
                "out-of-u32-range ledger_sequence for tx detail; degrading to heavy = unavailable"
            );
            return None;
        }
    };

    match state
        .runtime_enrichment
        .stellar_archive
        .fetch_ledger(seq)
        .await
    {
        Ok(meta) => extract_e3_heavy(&meta, hash, &state.network_id),
        Err(e) => {
            tracing::warn!(ledger_sequence = seq, error = %e, "failed to fetch ledger for tx detail");
            None
        }
    }
}

fn db_operations(op_rows: &[OpRow]) -> Vec<OperationItem> {
    op_rows
        .iter()
        .map(|op| OperationItem {
            appearance_id: op.appearance_id,
            type_name: op.type_name.clone(),
            op_type: op.op_type,
            source_account: op.source_account.clone(),
            destination_account: op.destination_account.clone(),
            contract_id: op.contract_id.clone(),
            asset_code: op.asset_code.clone(),
            asset_issuer: op.asset_issuer.clone(),
            pool_ids: op
                .pool_ids
                .iter()
                .map(|h| pool_id_hex_to_strkey(h))
                .collect(),
            application_order: op.application_order,
            ledger_sequence: op.ledger_sequence,
            created_at: op.created_at,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-source dispatch helpers
// ---------------------------------------------------------------------------

async fn fetch_list_for_source(
    state: &AppState,
    params: &ResolvedListParams,
    direction: Direction,
    // Known chain head for the live first page (task 0292 §6) — lets the CH
    // statement inline it instead of re-deriving `max(sequence)` and pins the
    // candidate scan to the ETag'd head. `None` for cursored pages / when the
    // head was not read.
    head: Option<i64>,
) -> Result<Vec<TxListRow>, clickhouse::error::Error> {
    // Test-only audit: count actual heavy-query executions so the
    // conditional-GET tests can prove a 304 short-circuits BEFORE this runs.
    #[cfg(test)]
    state
        .list_query_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    queries::fetch_list(&state.ch(), params, direction, head).await
}

/// Resolve a tx hash to its DB header. CH keys the detail read by
/// `(ledger_sequence, hash)` resolved via `transaction_hash_index`. A miss at
/// either step is `Ok(None)` → 404.
async fn lookup_detail_for_source(
    state: &AppState,
    hash_hex: &str,
) -> Result<Option<TxDetailRow>, clickhouse::error::Error> {
    let Some(ledger_sequence) = queries::lookup_hash_ledger(&state.ch(), hash_hex).await? else {
        return Ok(None);
    };
    queries::fetch_detail(&state.ch(), hash_hex, ledger_sequence).await
}

async fn fetch_operations_for_source(
    state: &AppState,
    tx: &TxDetailRow,
) -> Result<Vec<OpRow>, clickhouse::error::Error> {
    queries::fetch_operations(&state.ch(), tx.id, tx.ledger_sequence).await
}

async fn fetch_participants_for_source(
    state: &AppState,
    tx: &TxDetailRow,
) -> Result<Vec<String>, clickhouse::error::Error> {
    queries::fetch_participants(&state.ch(), tx.id, tx.ledger_sequence).await
}

async fn fetch_events_for_source(
    state: &AppState,
    tx: &TxDetailRow,
) -> Result<Vec<EventAppearanceRow>, clickhouse::error::Error> {
    queries::fetch_event_appearances(&state.ch(), tx.id, tx.ledger_sequence).await
}

async fn fetch_invocations_for_source(
    state: &AppState,
    tx: &TxDetailRow,
) -> Result<Vec<InvocationAppearanceRow>, clickhouse::error::Error> {
    queries::fetch_invocation_appearances(&state.ch(), tx.id, tx.ledger_sequence).await
}

#[cfg(test)]
mod conditional_tests {
    //! `CH_URL`-gated conditional-GET tests for `GET /v1/transactions`.
    //! Skips cleanly when the env var is unset/unreachable. Runs against a real
    //! ClickHouse — a migrated (possibly empty) `transactions` table is enough.
    use std::sync::atomic::Ordering;

    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;
    use utoipa_axum::router::OpenApiRouter;

    use crate::common::ch::test_client_from_env;
    use crate::runtime_enrichment::RuntimeEnrichment;
    use crate::runtime_enrichment::sep1::Sep1Fetcher;
    use crate::runtime_enrichment::stellar_archive::StellarArchiveFetcher;
    use crate::state::AppState;

    fn test_state(ch: clickhouse::Client) -> AppState {
        let runtime_enrichment = RuntimeEnrichment {
            stellar_archive: StellarArchiveFetcher::new(
                crate::runtime_enrichment::stellar_archive::test_client(),
            ),
            sep1: Sep1Fetcher::new().expect("build sep1 fetcher"),
            nft_token_uri: crate::runtime_enrichment::nft_token_uri::NftTokenUriFetcher::new()
                .expect("build nft_token_uri fetcher"),
            wasm_code: crate::runtime_enrichment::wasm_code::WasmCodeFetcher::new()
                .expect("build wasm_code fetcher"),
        };
        AppState::for_tests(ch, runtime_enrichment)
    }

    fn app(state: AppState) -> axum::Router {
        let (router, _spec) = OpenApiRouter::new()
            .nest("/v1", crate::transactions::router())
            .with_state(state)
            .split_for_parts();
        router
    }

    /// The load-bearing acceptance criterion (task 0292): a matching
    /// `If-None-Match` on the live first page returns `304` with an empty body
    /// **without** running the heavy list query — asserted via the shared
    /// `list_query_count` audit counter, which only the heavy path increments.
    #[tokio::test]
    async fn live_list_304_short_circuits_before_heavy_query() {
        let Some(ch) = test_client_from_env() else {
            eprintln!("CH_URL unset — skipping tx conditional-GET test");
            return;
        };
        let state = test_state(ch);

        // 1) Live first page → 200 + ETag; the heavy query runs exactly once.
        let resp = app(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/transactions?limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let etag = resp
            .headers()
            .get(header::ETAG)
            .expect("ETag on live 200")
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            state.list_query_count.load(Ordering::Relaxed),
            1,
            "first live request must run the heavy query"
        );

        // 2) Same head via If-None-Match → 304, empty body, heavy query NOT run.
        let resp = app(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/transactions?limit=5")
                    .header(header::IF_NONE_MATCH, &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty(), "304 body must be empty");
        assert_eq!(
            state.list_query_count.load(Ordering::Relaxed),
            1,
            "304 short-circuit must NOT run the heavy query"
        );
    }
}
