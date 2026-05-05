//! Handlers for the liquidity-pool endpoints (participants from task 0126;
//! list / detail / transactions / chart from task 0052).

#![allow(clippy::result_large_err)]

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};

use crate::common::cursor;
use crate::common::cursor::TsIdCursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, finalize_ts_id_page, into_envelope};
use crate::common::path;
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{
    ChartParams, ChartResponse, ParticipantItem, PoolAssetLeg, PoolItem, PoolListCursor,
    PoolListParams, PoolTransactionItem, SharesCursor,
};
use super::queries::{
    PoolRow, ResolvedPoolListParams, fetch_participants, fetch_pool_by_id, fetch_pool_chart,
    fetch_pool_list, fetch_pool_transactions, pool_exists,
};

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/participants",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — 64-char lowercase hex (BYTEA) per ADR 0024."),
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
    if let Err(resp) = path::pool_id_hex(&pool_id, "pool_id") {
        return resp;
    }

    // 404 vs 200-empty disambiguation: a missing pool gets 404 so the
    // frontend can route to a "pool not found" page. An existing pool
    // with no current participants returns 200 with `data: []`.
    match pool_exists(&state.db, &pool_id).await {
        Ok(true) => {}
        Ok(false) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!("DB error in pool_exists({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    // Fetch limit + 1 so `finalize_page` can detect a next page without
    // a separate count query.
    let fetch_limit = i64::from(pagination.limit) + 1;
    let mut rows = match fetch_participants(
        &state.db,
        &pool_id,
        pagination.cursor.as_ref(),
        fetch_limit,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in fetch_participants({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Cursor builder gets the kept tail row directly — both the wire
    // `shares` (NUMERIC string) and the internal `account_id_surrogate`
    // BIGINT travel inside the opaque payload, never on the wire.
    let page = finalize_page(&mut rows, pagination.limit, |last| {
        cursor::encode(&SharesCursor {
            shares: last.shares.clone(),
            account_id: last.account_id_surrogate,
        })
    });

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

    Json(into_envelope(data, page)).into_response()
}

// ---------------------------------------------------------------------------
// List / Detail / Transactions / Chart (task 0052)
// ---------------------------------------------------------------------------

/// Validate `filter[min_tvl]` shape: a non-negative decimal string with
/// at least one digit and at most one `.`.
///
/// `f64::parse` accepted `NaN` / `Infinity` / scientific notation /
/// negative values that PostgreSQL `NUMERIC` either rejects later or
/// stores in a way that breaks the `>= $X::numeric` predicate semantics
/// (`NaN >= anything` is FALSE in PG, including `NaN >= NaN`). It also
/// silently widened-then-narrowed `NUMERIC(28,7)` precision.
///
/// This validator stays at the API boundary so a confused caller gets a
/// 400 envelope explaining the shape rule, instead of a Postgres parse
/// error surfacing as 500 mid-query.
fn is_valid_decimal_string(s: &str) -> bool {
    let mut digits = 0usize;
    let mut dots = 0usize;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => digits += 1,
            b'.' => dots += 1,
            _ => return false,
        }
    }
    digits > 0 && dots <= 1
}

fn map_pool_item(row: PoolRow) -> PoolItem {
    PoolItem {
        pool_id: row.pool_id_hex,
        asset_a: PoolAssetLeg {
            asset_type_name: row.asset_a_type_name,
            asset_type: row.asset_a_type,
            asset_code: row.asset_a_code,
            issuer: row.asset_a_issuer,
        },
        asset_b: PoolAssetLeg {
            asset_type_name: row.asset_b_type_name,
            asset_type: row.asset_b_type,
            asset_code: row.asset_b_code,
            issuer: row.asset_b_issuer,
        },
        fee_bps: row.fee_bps,
        fee_percent: row.fee_percent,
        created_at_ledger: row.created_at_ledger,
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
    if let Err(resp) = filters::strkey_opt(
        params.filter_asset_a_issuer.as_deref(),
        'G',
        "asset_a_issuer",
    ) {
        return resp;
    }
    if let Err(resp) = filters::strkey_opt(
        params.filter_asset_b_issuer.as_deref(),
        'G',
        "asset_b_issuer",
    ) {
        return resp;
    }

    // Asset-leg filter pairing: classic identity is `(code, issuer)`. Native
    // legs have no code AND no issuer. Mixed (one set, one absent) is
    // ambiguous — canonical SQL 18 §46-49 says "API validates inputs
    // upstream"; this is that validator. Without it, `?filter[asset_a_code]=USDC`
    // alone would match every USDC-coded pool regardless of issuer (the wrong
    // USDC issuer included).
    let a_code_set = params.filter_asset_a_code.is_some();
    let a_issuer_set = params.filter_asset_a_issuer.is_some();
    if a_code_set != a_issuer_set {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "filter[asset_a_code] and filter[asset_a_issuer] must be supplied together \
             (classic identity) or both omitted",
            serde_json::json!({
                "filter[asset_a_code]": params.filter_asset_a_code,
                "filter[asset_a_issuer]": params.filter_asset_a_issuer,
            }),
        );
    }
    let b_code_set = params.filter_asset_b_code.is_some();
    let b_issuer_set = params.filter_asset_b_issuer.is_some();
    if b_code_set != b_issuer_set {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "filter[asset_b_code] and filter[asset_b_issuer] must be supplied together \
             (classic identity) or both omitted",
            serde_json::json!({
                "filter[asset_b_code]": params.filter_asset_b_code,
                "filter[asset_b_issuer]": params.filter_asset_b_issuer,
            }),
        );
    }

    if let Some(min) = params.filter_min_tvl.as_deref()
        && !is_valid_decimal_string(min)
    {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "filter[min_tvl] must be a non-negative decimal string \
             (digits and at most one `.`); NaN, Infinity, exponent forms, \
             and signed values are rejected",
            serde_json::json!({ "filter": "min_tvl", "received": min }),
        );
    }

    let resolved = ResolvedPoolListParams {
        limit: i64::from(pagination.limit) + 1,
        cursor: pagination.cursor,
        asset_a_code: params.filter_asset_a_code,
        asset_a_issuer: params.filter_asset_a_issuer,
        asset_b_code: params.filter_asset_b_code,
        asset_b_issuer: params.filter_asset_b_issuer,
        min_tvl: params.filter_min_tvl,
    };

    let mut rows = match fetch_pool_list(&state.db, &resolved).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in list_pools: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(&mut rows, pagination.limit, |r| {
        cursor::encode(&PoolListCursor {
            created_at_ledger: r.created_at_ledger,
            pool_id_hex: r.pool_id_hex.clone(),
        })
    });
    let data: Vec<PoolItem> = rows.into_iter().map(map_pool_item).collect();

    Json(into_envelope(data, page)).into_response()
}

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — 64-char lowercase hex (BYTEA(32)) per ADR 0024."),
    ),
    responses(
        (status = 200, description = "Pool detail", body = PoolItem),
        (status = 400, description = "Invalid pool_id", body = ErrorEnvelope),
        (status = 404, description = "Pool not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_pool(State(state): State<AppState>, Path(pool_id): Path<String>) -> Response {
    if let Err(resp) = path::pool_id_hex(&pool_id, "pool_id") {
        return resp;
    }

    let row = match fetch_pool_by_id(&state.db, &pool_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!("DB error in get_pool({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    Json(map_pool_item(row)).into_response()
}

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/transactions",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — 64-char lowercase hex (BYTEA(32))."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated pool transactions",
         body = Paginated<PoolTransactionItem>),
        (status = 400, description = "Invalid pool_id, limit, or cursor", body = ErrorEnvelope),
        (status = 404, description = "Pool not found",  body = ErrorEnvelope),
        (status = 500, description = "Database error",  body = ErrorEnvelope),
    )
)]
pub async fn list_pool_transactions(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    pagination: Pagination<TsIdCursor>,
) -> Response {
    if let Err(resp) = path::pool_id_hex(&pool_id, "pool_id") {
        return resp;
    }

    match pool_exists(&state.db, &pool_id).await {
        Ok(true) => {}
        Ok(false) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!("DB error in pool_exists({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    let mut rows = match fetch_pool_transactions(
        &state.db,
        &pool_id,
        i64::from(pagination.limit) + 1,
        pagination.cursor.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in fetch_pool_transactions({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_ts_id_page(&mut rows, pagination.limit, |r| r.created_at, |r| r.id);
    let data: Vec<PoolTransactionItem> = rows
        .into_iter()
        .map(|r| PoolTransactionItem {
            hash: r.hash,
            ledger_sequence: r.ledger_sequence,
            source_account: r.source_account,
            fee_charged: r.fee_charged,
            successful: r.successful,
            operation_count: r.operation_count,
            has_soroban: r.has_soroban,
            operation_types: r.operation_types,
            created_at: r.created_at,
        })
        .collect();

    Json(into_envelope(data, page)).into_response()
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
         description = "Pool ID — 64-char lowercase hex (BYTEA(32))."),
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
    if let Err(resp) = path::pool_id_hex(&pool_id, "pool_id") {
        return resp;
    }

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

    match pool_exists(&state.db, &pool_id).await {
        Ok(true) => {}
        Ok(false) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!("DB error in pool_exists({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    }

    let data_points = match fetch_pool_chart(&state.db, &pool_id, &interval, from, to).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("DB error in fetch_pool_chart({pool_id}): {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    Json(ChartResponse {
        pool_id,
        interval,
        from,
        to,
        data_points,
    })
    .into_response()
}
