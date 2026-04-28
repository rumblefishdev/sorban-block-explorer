//! Axum handlers for the contracts endpoints.
//!
//! Pattern (mirrors `transactions/handlers.rs`):
//!   1. validate path / query / cursor input → `400` on shape errors,
//!   2. resolve the contract row (404 on miss),
//!   3. for paginated routes, fetch a `+1` page from the appearance index,
//!      compute `has_more`, and build the next cursor,
//!   4. for read-time XDR routes (E13 / E14), fan out one S3 GET per unique
//!      ledger via `StellarArchiveFetcher::fetch_ledgers`, parse once per
//!      ledger, and expand each appearance row into per-node items.
//!
//! ADR 0029 graceful-degradation rule: if an S3 fetch fails for any ledger
//! in a page we drop the affected appearance rows from the response (a `warn`
//! log is emitted) but still return the rest. Cursor advancement is **not**
//! independent of fetch outcomes — it stops at the last appearance row that
//! expanded successfully *consecutively from the start of the page*, so a
//! transient public-archive failure does not create a permanent hole when
//! the client follows the returned cursor (the next request restarts past
//! the last good row and replays the failed-region rows). `has_more` is
//! `true` whenever the page was truncated for this reason, signalling the
//! client to keep paging.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use domain::{ContractEventType, ContractType};
use stellar_xdr::curr::{LedgerCloseMeta, TransactionEnvelope, TransactionMeta};

use crate::common::{errors, path};
use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::state::AppState;
use crate::stellar_archive::extractors::collect_tx_metas;

use super::cursor;
use super::dto::{
    ContractDetailResponse, ContractStats, EventItem, InterfaceFunction, InterfaceParam,
    InterfaceResponse, InvocationItem, ListParams,
};
use super::queries::{
    EventAppearanceRow, InvocationAppearanceRow, fetch_contract, fetch_contract_stats,
    fetch_event_appearances, fetch_invocation_appearances, fetch_wasm_interface,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode a `SMALLINT` `contract_type` into its label. Unknown discriminants
/// degrade to `None` rather than failing the response — they indicate a
/// schema drift that should be visible in logs but not break the endpoint.
fn contract_type_label(value: Option<i16>) -> Option<String> {
    let raw = value?;
    match ContractType::try_from(raw) {
        Ok(t) => Some(t.to_string()),
        Err(_) => {
            tracing::warn!("unknown contract_type discriminant {raw}; returning null");
            None
        }
    }
}

/// Build the `(unique_ledger_sequences, ledger_meta_map)` pair for a page of
/// appearance rows. Out-of-`u32`-range sequences are skipped with a warn so
/// they neither corrupt the fetch batch nor wrap silently into another ledger.
async fn fetch_unique_ledgers(
    state: &AppState,
    sequences: &[i64],
) -> HashMap<u32, LedgerCloseMeta> {
    let unique_seqs: Vec<u32> = {
        let mut seen = HashSet::new();
        sequences
            .iter()
            .filter_map(|s| match u32::try_from(*s) {
                Ok(seq) => seen.insert(seq).then_some(seq),
                Err(_) => {
                    tracing::warn!("skipping out-of-u32-range ledger_sequence {s}");
                    None
                }
            })
            .collect()
    };

    let results = state.fetcher.fetch_ledgers(&unique_seqs).await;
    unique_seqs
        .into_iter()
        .zip(results)
        .filter_map(|(seq, res)| match res {
            Ok(meta) => Some((seq, meta)),
            Err(e) => {
                tracing::warn!("failed to fetch ledger {seq} from public archive: {e}");
                None
            }
        })
        .collect()
}

/// Per-ledger memoisation of the parser outputs reused by both the events and
/// invocations expansion paths. Computed lazily per request.
struct ParsedLedger<'a> {
    ledger_sequence: u32,
    closed_at: i64,
    extracted_txs: Vec<xdr_parser::ExtractedTransaction>,
    tx_metas: Vec<&'a TransactionMeta>,
    /// `tx_hash` → index in `extracted_txs` / `tx_metas` / `envelopes`.
    /// Built once so each appearance row is O(1) instead of an O(N) scan
    /// across the ledger's transactions.
    tx_index: HashMap<String, usize>,
    /// Only populated when invocation expansion is needed.
    envelopes: Option<Vec<TransactionEnvelope>>,
}

impl<'a> ParsedLedger<'a> {
    fn new(meta: &'a LedgerCloseMeta, want_envelopes: bool) -> Option<Self> {
        let ledger = match xdr_parser::extract_ledger(meta) {
            Ok(l) => l,
            Err(e) => {
                // Distinct from "S3 fetch failed" so operators can tell the
                // two failure modes apart in logs.
                tracing::warn!("failed to extract ledger header from fetched LedgerCloseMeta: {e}");
                return None;
            }
        };
        let extracted_txs =
            xdr_parser::extract_transactions(meta, ledger.sequence, ledger.closed_at);
        let tx_metas = collect_tx_metas(meta);
        let tx_index: HashMap<String, usize> = extracted_txs
            .iter()
            .enumerate()
            .map(|(i, t)| (t.hash.clone(), i))
            .collect();
        let envelopes = want_envelopes.then(|| xdr_parser::envelope::extract_envelopes(meta));
        Some(Self {
            ledger_sequence: ledger.sequence,
            closed_at: ledger.closed_at,
            extracted_txs,
            tx_metas,
            tx_index,
            envelopes,
        })
    }

    fn tx_index_by_hash(&self, tx_hash: &str) -> Option<usize> {
        self.tx_index.get(tx_hash).copied()
    }
}

/// Materialise every ledger in `ledger_map` into a `ParsedLedger` once,
/// keyed by sequence, so each appearance row can pull its tx slice without
/// re-parsing the surrounding ledger.
fn build_parsed_ledgers<'a>(
    ledger_map: &'a HashMap<u32, LedgerCloseMeta>,
    want_envelopes: bool,
) -> HashMap<u32, ParsedLedger<'a>> {
    ledger_map
        .iter()
        .filter_map(|(seq, meta)| {
            let parsed = ParsedLedger::new(meta, want_envelopes)?;
            Some((*seq, parsed))
        })
        .collect()
}

/// Validation outcome returned by [`resolve_list_params`]. Replaces a
/// `Result<_, Response>` to keep the `Err` arm small (`clippy::result_large_err`).
enum ListParamsOutcome {
    Ok(u32, Option<(DateTime<Utc>, i64)>),
    BadRequest {
        code: &'static str,
        message: &'static str,
    },
}

/// Validate `limit` and decode `cursor` from `ListParams`.
fn resolve_list_params(params: &ListParams) -> ListParamsOutcome {
    let raw_limit = params.limit.unwrap_or(20);
    if raw_limit == 0 || raw_limit > 100 {
        return ListParamsOutcome::BadRequest {
            code: "invalid_limit",
            message: "limit must be between 1 and 100",
        };
    }
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(s) => match cursor::decode(s) {
            Ok(v) => Some(v),
            Err(_) => {
                return ListParamsOutcome::BadRequest {
                    code: "invalid_cursor",
                    message: "cursor is malformed",
                };
            }
        },
    };
    ListParamsOutcome::Ok(raw_limit, cursor)
}

// ---------------------------------------------------------------------------
// GET /v1/contracts/:contract_id  (E10)
// ---------------------------------------------------------------------------

/// Get a single contract's metadata + aggregate stats.
///
/// Cached for `cache::CACHE_TTL` (45 s) per Lambda warm container so repeated
/// detail-page hits avoid the stats aggregate.
#[utoipa::path(
    get,
    path = "/contracts/{contract_id}",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
    ),
    responses(
        (status = 200, description = "Contract detail", body = ContractDetailResponse),
        (status = 400, description = "Invalid contract_id",  body = ErrorEnvelope),
        (status = 404, description = "Contract not found",  body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_contract(
    State(state): State<AppState>,
    Path(contract_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }

    if let Some(cached) = state.contract_cache.get(&contract_id) {
        return Json((*cached).clone()).into_response();
    }

    let contract = match fetch_contract(&state.db, &contract_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return errors::not_found("contract not found"),
        Err(e) => {
            tracing::error!("DB error fetching contract {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let (invocation_count, event_count) = match fetch_contract_stats(&state.db, contract.id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("DB error fetching stats for {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let response = ContractDetailResponse {
        contract_id: contract.contract_id,
        wasm_hash: contract.wasm_hash,
        deployer_account: contract.deployer_account,
        deployed_at_ledger: contract.deployed_at_ledger,
        contract_type: contract_type_label(contract.contract_type),
        is_sac: contract.is_sac,
        metadata: contract.metadata,
        stats: ContractStats {
            invocation_count,
            event_count,
        },
    };

    let cached = state.contract_cache.put(contract_id, response);
    Json((*cached).clone()).into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/contracts/:contract_id/interface  (E11)
// ---------------------------------------------------------------------------

/// Get a contract's public function signatures.
///
/// Source: `wasm_interface_metadata.metadata` JSONB, written at ingestion
/// from the `contractspecv0` WASM custom section.
#[utoipa::path(
    get,
    path = "/contracts/{contract_id}/interface",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
    ),
    responses(
        (status = 200, description = "Public function signatures", body = InterfaceResponse),
        (status = 400, description = "Invalid contract_id",  body = ErrorEnvelope),
        (status = 404, description = "Contract / interface not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_interface(
    State(state): State<AppState>,
    Path(contract_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }

    let metadata = match fetch_wasm_interface(&state.db, &contract_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return errors::not_found("contract not found or no interface metadata available");
        }
        Err(e) => {
            tracing::error!("DB error fetching interface for {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    Json(map_interface(metadata)).into_response()
}

/// Map the persisted `wasm_interface_metadata.metadata` JSONB onto the API
/// response shape. The persisted blob is `{ "functions": [...], "wasm_byte_len": N }`
/// where each function carries `{ name, doc, inputs[{name, type_name}], outputs[String] }`.
fn map_interface(blob: serde_json::Value) -> InterfaceResponse {
    let functions = blob
        .get("functions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let functions = functions
        .into_iter()
        .map(|f| {
            let name = f
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let parameters = f
                .get("inputs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|p| InterfaceParam {
                            name: p
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            type_name: p
                                .get("type_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let return_type = f
                .get("outputs")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            InterfaceFunction {
                name,
                parameters,
                return_type,
            }
        })
        .collect();

    InterfaceResponse { functions }
}

// ---------------------------------------------------------------------------
// GET /v1/contracts/:contract_id/invocations  (E13)
// ---------------------------------------------------------------------------

/// List invocation-tree nodes for a contract, paginated by appearance row.
///
/// Page granularity is one appearance per `limit`; each appearance expands
/// into the per-node items emitted by `xdr_parser::extract_invocations`
/// filtered to nodes targeting the requested `contract_id`. The returned
/// `data.len()` may exceed `limit`.
#[utoipa::path(
    get,
    path = "/contracts/{contract_id}/invocations",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated invocation history",
         body = Paginated<InvocationItem>),
        (status = 400, description = "Invalid contract_id / limit / cursor", body = ErrorEnvelope),
        (status = 404, description = "Contract not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_invocations(
    State(state): State<AppState>,
    Path(contract_id): Path<String>,
    Query(params): Query<ListParams>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }
    let (raw_limit, cursor_pair) = match resolve_list_params(&params) {
        ListParamsOutcome::Ok(limit, cursor) => (limit, cursor),
        ListParamsOutcome::BadRequest { code, message } => {
            return errors::bad_request(code, message);
        }
    };

    let contract = match fetch_contract(&state.db, &contract_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return errors::not_found("contract not found"),
        Err(e) => {
            tracing::error!("DB error fetching contract {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let mut rows: Vec<InvocationAppearanceRow> = match fetch_invocation_appearances(
        &state.db,
        contract.id,
        i64::from(raw_limit),
        cursor_pair,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_invocations: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let db_had_more = rows.len() > raw_limit as usize;
    if db_had_more {
        rows.truncate(raw_limit as usize);
    }

    let sequences: Vec<i64> = rows.iter().map(|r| r.ledger_sequence).collect();
    let ledger_map = fetch_unique_ledgers(&state, &sequences).await;
    let parsed = build_parsed_ledgers(&ledger_map, /* want_envelopes */ true);

    let expanded = expand_invocations(&rows, &parsed, &contract_id);
    let next_cursor = expansion_cursor(&rows, &expanded, params.cursor.as_deref());
    // `has_more` is true if either the DB had more rows past this page OR
    // we stopped expanding mid-page (the unexpanded tail must be retried).
    let stopped_short = expanded.last_consecutive_idx.map_or(
        // No row expanded — only "more" if the page actually had any rows.
        !rows.is_empty(),
        |idx| idx + 1 < rows.len(),
    );
    let has_more = db_had_more || stopped_short;

    Json(Paginated {
        data: expanded.items,
        page: PageInfo {
            cursor: next_cursor,
            limit: raw_limit,
            has_more,
        },
    })
    .into_response()
}

/// Result of expanding a page of appearance rows into per-node items.
///
/// `last_consecutive_idx` is the highest `i` such that every row in
/// `rows[0..=i]` was expanded successfully. This is what the cursor
/// advances past — failures stop the walk so the client can retry the
/// unexpanded tail without losing items from a transient archive outage.
struct ExpandedPage<T> {
    items: Vec<T>,
    last_consecutive_idx: Option<usize>,
}

fn expand_invocations(
    rows: &[InvocationAppearanceRow],
    parsed: &HashMap<u32, ParsedLedger<'_>>,
    contract_id: &str,
) -> ExpandedPage<InvocationItem> {
    let mut items = Vec::with_capacity(rows.len());
    let mut last_consecutive_idx: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        let Ok(seq) = u32::try_from(row.ledger_sequence) else {
            tracing::warn!(
                "out-of-u32-range ledger_sequence {} on invocation row — \
                 stopping page expansion to avoid skipping items",
                row.ledger_sequence
            );
            break;
        };
        let Some(ledger) = parsed.get(&seq) else {
            // Either the S3 fetch failed (already warned) or the parse
            // failed (warned in ParsedLedger::new). Stop here so the cursor
            // does not advance past unexpanded rows.
            break;
        };
        let Some(idx) = ledger.tx_index_by_hash(&row.transaction_hash) else {
            tracing::warn!(
                "tx {} missing from fetched ledger {} — stopping invocation page expansion",
                row.transaction_hash,
                ledger.ledger_sequence
            );
            break;
        };
        let Some(envelopes) = ledger.envelopes.as_ref() else {
            break;
        };
        let Some(envelope) = envelopes.get(idx) else {
            break;
        };
        let Some(ext_tx) = ledger.extracted_txs.get(idx) else {
            break;
        };
        let tx_meta = ledger.tx_metas.get(idx).copied();

        let inner = xdr_parser::envelope::inner_transaction(envelope);
        let result = xdr_parser::extract_invocations(
            &inner,
            tx_meta,
            &ext_tx.hash,
            ledger.ledger_sequence,
            ledger.closed_at,
            &ext_tx.source_account,
            ext_tx.successful,
        );

        for inv in result.invocations {
            if inv.contract_id.as_deref() != Some(contract_id) {
                continue;
            }
            items.push(InvocationItem {
                transaction_hash: row.transaction_hash.clone(),
                caller_account: inv.caller_account,
                function_name: inv.function_name,
                function_args: inv.function_args,
                return_value: inv.return_value,
                successful: inv.successful,
                ledger_sequence: row.ledger_sequence,
                created_at: row.created_at,
            });
        }
        last_consecutive_idx = Some(i);
    }
    ExpandedPage {
        items,
        last_consecutive_idx,
    }
}

/// Compute `next_cursor` for an expanded page. Generic over the row type
/// so both invocation and event handlers reuse the same logic.
///
/// The cursor advances to the **last consecutively-expanded row's**
/// `(created_at, transaction_id)`. When zero rows expanded, the original
/// input `cursor` (or `None` for first-page requests) is echoed back so
/// the client retries the same page on transient archive failure.
fn expansion_cursor<T, R: AppearanceRow>(
    rows: &[R],
    expanded: &ExpandedPage<T>,
    input_cursor: Option<&str>,
) -> Option<String> {
    match expanded.last_consecutive_idx {
        Some(idx) => Some(cursor::encode(
            rows[idx].created_at(),
            rows[idx].transaction_id(),
        )),
        None => input_cursor.map(str::to_string),
    }
}

trait AppearanceRow {
    fn created_at(&self) -> DateTime<Utc>;
    fn transaction_id(&self) -> i64;
}

impl AppearanceRow for InvocationAppearanceRow {
    fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
    fn transaction_id(&self) -> i64 {
        self.transaction_id
    }
}

impl AppearanceRow for EventAppearanceRow {
    fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
    fn transaction_id(&self) -> i64 {
        self.transaction_id
    }
}

// ---------------------------------------------------------------------------
// GET /v1/contracts/:contract_id/events  (E14)
// ---------------------------------------------------------------------------

/// List non-diagnostic events emitted by a contract, paginated by appearance
/// row. Each appearance expands into the matching events extracted from XDR.
#[utoipa::path(
    get,
    path = "/contracts/{contract_id}/events",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated event history",
         body = Paginated<EventItem>),
        (status = 400, description = "Invalid contract_id / limit / cursor", body = ErrorEnvelope),
        (status = 404, description = "Contract not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_events(
    State(state): State<AppState>,
    Path(contract_id): Path<String>,
    Query(params): Query<ListParams>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }
    let (raw_limit, cursor_pair) = match resolve_list_params(&params) {
        ListParamsOutcome::Ok(limit, cursor) => (limit, cursor),
        ListParamsOutcome::BadRequest { code, message } => {
            return errors::bad_request(code, message);
        }
    };

    let contract = match fetch_contract(&state.db, &contract_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return errors::not_found("contract not found"),
        Err(e) => {
            tracing::error!("DB error fetching contract {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let mut rows: Vec<EventAppearanceRow> =
        match fetch_event_appearances(&state.db, contract.id, i64::from(raw_limit), cursor_pair)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("DB error in list_events: {e}");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    let db_had_more = rows.len() > raw_limit as usize;
    if db_had_more {
        rows.truncate(raw_limit as usize);
    }

    let sequences: Vec<i64> = rows.iter().map(|r| r.ledger_sequence).collect();
    let ledger_map = fetch_unique_ledgers(&state, &sequences).await;
    let parsed = build_parsed_ledgers(&ledger_map, /* want_envelopes */ false);

    let expanded = expand_events(&rows, &parsed, &contract_id);
    let next_cursor = expansion_cursor(&rows, &expanded, params.cursor.as_deref());
    let stopped_short = expanded
        .last_consecutive_idx
        .map_or(!rows.is_empty(), |idx| idx + 1 < rows.len());
    let has_more = db_had_more || stopped_short;

    Json(Paginated {
        data: expanded.items,
        page: PageInfo {
            cursor: next_cursor,
            limit: raw_limit,
            has_more,
        },
    })
    .into_response()
}

fn expand_events(
    rows: &[EventAppearanceRow],
    parsed: &HashMap<u32, ParsedLedger<'_>>,
    contract_id: &str,
) -> ExpandedPage<EventItem> {
    let mut items = Vec::with_capacity(rows.len());
    let mut last_consecutive_idx: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        let Ok(seq) = u32::try_from(row.ledger_sequence) else {
            tracing::warn!(
                "out-of-u32-range ledger_sequence {} on event row — \
                 stopping page expansion to avoid skipping items",
                row.ledger_sequence
            );
            break;
        };
        let Some(ledger) = parsed.get(&seq) else {
            break;
        };
        let Some(idx) = ledger.tx_index_by_hash(&row.transaction_hash) else {
            tracing::warn!(
                "tx {} missing from fetched ledger {} — stopping event page expansion",
                row.transaction_hash,
                ledger.ledger_sequence
            );
            break;
        };
        let Some(tm) = ledger.tx_metas.get(idx).copied() else {
            break;
        };
        let events = xdr_parser::extract_events(
            tm,
            &row.transaction_hash,
            ledger.ledger_sequence,
            ledger.closed_at,
        );
        for event in events {
            if event.contract_id.as_deref() != Some(contract_id) {
                continue;
            }
            if event.event_type == ContractEventType::Diagnostic {
                continue;
            }
            let topics = match event.topics {
                serde_json::Value::Array(a) => a,
                other => vec![other],
            };
            items.push(EventItem {
                transaction_hash: row.transaction_hash.clone(),
                event_type: event.event_type.to_string(),
                topics,
                data: event.data,
                ledger_sequence: row.ledger_sequence,
                created_at: row.created_at,
            });
        }
        last_consecutive_idx = Some(i);
    }
    ExpandedPage {
        items,
        last_consecutive_idx,
    }
}
