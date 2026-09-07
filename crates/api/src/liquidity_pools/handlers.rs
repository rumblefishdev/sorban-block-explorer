//! Handlers for the liquidity-pool endpoints (participants from task 0126;
//! list / detail / activity / chart from tasks 0052 and 0491).

#![allow(clippy::result_large_err)]

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};

use crate::common::cache_control;
use crate::common::cursor;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::common::path;
use crate::common::pool_asset_codes::normalize_asset_codes;
use crate::common::strkey::{pool_id_from_text, pool_id_hex_to_strkey};
use crate::openapi::schemas::{ErrorEnvelope, Paginated};
use crate::state::AppState;

use super::dto::{
    ChartParams, ChartResponse, ParticipantItem, PoolActivityCursor, PoolActivityItem,
    PoolActivityParams, PoolAssetLeg, PoolEvent, PoolItem, PoolLegAmount, PoolLegItem,
    PoolListCursor, PoolListParams, SharesCursor,
};
use super::queries::{self, PoolRow, ResolvedPoolListParams};
use crate::common::strkey::contract_hex_to_strkey;

#[utoipa::path(
    get,
    path = "/liquidity-pools/{pool_id}/participants",
    tag = "liquidity-pools",
    params(
        ("pool_id" = String, Path,
         description = "Pool ID — 56-char StrKey: `L...` (classic pool, SEP-23) or `C...` (soroban pool contract). Internal DB form is hex (ADR 0024); the strkey is the canonical wire form."),
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

    // Kind gate first (task 0374): the pool's kind decides WHICH population
    // its participants are — `lp_positions` rows for classic, share-token
    // holders in `balances` for soroban — so the existence check and the
    // page read can no longer overlap (the 0446 pairing applied to the
    // classic-only world; the gate is a cheap point read).
    let ch = state.ch();
    let (pool_kind, share_token_id) = match queries::fetch_pool_kind_share(&ch, &pool_id_hex).await
    {
        Ok(Some(ks)) => ks,
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_kind_share");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // Soroban pool with no share-token relation: the relation is either not
    // yet derived (indexing lag) or structurally absent (a concentrated pool
    // mints no share token — its positions are (owner, tick-range) entries
    // in its own storage, not yet indexed). An
    // empty 200 would read as "no participants" about a pool that HAS them,
    // so this refuses explicitly instead (same shape as the min_tvl refusal).
    if pool_kind == 1 && share_token_id == 0 {
        return errors::bad_request_with_details(
            errors::INVALID_FILTER,
            "participants are not available for this pool: no share token is \
             known (either not yet derived, or a concentrated pool whose \
             positions are not share-token balances)",
            serde_json::json!({ "pool_id": pool_id, "pool_kind": "soroban" }),
        );
    }

    let fetched = if pool_kind == 1 {
        queries::fetch_soroban_participants(
            &ch,
            share_token_id,
            pagination.cursor.as_ref(),
            fetch_limit,
            direction,
        )
        .await
    } else {
        queries::fetch_participants(
            &ch,
            &pool_id_hex,
            pagination.cursor.as_ref(),
            fetch_limit,
            direction,
        )
        .await
    };
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
                    // ALWAYS the database-side form (`cursor_shares`), never
                    // the display value — the keyset compares against the
                    // stored column, and the soroban display is re-scaled.
                    shares: last.cursor_shares.clone(),
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

/// The soroban half of a pool row, assembled by [`soroban_views`] and NOT
/// derivable from the row alone (legs, reserves and the protocol label all
/// resolve through other tables).
struct SorobanView {
    legs: Vec<PoolLegItem>,
    protocol: Option<String>,
}

/// USD TVL for every soroban row on a page, keyed by pool hex.
///
/// Two price queries for the WHOLE page, not per row — the same discipline the
/// classic list follows. The detail is just a page of one.
///
/// Degrades to an empty map on error, like the classic analytics: a missing
/// `prices.*` grant blanks a figure rather than failing the page.
async fn soroban_tvls(
    client: &clickhouse::Client,
    rows: &[PoolRow],
    views: &std::collections::HashMap<String, SorobanView>,
) -> std::collections::HashMap<String, String> {
    let soroban: Vec<&PoolRow> = rows.iter().filter(|r| r.pool_kind == 1).collect();
    if soroban.is_empty() {
        return std::collections::HashMap::new();
    }
    let all_legs: Vec<i64> = soroban.iter().flat_map(|r| r.legs.clone()).collect();
    let resolved = match queries::soroban_chart_legs(client, &all_legs).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error resolving soroban legs for TVL");
            return std::collections::HashMap::new();
        }
    };
    let prices = match queries::fetch_soroban_leg_prices(client, &resolved).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "DB error pricing soroban legs (TVL degraded to NULL)");
            return std::collections::HashMap::new();
        }
    };
    let by_id: std::collections::HashMap<i64, &Option<queries::SorobanChartLeg>> =
        resolved.iter().map(|(id, leg)| (*id, leg)).collect();

    soroban
        .iter()
        .filter_map(|r| {
            let view = views.get(&r.pool_id_hex)?;
            let legs: Vec<(i64, Option<queries::SorobanChartLeg>)> = r
                .legs
                .iter()
                .map(|id| (*id, by_id.get(id).and_then(|l| (*l).clone())))
                .collect();
            let reserves: Vec<String> = view
                .legs
                .iter()
                .map(|l| l.reserve.clone().unwrap_or_default())
                .collect();
            let tvl = queries::soroban_tvl(&legs, &reserves, &prices)?;
            Some((r.pool_id_hex.clone(), queries::usd_str(tvl)))
        })
        .collect()
}

/// 24h volume + fee revenue for ONE soroban pool, detail only — the same
/// scope the classic branch has, since both read per-pool sources a list page
/// cannot afford.
///
/// The classic path prices `gross_volume_a` from the snapshots table, which
/// soroban never writes; this prices the pool's own `trade` events, the source
/// its chart already aggregates. Degrades to NULL on error rather than failing
/// the page.
async fn soroban_flow_analytics(client: &clickhouse::Client, row: &mut PoolRow) {
    let resolved = match queries::soroban_chart_legs(client, &row.legs).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error resolving legs for soroban volume");
            return;
        }
    };
    let prices = match queries::fetch_soroban_leg_prices(client, &resolved).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "DB error pricing legs for soroban volume");
            return;
        }
    };
    let pool_strkey = crate::common::strkey::contract_hex_to_strkey(&row.pool_id_hex);
    match queries::fetch_soroban_volume_24h(client, &pool_strkey, &resolved, &prices).await {
        Ok(Some(vol)) => {
            row.volume = Some(queries::usd_str(vol));
            row.fee_revenue = Some(queries::usd_str(queries::fee_revenue_usd(vol, row.fee_bps)));
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "DB error in fetch_soroban_volume_24h");
        }
    }
}

fn map_pool_item(row: PoolRow, soroban: Option<SorobanView>) -> PoolItem {
    // A soroban pool's id bytes are a CONTRACT address payload — rendering
    // them as `L...` would produce a well-formed WRONG key, so each kind
    // encodes its own strkey flavour.
    let is_soroban = row.pool_kind == 1;
    let pool_id = if is_soroban {
        contract_hex_to_strkey(&row.pool_id_hex)
    } else {
        pool_id_hex_to_strkey(&row.pool_id_hex)
    };
    let (legs, protocol) = match soroban {
        Some(v) => (Some(v.legs), v.protocol),
        None => (None, None),
    };
    PoolItem {
        pool_id,
        pool_kind: if is_soroban { "soroban" } else { "classic" }.to_string(),
        protocol,
        pool_type: (!row.pool_type_raw.is_empty()).then(|| row.pool_type_raw.clone()),
        legs,
        // The pair columns on a soroban row are storage defaults, not legs —
        // surfacing them would render every soroban pool as native/native.
        asset_a: (!is_soroban).then_some(PoolAssetLeg {
            asset_type_name: row.asset_a_type_name,
            asset_type: row.asset_a_type,
            asset_code: row.asset_a_code,
            issuer: row.asset_a_issuer,
            contract_id: row.asset_a_contract_id,
            icon_url: row.asset_a_icon_url,
        }),
        asset_b: (!is_soroban).then_some(PoolAssetLeg {
            asset_type_name: row.asset_b_type_name,
            asset_type: row.asset_b_type,
            asset_code: row.asset_b_code,
            issuer: row.asset_b_issuer,
            contract_id: row.asset_b_contract_id,
            icon_url: row.asset_b_icon_url,
        }),
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

/// `AssetFamily` discriminant → wire label for a soroban-pool leg.
///
/// The mapping itself belongs to the domain enum and is NOT repeated here — a
/// second copy is exactly the drift task 0496 recorded. Only the fallback is
/// local, because it is not a family: a leg whose surrogate resolved to
/// nothing has no discriminant to name, and saying so explicitly beats
/// guessing a family for it.
fn family_label(family: i16) -> &'static str {
    domain::enums::AssetFamily::try_from(family)
        .map(domain::enums::AssetFamily::as_str)
        .unwrap_or("unresolved")
}

/// Assemble the soroban halves for the soroban rows of a page, keyed by
/// `pool_id_hex`. One batched round per concern (legs, issuers, reserves,
/// protocol labels) — never per row. Classic rows get no entry.
///
/// Resolution misses DEGRADE, they never fail the page: an unresolvable leg
/// renders as `family: "unresolved"` (house rule — explicit, not a plausible
/// empty asset), and a reserves/protocol lookup error only loses that
/// enrichment.
async fn soroban_views(
    client: &clickhouse::Client,
    rows: &[PoolRow],
) -> Result<std::collections::HashMap<String, SorobanView>, clickhouse::error::Error> {
    let soroban_rows: Vec<&PoolRow> = rows.iter().filter(|r| r.pool_kind == 1).collect();
    if soroban_rows.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let leg_ids: Vec<i64> = soroban_rows.iter().flat_map(|r| r.legs.clone()).collect();
    let pool_hexes: Vec<&str> = soroban_rows
        .iter()
        .map(|r| r.pool_id_hex.as_str())
        .collect();
    let deployment_ids: Vec<i64> = soroban_rows.iter().map(|r| r.deployment_id).collect();

    let (resolved, reserves, protocols) = tokio::join!(
        queries::resolve_leg_assets(client, &leg_ids),
        queries::fetch_latest_soroban_reserves(client, &pool_hexes),
        queries::resolve_protocol_labels(client, deployment_ids),
    );
    let resolved = resolved?;
    // Reserves and protocol labels are enrichment: log-and-degrade so a
    // side-table hiccup does not blank the whole pool list.
    let reserves = reserves.unwrap_or_else(|e| {
        tracing::error!("DB error in fetch_latest_soroban_reserves (degraded to null): {e}");
        std::collections::HashMap::new()
    });
    let protocols = protocols.unwrap_or_else(|e| {
        tracing::error!("DB error in resolve_protocol_labels (degraded to null): {e}");
        std::collections::HashMap::new()
    });

    // Classic-credit legs carry an issuer surrogate that must render as a
    // G-strkey; one batched accounts seek for the whole page.
    let issuer_ids: Vec<i64> = resolved
        .values()
        .filter(|l| l.issuer_id != 0)
        .map(|l| l.issuer_id)
        .collect();
    let issuers = crate::common::ch::resolve_accounts(client, issuer_ids).await?;

    Ok(soroban_rows
        .into_iter()
        .map(|row| {
            let pool_reserves = reserves.get(&row.pool_id_hex);
            let legs = row
                .legs
                .iter()
                .enumerate()
                .map(|(i, leg_id)| {
                    // Reserves ride the same emission order as the legs; the
                    // vector may carry a per-tick tail past the leg count
                    // (concentrated pools), which this slice-by-index
                    // deliberately never reads.
                    let reserve = pool_reserves.and_then(|r| r.get(i).cloned());
                    match resolved.get(leg_id) {
                        Some(l) => PoolLegItem {
                            family: family_label(l.family).to_string(),
                            asset_code: (!l.asset_code.is_empty()).then(|| l.asset_code.clone()),
                            issuer: issuers.get(&l.issuer_id).cloned(),
                            contract_id: l.contract_strkey.clone(),
                            symbol: l.symbol.clone(),
                            name: l.name.clone(),
                            decimals: l.decimals,
                            reserve,
                        },
                        None => PoolLegItem {
                            family: "unresolved".to_string(),
                            asset_code: None,
                            issuer: None,
                            contract_id: None,
                            symbol: None,
                            name: None,
                            decimals: None,
                            reserve,
                        },
                    }
                })
                .collect();
            (
                row.pool_id_hex.clone(),
                SorobanView {
                    legs,
                    protocol: protocols.get(&row.deployment_id).map(|s| s.to_string()),
                },
            )
        })
        .collect())
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

    // `filter[pool_kind]` → stored discriminant (task 0374). Validated here
    // so a bad value gets this API's envelope with the allowed list — same
    // shape as `filter[event]` / chart `interval`.
    let pool_kind = match params.filter_pool_kind.as_deref() {
        None => None,
        Some("classic") => Some(0u8),
        Some("soroban") => Some(1u8),
        Some(other) => {
            return errors::bad_request_with_details(
                errors::INVALID_FILTER,
                "filter[pool_kind] must be one of: classic, soroban",
                serde_json::json!({
                    "param": "filter[pool_kind]",
                    "received": other,
                    "allowed": ["classic", "soroban"],
                }),
            );
        }
    };

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
    let resolved = ResolvedPoolListParams {
        limit: pagination.fetch_limit(),
        cursor: pagination.cursor,
        asset_a_code: params.filter_asset_a_code,
        asset_a_issuer: params.filter_asset_a_issuer,
        asset_b_code: params.filter_asset_b_code,
        asset_b_issuer: params.filter_asset_b_issuer,
        asset_codes,
        pool_id_hex,
        pool_kind,
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
    // Soroban rows need their legs/reserves/protocol resolved (task 0374);
    // classic rows pass through untouched.
    let mut views = match soroban_views(&state.ch(), &rows).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "DB error in soroban_views (list)");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };
    let tvls = soroban_tvls(&state.ch(), &rows, &views).await;
    let data: Vec<PoolItem> = rows
        .into_iter()
        .map(|mut r| {
            let view = views.remove(&r.pool_id_hex);
            if r.pool_kind == 1 {
                r.tvl = tvls.get(&r.pool_id_hex).cloned();
            }
            map_pool_item(r, view)
        })
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
         description = "Pool ID — 56-char StrKey: `L...` (classic pool, SEP-23) or `C...` (soroban pool contract). Internal DB form is hex (ADR 0024); the strkey is the canonical wire form."),
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

    // Soroban pool: resolve legs/reserves/protocol instead of the classic
    // USD analytics — its legs have no classic prices identity, so
    // tvl/volume stay NULL by the existing "unpriceable" convention.
    if row.pool_kind == 1 {
        let rows = std::slice::from_ref(&row);
        let mut views = match soroban_views(&state.ch(), rows).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(pool_id = %pool_id, error = %e, "DB error in soroban_views (detail)");
                return errors::internal_error(errors::DB_ERROR, "database error");
            }
        };
        // The chart below this page prices these exact legs, so the headline
        // must not sit empty beside it. Degrades to NULL on error, like the
        // classic branch — a pool's on-chain data is valid without prices.
        row.tvl = soroban_tvls(&state.ch(), std::slice::from_ref(&row), &views)
            .await
            .remove(&row.pool_id_hex);
        soroban_flow_analytics(&state.ch(), &mut row).await;
        let view = views.remove(&row.pool_id_hex);
        let mut resp = Json(map_pool_item(row, view)).into_response();
        cache_control::attach(&mut resp, cache_control::SHORT);
        return resp;
    }

    // USD analytics (0199 compute-at-read): spot TVL + 24h volume/fee from
    // the in-cluster `prices.*` views. Deliberately DEGRADES to NULL fields
    // on error instead of failing the whole detail — the pool's on-chain
    // data is still valid without prices, and the FE already renders the
    // NULL ("stale") state. The error log is the operator signal (a missing
    // `prices.*` SELECT grant lands here, not in a 500).
    let ctx = queries::PoolPriceContext {
        leg_a: queries::price_leg(
            row.asset_a_type,
            row.asset_a_code.as_deref(),
            row.asset_a_issuer.as_deref(),
        ),
        leg_b: queries::price_leg(
            row.asset_b_type,
            row.asset_b_code.as_deref(),
            row.asset_b_issuer.as_deref(),
        ),
        fee_bps: row.fee_bps,
        pool_kind: row.pool_kind,
        // The detail analytics never read legs — only the chart branch does.
        legs: Vec::new(),
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

    let mut resp = Json(map_pool_item(row, None)).into_response();
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
         description = "Pool ID — 56-char StrKey: `L...` (classic pool, SEP-23) or `C...` (soroban pool contract)."),
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

    // The pool's feed source, which doubles as this path's existence check:
    // classic reads lp_operation_amounts by the pair surrogates; soroban
    // reads the pool's own flow events (task 0374 — the data was always in
    // soroban_events).
    let feed = match queries::fetch_pool_feed(&state.ch(), &pool_id_hex).await {
        Ok(Some(f)) => f,
        Ok(None) => return errors::not_found("liquidity pool not found"),
        Err(e) => {
            tracing::error!(pool_id = %pool_id, error = %e, "DB error in fetch_pool_feed");
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

    let fetched = match feed {
        queries::PoolFeed::Classic {
            asset_a_id,
            asset_b_id,
        } => {
            queries::fetch_pool_activity(
                &state.ch(),
                &pool_id_hex,
                (asset_a_id, asset_b_id),
                pagination.fetch_limit(),
                pagination.cursor.as_ref(),
                pagination.direction,
                event,
            )
            .await
        }
        queries::PoolFeed::Soroban { legs } => {
            queries::fetch_soroban_pool_activity(
                &state.ch(),
                &pool_id,
                &legs,
                pagination.fetch_limit(),
                pagination.cursor.as_ref(),
                pagination.direction,
                event,
            )
            .await
        }
    }
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
                    application_order: r.application_order.unwrap_or(0),
                    event_index: r.event_index,
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
            leg_amounts: r.leg_amounts.map(|la| {
                la.into_iter()
                    .map(|(leg_index, amount)| PoolLegAmount { leg_index, amount })
                    .collect()
            }),
            created_at: r.created_at,
        })
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
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
         description = "Pool ID — 56-char StrKey: `L...` (classic pool, SEP-23) or `C...` (soroban pool contract)."),
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
    // Soroban branch (task 0374): history from `pool_state_changes`, volume
    // from the pool's own trade events, prices via the SAME series — SAC
    // legs by their classic identity, bespoke tokens under
    // `asset_kind = 'contract'`. An unresolvable/unscaled leg makes the
    // affected values null, never partial. Both arms fall through to the
    // one response tail below.
    let fetched = if ctx.pool_kind == 1 {
        match queries::soroban_chart_legs(&state.ch(), &ctx.legs).await {
            Ok(chart_legs) => {
                queries::fetch_soroban_pool_chart(
                    &state.ch(),
                    &pool_id_hex,
                    &pool_id,
                    &chart_legs,
                    ctx.fee_bps,
                    &interval,
                    from,
                    to,
                )
                .await
            }
            Err(e) => Err(e),
        }
    } else {
        queries::fetch_pool_chart(&state.ch(), &pool_id_hex, &ctx, &interval, from, to).await
    }
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
#[path = "handlers_tests.rs"]
mod tests;
