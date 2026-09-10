//! Handlers for the liquidity-pool endpoints (participants from task 0126;
//! list / detail / activity / chart from tasks 0052 and 0491).

#![allow(clippy::result_large_err)]

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};

use crate::common::asset_identity::sac_strkey;
use crate::common::cache_control;
use crate::common::cursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::common::path;
use crate::common::pool_asset_codes::normalize_asset_codes;
use crate::common::strkey::{pool_id_from_text, pool_identifier};
use crate::openapi::schemas::PageInfo;
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{
    ChartParams, ChartResponse, ParticipantItem, PoolActivityCursor, PoolActivityItem,
    PoolActivityParams, PoolAssetLeg, PoolEvent, PoolItem, PoolListCursor, PoolListParams,
    SharesCursor,
};
use super::queries::{self, PoolLegRow, PoolRow, ResolvedPoolListParams};

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/participants",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — SEP-23 strkey (`L...`, 56 chars). Internal DB form is hex (ADR 0024); strkey is the canonical wire form."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated participants list",
         body = Paginated<ParticipantItem>),
        (status = 400, description = "Invalid pool_id, limit, or cursor", body = ErrorEnvelope),
        (status = 404, description = "Pool not found",  body = ErrorEnvelope),
        (status = 500, description = "Database error",  body = ErrorEnvelope),
    )
)]
pub async fn list_participants(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    pagination: Pagination<SharesCursor>,
) -> Response {
    let pool_id_hex = match path::pool_id_strkey(&pool_id, "pool_id") {
        Ok(hex) => hex,
        Err(resp) => return resp,
    };

    // Fetch limit + 1 so `finalize_page` can detect a next page without
    // a separate count query.
    let fetch_limit = pagination.fetch_limit();
    let has_predecessor = pagination.has_predecessor();
    let direction = pagination.direction;

    // 404 vs 200-empty disambiguation: a missing pool gets 404 so the
    // frontend can route to a "pool not found" page. An existing pool
    // with no current participants returns 200 with `data: []`.
    //
    // Both reads derive everything from the path — the page never consumes the
    // existence answer — so they go out together (task 0446). `exists` is still
    // what decides the 404 and is still checked first, so responses are
    // unchanged; the cost is one wasted page read when the pool is missing.
    let ch = state.ch();
    let (exists, fetched) = tokio::join!(
        queries::pool_exists(&ch, &pool_id_hex),
        queries::fetch_participants(
            &ch,
            &pool_id_hex,
            pagination.cursor.as_ref(),
            fetch_limit,
            direction,
        ),
    );
    match exists.map_err(|e| e.to_string()) {
        Ok(true) => {}
        Ok(false) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in pool_exists");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    let mut rows = match fetched.map_err(|e| e.to_string()) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_participants");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Cursor builder gets the kept tail / head row directly — both the
    // wire `shares` (NUMERIC string) and the internal
    // `account_id_surrogate` BIGINT travel inside the opaque payload,
    // never on the wire.
    let page = finalize_page(
        &mut rows,
        pagination.limit,
        direction,
        has_predecessor,
        |dir, last| {
            cursor::encode(
                &SharesCursor {
                    shares: last.shares.clone(),
                    account_id: last.account_id_surrogate,
                },
                dir,
            )
        },
    );

    let data: Vec<ParticipantItem> = rows
        .into_iter()
        .map(|r| ParticipantItem {
            account: r.account,
            shares: r.shares,
            share_percentage: r.share_percentage,
            first_deposit_ledger: r.first_deposit_ledger,
            last_updated_ledger: r.last_updated_ledger,
        })
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

// ---------------------------------------------------------------------------
// List / Detail / Activity / Chart (tasks 0052, 0491)
// ---------------------------------------------------------------------------

// `normalize_asset_codes` used to live here. It moved to
// `common::pool_asset_codes` when global search adopted the same matching rule
// (task 0470) — the needle split and the WHERE clause it feeds have to agree,
// so they now sit in one module together. The `splitn(2)` bound, the
// empty-needle drop and the pair semantics are documented there.

/// Finish one leg at the response boundary.
///
/// The SAC mirror's address is derived HERE and nowhere else (ADR 0051 — never
/// stored, never looked up), because the network id lives on the app state.
/// `/v1/assets` splits it at the same seam, through the same helper.
fn map_leg(leg: PoolLegRow, network_id: &[u8; 32]) -> PoolAssetLeg {
    PoolAssetLeg {
        asset_type_name: domain::AssetFamily::try_from(leg.family)
            .ok()
            .map(|f| f.as_str().to_string()),
        sac_contract_id: sac_strkey(
            leg.sac_observed,
            leg.asset_code.as_deref().unwrap_or_default(),
            leg.issuer.as_deref().unwrap_or_default(),
            network_id,
        ),
        asset_code: leg.asset_code,
        issuer: leg.issuer,
        contract_id: leg.contract_id,
        icon_url: leg.icon_url,
        reserve: leg.reserve,
    }
}

fn map_pool_item(row: PoolRow, network_id: &[u8; 32]) -> PoolItem {
    let kind = domain::PoolKind::try_from(row.pool_kind).ok();
    PoolItem {
        // The same 32 bytes are an `L…` strkey for a classic pool and a `C…`
        // address for a soroban one, and the wrong form is well-formed rather
        // than an error — so the encoding follows the kind. What an unreadable
        // kind renders as is `pool_identifier`'s decision, not this handler's.
        pool_id: pool_identifier(&row.pool_id_hex, row.pool_kind),
        pool_kind: kind.map(|k| k.as_str().to_string()),
        legs: row
            .legs
            .into_iter()
            .map(|l| map_leg(l, network_id))
            .collect(),
        fee_bps: row.fee_bps,
        fee_percent: row.fee_percent,
        created_at_ledger: row.created_at_ledger,
        participant_count: row.participant_count,
        latest_snapshot_ledger: row.latest_snapshot_ledger,
        reserve_a: row.reserve_a,
        reserve_b: row.reserve_b,
        total_shares: row.total_shares,
        tvl: row.tvl,
        volume: row.volume,
        fee_revenue: row.fee_revenue,
        latest_snapshot_at: row.latest_snapshot_at,
    }
}

#[utoipa::path(
    get,
    path = "/liquidity-pools",
    tag = "liquidity-pools",
    params(
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        PoolListParams,
    ),
    responses(
        (status = 200, description = "Paginated liquidity-pool list",
         body = Paginated<PoolItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_pools(
    State(state): State<AppState>,
    pagination: Pagination<PoolListCursor>,
    Query(params): Query<PoolListParams>,
) -> Response {
    // `filter[min_tvl]` is REJECTED, not ignored and not silently empty.
    //
    // Its SQL pre-filter reads `liquidity_pool_snapshots.tvl`, a column task
    // 0199 established is never written (USD is computed at read, ADR 0053),
    // so the predicate matched nothing and the endpoint answered "no pools"
    // — while the same response now carries real per-row USD `tvl`. A filter
    // that contradicts the rows it filters is worse than an absent one, so
    // callers get a 400 that says why rather than a plausible empty page.
    //
    // Restoring it needs TVL for ALL pools per request (it changes page
    // membership, so it cannot ride the per-page price lookup) — i.e. the
    // prices-side identity-keyed materialization. Until then this stays a
    // 400 and `ResolvedPoolListParams::min_tvl` stays `None`.
    if let Some(min) = params.filter_min_tvl.as_deref() {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "filter[min_tvl] is not supported: pool TVL is computed at read \
             from off-chain prices, so it cannot filter page membership. \
             Filter client-side on the `tvl` field of the returned rows.",
            serde_json::json!({ "filter": "min_tvl", "received": min }),
        );
    }

    let has_predecessor = pagination.has_predecessor();
    let direction = pagination.direction;
    // One free-text box, two things a reader can paste into it: an asset code
    // (or `A/B` pair) and a pool identifier. Try the identifier first — it is
    // the unambiguous shape, and treating it as a code found nothing, so the
    // page claimed the pool did not exist (task 0470).
    let pool_id_hex = params
        .filter_asset_code
        .as_deref()
        .and_then(pool_id_from_text);
    let asset_codes = if pool_id_hex.is_some() {
        Vec::new()
    } else {
        normalize_asset_codes(params.filter_asset_code)
    };
    // An unknown kind is a 400, never a silent pass-through: dropping the
    // filter would answer with a page that contradicts the request, which is
    // the same failure the rejected `filter[min_tvl]` produced.
    let pool_kind = match params.filter_pool_kind.as_deref() {
        None => None,
        Some(raw) => match raw.parse::<domain::PoolKind>() {
            Ok(k) => Some(k),
            Err(_) => {
                return errors::bad_request_with_details(
                    errors::INVALID_FILTER,
                    "filter[pool_kind] must be `classic` or `soroban`",
                    serde_json::json!({ "filter[pool_kind]": raw }),
                );
            }
        },
    };

    let resolved = ResolvedPoolListParams {
        limit: pagination.fetch_limit(),
        cursor: pagination.cursor,
        pool_kind,
        asset_codes,
        pool_id_hex,
    };

    // The CH list keys on `last_updated_ledger` (see
    // `queries::fetch_pool_list`); the sort key travels in
    // `PoolRow::cursor_ledger`.
    let fetched = queries::fetch_pool_list(&state.ch(), &resolved, direction)
        .await
        .map_err(|e| e.to_string());
    let mut rows = match fetched {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error in list_pools");
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
                &PoolListCursor {
                    created_at_ledger: r.cursor_ledger,
                    pool_id_hex: r.pool_id_hex.clone(),
                },
                dir,
            )
        },
    );
    let data: Vec<PoolItem> = rows
        .into_iter()
        .map(|r| map_pool_item(r, &state.network_id))
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — SEP-23 strkey (`L...`, 56 chars). Internal DB form is hex (ADR 0024); strkey is the canonical wire form."),
    ),
    responses(
        (status = 200, description = "Pool detail", body = PoolItem),
        (status = 400, description = "Invalid pool_id", body = ErrorEnvelope),
        (status = 404, description = "Pool not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_pool(State(state): State<AppState>, Path(pool_id): Path<String>) -> Response {
    let pool_id_hex = match path::pool_id_strkey(&pool_id, "pool_id") {
        Ok(hex) => hex,
        Err(resp) => return resp,
    };

    let fetched = queries::fetch_pool_by_id(&state.ch(), &pool_id_hex)
        .await
        .map_err(|e| e.to_string());
    let mut row = match fetched {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in get_pool");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // USD analytics (0199 compute-at-read): spot TVL + 24h volume/fee from
    // the in-cluster `prices.*` views. Deliberately DEGRADES to NULL fields
    // on error instead of failing the whole detail — the pool's on-chain
    // data is still valid without prices, and the FE already renders the
    // NULL ("stale") state. The error log is the operator signal (a missing
    // `prices.*` SELECT grant lands here, not in a 500).
    let ctx = queries::PoolPriceContext {
        legs: row
            .legs
            .iter()
            .map(|l| queries::price_leg(l.family, l.asset_code.as_deref(), l.issuer.as_deref()))
            .collect(),
        fee_bps: row.fee_bps,
    };
    match queries::fetch_pool_usd_analytics(
        &state.ch(),
        &pool_id_hex,
        &ctx,
        row.reserve_a.as_deref(),
        row.reserve_b.as_deref(),
    )
    .await
    {
        Ok(analytics) => {
            row.tvl = analytics.tvl;
            row.volume = analytics.volume;
            row.fee_revenue = analytics.fee_revenue;
        }
        Err(e) => {
            tracing::error!("DB error in fetch_pool_usd_analytics({pool_id}): {e}");
        }
    }

    let mut resp = Json(map_pool_item(row, &state.network_id)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

/// The `allowed` list a `filter[event]` rejection returns. Derived from the
/// enum's own spellings rather than retyped, so it cannot advertise a value
/// `PoolEvent::from_param` would then refuse.
const ALLOWED_EVENTS: [&str; 3] = [
    PoolEvent::Trade.as_param(),
    PoolEvent::Deposit.as_param(),
    PoolEvent::Withdrawal.as_param(),
];

/// `GET /v1/liquidity-pools/{pool_id}/activity` — the pool's operations
/// (task 0491, issue #371).
///
/// Supersedes `/transactions`, whose row was a transaction. That unit could
/// not carry an honest `Event` chip (a bundled deposit + trade collapsed to
/// one label), forced the Amount cell to stack figures that must not be
/// summed, and made a trades filter inexpressible — "trades only" has no
/// truthful answer for a transaction that deposits *and* trades. The old path
/// stays mounted until the frontend moves to this one (task 0491 step 3),
/// which is also when its handler, DTO and query go.
#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/activity",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — SEP-23 strkey (`L...`, 56 chars)."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ("filter[event]" = Option<String>, Query,
         description = "Restrict to `trade`, `deposit` or `withdrawal`."),
    ),
    responses(
        (status = 200, description = "Paginated pool activity, one row per operation",
         body = Paginated<PoolActivityItem>),
        (status = 400, description = "Invalid pool_id, limit, cursor, or event", body = ErrorEnvelope),
        (status = 404, description = "Pool not found",  body = ErrorEnvelope),
        (status = 500, description = "Database error",  body = ErrorEnvelope),
    )
)]
pub async fn list_pool_activity(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    pagination: Pagination<PoolActivityCursor>,
    Query(params): Query<PoolActivityParams>,
) -> Response {
    let pool_id_hex = match path::pool_id_strkey(&pool_id, "pool_id") {
        Ok(hex) => hex,
        Err(resp) => return resp,
    };

    // The pool's two leg surrogates, which double as this path's existence
    // check: the driver pivots `lp_operation_amounts.asset_id` onto them, so
    // the read cannot run without them and a missing pool is one seek away
    // (task 0279's pairing, kept).
    let legs = queries::fetch_pool_asset_ids(&state.ch(), &pool_id_hex)
        .await
        .map_err(|e| e.to_string());
    let asset_ids = match legs {
        // The activity feed reads `lp_operation_amounts`, whose rows come from
        // CLASSIC operations — and a classic pool has exactly two legs. Taking
        // the first two is therefore exact for every pool this endpoint can
        // serve.
        //
        // A pool with FEWER is not a missing pool: `legs` is still being
        // backfilled, so a pool that exists can answer with an empty array
        // today (10,437 of them on production, measured 2026-09-09). Saying
        // "not found" about it contradicted the detail endpoint, which renders
        // that same pool one call earlier. It has no amount rows to map, so the
        // honest answer is an empty page — the same one a pool with no activity
        // gets.
        Ok(Some(ids)) => match ids.as_slice() {
            [a, b, ..] => (*a, *b),
            _ => return empty_activity_page(pagination.limit),
        },
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_asset_ids");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Validated here rather than by serde on the way in, so a bad value gets
    // this API's error envelope with the allowed list — same shape the chart's
    // `interval` returns.
    let event = match params.event.as_deref() {
        Some(s) => match PoolEvent::from_param(s) {
            Some(e) => Some(e),
            None => {
                return errors::bad_request_with_details(
                    errors::INVALID_FILTER,
                    "filter[event] must be one of: trade, deposit, withdrawal",
                    serde_json::json!({
                        "param": "filter[event]",
                        "received": s,
                        "allowed": ALLOWED_EVENTS,
                    }),
                );
            }
        },
        None => None,
    };

    let fetched = queries::fetch_pool_activity(
        &state.ch(),
        &pool_id_hex,
        asset_ids,
        pagination.fetch_limit(),
        pagination.cursor.as_ref(),
        pagination.direction,
        event,
    )
    .await
    .map_err(|e| e.to_string());
    let mut rows = match fetched {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_activity");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(
        &mut rows,
        pagination.limit,
        pagination.direction,
        pagination.has_predecessor(),
        |dir, r| {
            cursor::encode(
                &PoolActivityCursor {
                    ledger_sequence: r.ledger_sequence,
                    transaction_id: r.transaction_id,
                    application_order: r.application_order,
                },
                dir,
            )
        },
    );
    let data: Vec<PoolActivityItem> = rows
        .into_iter()
        .map(|r| PoolActivityItem {
            transaction_hash: r.transaction_hash,
            ledger_sequence: r.ledger_sequence,
            application_order: r.application_order,
            event: r.event,
            amount_a: r.amount_a,
            amount_b: r.amount_b,
            source_account: r.source_account,
            pools_crossed: r.pools_crossed,
            created_at: r.created_at,
        })
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}
/// The activity response for a pool that exists but has no rows to map.
///
/// Distinct from a 404 on purpose: the pool is real, and the caller asked a
/// well-formed question about it. Shares the list envelope so the frontend's
/// empty state is the one it already renders.
fn empty_activity_page(limit: u32) -> axum::response::Response {
    let page = PageInfo {
        next_cursor: None,
        prev_cursor: None,
        limit,
    };
    let mut resp = Json(into_envelope(Vec::<PoolActivityItem>::new(), page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

const ALLOWED_INTERVALS: &[&str] = &["1h", "1d", "1w"];

/// Hard cap on the number of buckets a single chart request can produce.
///
/// Without a cap a malicious / buggy caller could request a 10-year window
/// at `interval=1h` (≈ 87 600 buckets), which forces the planner into a
/// large GROUP BY + ARRAY_AGG aggregation. 1 000 buckets covers every
/// realistic UI need (≈ 41 days at 1h, ≈ 2.7 years at 1d, ≈ 19 years at
/// 1w) and stays cheap on the snapshots index.
const MAX_CHART_BUCKETS: i64 = 1_000;

/// Approximate bucket width in seconds for each allowlisted interval.
/// Used only for the bucket-count guard before SQL — `date_trunc`
/// computes the actual buckets.
fn interval_seconds(interval: &str) -> i64 {
    match interval {
        "1h" => 3_600,
        "1d" => 86_400,
        "1w" => 604_800,
        // unreachable — handler validates against ALLOWED_INTERVALS first.
        _ => 1,
    }
}

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/chart",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — SEP-23 strkey (`L...`, 56 chars)."),
        ChartParams,
    ),
    responses(
        (status = 200, description = "Time-bucketed pool chart series", body = ChartResponse),
        (status = 400, description = "Invalid pool_id / interval / from / to", body = ErrorEnvelope),
        (status = 404, description = "Pool not found", body = ErrorEnvelope),
        (status = 500, description = "Database error", body = ErrorEnvelope),
    ),
)]
pub async fn get_pool_chart(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    Query(params): Query<ChartParams>,
) -> Response {
    let pool_id_hex = match path::pool_id_strkey(&pool_id, "pool_id") {
        Ok(hex) => hex,
        Err(resp) => return resp,
    };

    // All three params are optional. Defaults are tuned per interval so a
    // bare `?` request produces a useful chart without bucket-cap
    // violations:
    //   1h → last 7 days     (168 buckets)
    //   1d → last 90 days    ( 90 buckets, ≈ 3 months)
    //   1w → last 104 weeks  (104 buckets, ≈ 2 years)
    let interval = match params.interval.as_deref() {
        Some(s) if ALLOWED_INTERVALS.contains(&s) => s.to_string(),
        Some(s) => {
            return errors::bad_request_with_details(
                errors::INVALID_FILTER,
                "interval must be one of: 1h, 1d, 1w",
                serde_json::json!({
                    "param": "interval",
                    "received": s,
                    "allowed": ALLOWED_INTERVALS,
                }),
            );
        }
        None => "1d".to_string(),
    };

    let to = match params.to.as_deref() {
        Some(v) => match filters::parse_iso8601(v, "to") {
            Ok(d) => d,
            Err(resp) => return resp,
        },
        None => chrono::Utc::now(),
    };
    let from = match params.from.as_deref() {
        Some(v) => match filters::parse_iso8601(v, "from") {
            Ok(d) => d,
            Err(resp) => return resp,
        },
        None => {
            // Default window matches the interval — see comment above.
            let back = match interval.as_str() {
                "1h" => chrono::Duration::days(7),
                "1d" => chrono::Duration::days(90),
                "1w" => chrono::Duration::weeks(104),
                _ => unreachable!("interval already validated against allowlist"),
            };
            to - back
        }
    };
    if from >= to {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "from must be strictly before to",
            serde_json::json!({ "from": from.to_rfc3339(), "to": to.to_rfc3339() }),
        );
    }

    // Bucket-count guard: reject ranges that would force the aggregation
    // beyond `MAX_CHART_BUCKETS`. `date_trunc` aligns buckets to wall-clock
    // boundaries — a span that crosses a boundary mid-interval produces
    // one extra bucket. Ceil division covers the "span just under N
    // intervals" case; `+ 1` covers the wall-clock alignment case.
    let interval_secs = interval_seconds(&interval);
    let span_seconds = (to - from).num_seconds();
    // Manual ceil division (`i64::div_ceil` is still unstable as of stable
    // Rust 2024). `+ 1` covers the wall-clock alignment edge.
    let approx_buckets = (span_seconds + interval_secs - 1) / interval_secs + 1;
    if approx_buckets > MAX_CHART_BUCKETS {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            format!(
                "(to - from) at interval={interval} would produce ~{approx_buckets} buckets; \
                 maximum is {MAX_CHART_BUCKETS}"
            ),
            serde_json::json!({
                "interval": interval,
                "approx_buckets": approx_buckets,
                "max_buckets": MAX_CHART_BUCKETS,
                "from": from.to_rfc3339(),
                "to": to.to_rfc3339(),
            }),
        );
    }

    // Doubles as the 404 existence gate (one row on `liquidity_pools`) and
    // supplies the leg identities + fee_bps the USD computation joins on.
    //
    // Stays SERIAL, unlike the gates in `list_participants` /
    // `list_pool_transactions` (task 0446), and for two independent reasons.
    // The chart read now CONSUMES `ctx`, so it is genuinely dependent — nothing
    // to overlap. It also could not have been paired even before that: its
    // `JOIN (SELECT … FROM ledgers WHERE closed_at …)` build side is
    // materialised even when the left side is empty, and `MAX_CHART_BUCKETS`
    // admits a ~19-year window, so speculatively running it cost a measured
    // 43.7M rows / 4.66 s for a pool that does not exist, against 16.5k rows /
    // 3.6 ms for the gate. Pool ids are user-supplied strkeys. If a future
    // change breaks the data dependency, that measurement still stands.
    let ctx = match queries::fetch_pool_price_context(&state.ch(), &pool_id_hex).await {
        Ok(Some(ctx)) => ctx,
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_price_context");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let fetched = queries::fetch_pool_chart(&state.ch(), &pool_id_hex, &ctx, &interval, from, to)
        .await
        .map_err(|e| e.to_string());
    let data_points = match fetched {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_chart");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let mut resp = Json(ChartResponse {
        pool_id,
        interval,
        from,
        to,
        data_points,
    })
    .into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

#[cfg(test)]
mod normalize_asset_code_tests {
    use super::normalize_asset_codes;

    #[test]
    fn none_passes_through() {
        assert!(normalize_asset_codes(None).is_empty());
    }

    #[test]
    fn empty_string_becomes_none() {
        assert!(normalize_asset_codes(Some(String::new())).is_empty());
        assert!(normalize_asset_codes(Some("   ".into())).is_empty());
    }

    #[test]
    fn lowercase_is_uppercased() {
        assert_eq!(normalize_asset_codes(Some("usdc".into())), ["USDC"]);
    }

    #[test]
    fn mixed_case_is_uppercased() {
        assert_eq!(normalize_asset_codes(Some("UsDc".into())), ["USDC"]);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(normalize_asset_codes(Some("  xlm  ".into())), ["XLM"]);
    }

    #[test]
    fn pair_splits_into_two_needles() {
        assert_eq!(
            normalize_asset_codes(Some("usdc/xlm".into())),
            ["USDC", "XLM"]
        );
    }

    #[test]
    fn pair_tolerates_spaces_around_the_slash() {
        assert_eq!(
            normalize_asset_codes(Some(" usdc / xlm ".into())),
            ["USDC", "XLM"]
        );
    }

    #[test]
    fn half_written_pair_keeps_the_written_half() {
        // Mid-typing state: the field debounces and fires on `USDC/`.
        assert_eq!(normalize_asset_codes(Some("USDC/".into())), ["USDC"]);
        assert_eq!(normalize_asset_codes(Some("/XLM".into())), ["XLM"]);
        assert!(normalize_asset_codes(Some("/".into())).is_empty());
    }

    #[test]
    fn third_code_stays_inside_the_second_needle() {
        // `splitn(2)` bounds the needle count. The remainder is not discarded —
        // it becomes a needle no asset code can contain, so the query returns
        // nothing rather than silently answering a narrower question.
        assert_eq!(
            normalize_asset_codes(Some("USDC/XLM/BTC".into())),
            ["USDC", "XLM/BTC"]
        );
        assert!(normalize_asset_codes(Some("/".repeat(5_000))).len() <= 2);
    }

    #[test]
    fn unicode_lower_uppercases_too() {
        // Stellar codes are ASCII-only in practice, but the normalizer
        // should not panic on UTF-8 — `String::to_uppercase` handles it.
        assert_eq!(normalize_asset_codes(Some("usdc🪙".into())), ["USDC🪙"]);
    }
}

#[cfg(test)]
mod map_pool_item_tests {
    use super::*;
    use crate::liquidity_pools::queries::{PoolLegRow, PoolRow};

    /// Mainnet's network id, so a derived SAC address is the real one rather
    /// than a value only this test would ever produce.
    fn net() -> [u8; 32] {
        xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE)
    }

    fn native_leg() -> PoolLegRow {
        PoolLegRow {
            family: domain::AssetFamily::Native as i16,
            asset_code: None,
            issuer: None,
            contract_id: None,
            sac_observed: false,
            icon_url: None,
            reserve: None,
        }
    }

    fn usdc_leg() -> PoolLegRow {
        PoolLegRow {
            family: domain::AssetFamily::ClassicCredit as i16,
            asset_code: Some("USDC".into()),
            issuer: Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".into()),
            contract_id: None,
            sac_observed: false,
            icon_url: None,
            reserve: None,
        }
    }

    fn base_row() -> PoolRow {
        PoolRow {
            pool_id_hex: "0".repeat(64),
            pool_kind: domain::PoolKind::Classic as i16,
            legs: vec![native_leg(), usdc_leg()],
            fee_bps: 30,
            fee_percent: "0.30".into(),
            created_at_ledger: Some(100),
            cursor_ledger: 100,
            participant_count: 0,
            latest_snapshot_ledger: None,
            reserve_a: None,
            reserve_b: None,
            total_shares: None,
            tvl: None,
            volume: None,
            fee_revenue: None,
            latest_snapshot_at: None,
        }
    }

    #[test]
    fn legs_carry_the_family_vocabulary_not_the_xdr_one() {
        let item = map_pool_item(base_row(), &net());
        assert_eq!(item.legs.len(), 2);
        assert_eq!(item.legs[0].asset_type_name.as_deref(), Some("native"));
        // `classic_credit`, NOT `credit_alphanum4`: one vocabulary across the
        // API, the one `/v1/assets` already speaks and the frontend maps.
        assert_eq!(
            item.legs[1].asset_type_name.as_deref(),
            Some("classic_credit")
        );
    }

    #[test]
    fn a_leg_without_an_observed_sac_gets_no_address() {
        let item = map_pool_item(base_row(), &net());
        assert_eq!(item.legs[0].sac_contract_id, None);
        assert_eq!(item.legs[1].sac_contract_id, None);
        // Neither is a soroban token, so neither names a contract of its own.
        assert_eq!(item.legs[1].contract_id, None);
    }

    /// The address is DERIVED, never stored — so an observed SAC produces one
    /// without the row carrying it.
    #[test]
    fn an_observed_sac_derives_its_address() {
        let mut row = base_row();
        row.legs[1].sac_observed = true;
        let item = map_pool_item(row, &net());
        let sac = item.legs[1]
            .sac_contract_id
            .as_deref()
            .expect("an observed SAC derives an address");
        assert!(sac.starts_with('C'), "{sac}");
        assert_eq!(sac.len(), 56);
    }

    #[test]
    fn icon_url_propagates_per_leg() {
        let mut row = base_row();
        row.legs[1].icon_url = Some("https://cdn.example.test/icons/usdc.svg".into());
        let item = map_pool_item(row, &net());
        assert_eq!(item.legs[0].icon_url, None, "native leg has no icon");
        assert_eq!(
            item.legs[1].icon_url.as_deref(),
            Some("https://cdn.example.test/icons/usdc.svg")
        );
    }

    /// Three legs are not a hypothetical — stable pools carry them on mainnet,
    /// and the pair shape this replaced could not express one.
    #[test]
    fn a_three_leg_pool_renders_all_three() {
        let mut row = base_row();
        row.legs.push(PoolLegRow {
            family: domain::AssetFamily::Soroban as i16,
            asset_code: None,
            issuer: None,
            contract_id: Some("CAQCFVLOBK5GIULPNZRGSXFPMIDUTBDDKCEHQNCZGYNK5JEN6IY5RZQB".into()),
            sac_observed: false,
            icon_url: None,
            reserve: None,
        });
        let item = map_pool_item(row, &net());
        assert_eq!(item.legs.len(), 3);
        assert_eq!(item.legs[2].asset_type_name.as_deref(), Some("soroban"));
        // A soroban token IS its contract, so that is the link target.
        assert_eq!(
            item.legs[2].contract_id.as_deref(),
            Some("CAQCFVLOBK5GIULPNZRGSXFPMIDUTBDDKCEHQNCZGYNK5JEN6IY5RZQB")
        );
    }

    /// The id encoding follows the KIND. Both forms are well-formed for the
    /// same 32 bytes, so rendering the wrong one is silent, not an error.
    #[test]
    fn the_pool_id_renders_by_kind() {
        let classic = map_pool_item(base_row(), &net());
        assert!(classic.pool_id.starts_with('L'), "{}", classic.pool_id);
        assert_eq!(classic.pool_kind.as_deref(), Some("classic"));

        let mut row = base_row();
        row.pool_kind = domain::PoolKind::Soroban as i16;
        let soroban = map_pool_item(row, &net());
        assert!(soroban.pool_id.starts_with('C'), "{}", soroban.pool_id);
        assert_eq!(soroban.pool_kind.as_deref(), Some("soroban"));
    }
}
