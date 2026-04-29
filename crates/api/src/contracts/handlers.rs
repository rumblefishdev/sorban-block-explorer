//! Axum handlers for the contracts endpoints.
//! Mirrors the transactions / assets pattern: `common::*` for pagination,
//! errors, cursor codec, and StrKey validation (task 0043).

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use stellar_xdr::curr::{LedgerCloseMeta, TransactionMeta};
use xdr_parser::EventSource;

use crate::common::cursor::{self, TsIdCursor};
use crate::common::extractors::Pagination;
use crate::common::{errors, path};
use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::state::AppState;
use crate::stellar_archive::extractors::collect_tx_metas;

use super::dto::{
    ContractDetailResponse, ContractStats, EventItem, InterfaceResponse, InvocationItem,
};
use super::queries::{
    EventAppearanceRow, InvocationAppearanceRow, fetch_contract, fetch_contract_stats,
    fetch_event_appearances, fetch_invocation_appearances, fetch_wasm_interface,
};

/// Default time window for `fetch_contract_stats` (canonical 11 Statement B).
const STATS_WINDOW: &str = "7 days";

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

/// Parser outputs reused by the events handler. E13 dropped XDR per
/// canonical 13's "out of scope" note, so this is events-only now.
struct ParsedLedger<'a> {
    ledger_sequence: u32,
    closed_at: i64,
    tx_metas: Vec<&'a TransactionMeta>,
    tx_index: HashMap<String, usize>,
}

impl<'a> ParsedLedger<'a> {
    fn new(meta: &'a LedgerCloseMeta, network_id: &[u8; 32]) -> Option<Self> {
        let ledger = xdr_parser::extract_ledger(meta);
        let extracted_txs =
            xdr_parser::extract_transactions(meta, ledger.sequence, ledger.closed_at, network_id);
        let tx_metas = collect_tx_metas(meta);
        let tx_index: HashMap<String, usize> = extracted_txs
            .iter()
            .enumerate()
            .map(|(i, t)| (t.hash.clone(), i))
            .collect();
        Some(Self {
            ledger_sequence: ledger.sequence,
            closed_at: ledger.closed_at,
            tx_metas,
            tx_index,
        })
    }

    fn tx_index_by_hash(&self, tx_hash: &str) -> Option<usize> {
        self.tx_index.get(tx_hash).copied()
    }
}

fn build_parsed_ledgers<'a>(
    ledger_map: &'a HashMap<u32, LedgerCloseMeta>,
    network_id: &[u8; 32],
) -> HashMap<u32, ParsedLedger<'a>> {
    ledger_map
        .iter()
        .filter_map(|(seq, meta)| {
            let parsed = ParsedLedger::new(meta, network_id)?;
            Some((*seq, parsed))
        })
        .collect()
}

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

    let (recent_invocations, recent_unique_callers, stats_window) =
        match fetch_contract_stats(&state.db, contract.id, STATS_WINDOW).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("DB error fetching stats for {contract_id}: {e}");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };

    let response = ContractDetailResponse {
        contract_id: contract.contract_id,
        wasm_hash: contract.wasm_hash,
        wasm_uploaded_at_ledger: contract.wasm_uploaded_at_ledger,
        deployer: contract.deployer,
        deployed_at_ledger: contract.deployed_at_ledger,
        contract_type_name: contract.contract_type_name,
        contract_type: contract.contract_type,
        is_sac: contract.is_sac,
        metadata: contract.metadata,
        stats: ContractStats {
            recent_invocations,
            recent_unique_callers,
            stats_window,
        },
    };

    state
        .contract_cache
        .insert(contract_id, std::sync::Arc::new(response.clone()));
    Json(response).into_response()
}

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
        (status = 404, description = "Contract not found", body = ErrorEnvelope),
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

    // 200 + interface_metadata=null for SAC / pre-upload / stub-only;
    // 404 only when the contract row itself is missing.
    let row = match fetch_wasm_interface(&state.db, &contract_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("contract not found"),
        Err(e) => {
            tracing::error!("DB error fetching interface for {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    Json(InterfaceResponse {
        contract_id: row.contract_id,
        wasm_hash: row.wasm_hash,
        interface_metadata: row.interface_metadata,
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/contracts/{contract_id}/invocations",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
        ("limit" = Option<u32>, Query, description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated invocation appearance index",
         body = Paginated<InvocationItem>),
        (status = 400, description = "Invalid contract_id / limit / cursor", body = ErrorEnvelope),
        (status = 404, description = "Contract not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_invocations(
    State(state): State<AppState>,
    pagination: Pagination<TsIdCursor>,
    Path(contract_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }

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
        i64::from(pagination.limit),
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_invocations: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let db_had_more = rows.len() > pagination.limit as usize;
    if db_had_more {
        rows.truncate(pagination.limit as usize);
    }
    let next_cursor = if db_had_more {
        rows.last()
            .map(|r| cursor::encode(&TsIdCursor::new(r.created_at, r.transaction_id)))
    } else {
        None
    };

    let data: Vec<InvocationItem> = rows
        .into_iter()
        .map(|row| InvocationItem {
            transaction_hash: row.transaction_hash,
            ledger_sequence: row.ledger_sequence,
            caller_account: row.caller_account,
            amount: row.amount,
            created_at: row.created_at,
            successful: row.successful,
        })
        .collect();

    Json(Paginated {
        data,
        page: PageInfo {
            cursor: next_cursor,
            limit: pagination.limit,
            has_more: db_had_more,
        },
    })
    .into_response()
}

#[utoipa::path(
    get,
    path = "/contracts/{contract_id}/events",
    tag = "contracts",
    params(
        ("contract_id" = String, Path, description = "Contract StrKey (C…, 56 chars)"),
        ("limit" = Option<u32>, Query, description = "Items per page (1–100, default 20)."),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor from a previous response."),
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
    pagination: Pagination<TsIdCursor>,
    Path(contract_id): Path<String>,
) -> Response {
    if let Err(resp) = path::strkey(&contract_id, 'C', "contract_id") {
        return resp;
    }

    let contract = match fetch_contract(&state.db, &contract_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return errors::not_found("contract not found"),
        Err(e) => {
            tracing::error!("DB error fetching contract {contract_id}: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let mut rows: Vec<EventAppearanceRow> = match fetch_event_appearances(
        &state.db,
        contract.id,
        i64::from(pagination.limit),
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_events: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let db_had_more = rows.len() > pagination.limit as usize;
    if db_had_more {
        rows.truncate(pagination.limit as usize);
    }

    let sequences: Vec<i64> = rows.iter().map(|r| r.ledger_sequence).collect();
    let ledger_map = fetch_unique_ledgers(&state, &sequences).await;
    let parsed = build_parsed_ledgers(&ledger_map, &state.network_id);

    let expanded = expand_events(&rows, &parsed, &contract_id);
    let stopped_short = expanded
        .last_consecutive_idx
        .map_or(!rows.is_empty(), |idx| idx + 1 < rows.len());
    let has_more = db_had_more || stopped_short;

    // Cursor advances only past consecutively-expanded rows so a transient
    // archive outage never creates a permanent hole. If no row expanded,
    // echo the incoming cursor so clients retry the same page instead of
    // restarting from the top (avoids duplicates / infinite loops).
    let next_cursor = match expanded.last_consecutive_idx {
        Some(idx) => Some(cursor::encode(&TsIdCursor::new(
            rows[idx].created_at,
            rows[idx].transaction_id,
        ))),
        None => pagination.cursor.as_ref().map(cursor::encode),
    };

    Json(Paginated {
        data: expanded.items,
        page: PageInfo {
            cursor: next_cursor,
            limit: pagination.limit,
            has_more,
        },
    })
    .into_response()
}

struct ExpandedPage<T> {
    items: Vec<T>,
    /// Highest `i` such that every row in `rows[0..=i]` was expanded
    /// successfully. Cursor advances only past this index.
    last_consecutive_idx: Option<usize>,
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
                "out-of-u32-range ledger_sequence {} on event row — stopping",
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
            // Drop the entire diagnostic_events container — its
            // Contract-typed entries are byte-identical mirrors of the
            // per-op consensus events (task 0182). Filtering by inner
            // `event_type` would surface those mirrors as duplicates.
            if event.source == EventSource::Diagnostic {
                continue;
            }
            let topics = match event.topics {
                serde_json::Value::Array(a) => a,
                other => vec![other],
            };
            items.push(EventItem {
                transaction_hash: row.transaction_hash.clone(),
                ledger_sequence: row.ledger_sequence,
                transaction_id: row.transaction_id,
                successful: row.successful,
                amount: row.amount,
                created_at: row.created_at,
                event_type: event.event_type.to_string(),
                topics,
                data: event.data,
            });
        }
        last_consecutive_idx = Some(i);
    }
    ExpandedPage {
        items,
        last_consecutive_idx,
    }
}
