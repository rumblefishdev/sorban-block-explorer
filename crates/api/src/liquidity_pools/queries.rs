//! ClickHouse queries for the liquidity-pool endpoints (task 0243).
//!
//! Returns the `PoolRow` / `PoolTxRow` /
//! `ParticipantRow` / `ChartDataPoint` shapes, so the handlers reuse
//! `map_pool_item` / cursor builders unchanged after the fetch.
//!
//! CH-specific translation choices (see task 0243 handoff note):
//! - **Decimal128(7)** columns are read via `toString(...)` in SQL → wire
//!   decimal strings, sidestepping the clickhouse-rs Decimal decode gotcha.
//! - **`pool_id`** is a `FixedString(32)`; the wire/hex form is the 64-char
//!   lowercase hex. SQL compares with `pool_id = unhex(?)` and reads back
//!   `lower(hex(pool_id))`.
//! - **`created_at_ledger`** does NOT exist on CH `liquidity_pools` (dropped,
//!   see schema header) — derived as `min(ledger_sequence)` over the pool's
//!   snapshots, falling back to `last_updated_ledger` for a pool that somehow
//!   has no snapshot yet.
//! - **snapshot `created_at`** does NOT exist on CH `liquidity_pool_snapshots`
//!   (only `ledger_sequence`) — the latest-snapshot timestamp is derived from
//!   the joined `ledgers.closed_at`.
//! - The freshness window (PG: `snapshots.created_at >= NOW() - 7d`) is NOT
//!   applied on the detail/list latest-snapshot pick yet — detail takes the
//!   single latest snapshot regardless of age (matches the "latest known
//!   state" intent); a staleness cutoff is a follow-up if parity needs it.

use chrono::{DateTime, Utc};
use clickhouse::Row;
use db_clickhouse::persist::ids;
use serde::Deserialize;
use std::collections::HashMap;

use crate::common::ch::{millis_to_utc, resolve_accounts};
use crate::common::cursor::{Direction, keyset_sql_desc};
use crate::common::pool_asset_codes::asset_codes_predicate;

use super::dto::{ChartDataPoint, PoolActivityCursor, PoolEvent, PoolListCursor, SharesCursor};

// ---------------------------------------------------------------------------
// Internal query-result rows + resolved params (not serialized; the handler
// maps these into the public response DTOs).
// ---------------------------------------------------------------------------

/// Canonical pool column projection shared between list and detail.
#[derive(Debug, Clone)]
pub struct PoolRow {
    pub pool_id_hex: String,
    pub asset_a_type: i16,
    pub asset_a_type_name: Option<String>,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer: Option<String>,
    /// C-strkey of the SAC mirror for the asset-A leg. `None` otherwise (task 0263).
    pub asset_a_contract_id: Option<String>,
    /// `icon_url` for the asset-A leg, from `asset_enrichment` (ADR 0050).
    /// NOT from `assets` — task 0310 dropped that dead column from prod.
    pub asset_a_icon_url: Option<String>,
    pub asset_b_type: i16,
    pub asset_b_type_name: Option<String>,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer: Option<String>,
    /// C-strkey of the SAC mirror for the asset-B leg. See `asset_a_contract_id`.
    pub asset_b_contract_id: Option<String>,
    /// `icon_url` for the asset-B leg. See `asset_a_icon_url`.
    pub asset_b_icon_url: Option<String>,
    pub fee_bps: i32,
    pub fee_percent: String,
    pub created_at_ledger: i64,
    /// Ledger value the list keyset orders + paginates on. CH keys on the
    /// native `last_updated_ledger` ("most recently active"), carried here.
    /// The wire `PoolListCursor.created_at_ledger` slot stays opaque (ADR
    /// 0008); only this field feeds the cursor builder. Unused by detail.
    pub cursor_ledger: i64,
    /// `COUNT(*) FROM lp_positions WHERE pool_id = lp.pool_id AND shares > 0`.
    /// Task 0246 — see DTO doc for surfacing rules.
    pub participant_count: i64,
    pub latest_snapshot_ledger: Option<i64>,
    pub reserve_a: Option<String>,
    pub reserve_b: Option<String>,
    pub total_shares: Option<String>,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
    pub latest_snapshot_at: Option<DateTime<Utc>>,
}

/// One current LP participant (a positive-shares position). Handler strips the
/// surrogate before building the API response.
#[derive(Debug)]
pub struct ParticipantRow {
    /// G-StrKey resolved via JOIN on `accounts`.
    pub account: String,
    /// `accounts.id` BIGINT — used only to encode the next cursor; not
    /// exposed in the response DTO.
    pub account_id_surrogate: i64,
    /// Numeric carried as text to preserve `NUMERIC(28,7)` precision.
    pub shares: String,
    /// `100 * shares / total_pool_shares`, NULL when the pool has no snapshot
    /// in the 7-day freshness window. Already a decimal string.
    pub share_percentage: Option<String>,
    pub first_deposit_ledger: i64,
    pub last_updated_ledger: i64,
}

/// Resolved, validated `GET /v1/liquidity-pools` list params.
pub struct ResolvedPoolListParams {
    pub limit: i64,
    pub cursor: Option<PoolListCursor>,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer: Option<String>,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer: Option<String>,
    /// Free-text asset filter (task 0246, widened in 0440) — trimmed and
    /// uppercased at the handler boundary, then split on `/` into at most two
    /// needles. Every needle must appear on *some* leg, which makes a pair
    /// query order-insensitive without anyone knowing Stellar's canonical leg
    /// ordering. Empty = no filter.
    pub asset_codes: Vec<String>,
    /// The same free-text box, when it held a pool identifier instead
    /// (`L…` SEP-23 StrKey, the one canonical form per task 0264, resolved to
    /// the stored hex — task 0470).
    /// Mutually exclusive with `asset_codes`: an identifier names exactly one
    /// pool, so there is nothing left for a code match to narrow.
    pub pool_id_hex: Option<String>,
}

/// One activity row after enrichment — the handler maps this straight into
/// `PoolActivityItem` (task 0491).
#[derive(Debug, Clone)]
pub struct PoolActivityRow {
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    /// Surrogate `transactions.id`. Not on the wire — it is the cursor's
    /// middle component, the same tie-break the sort key uses.
    pub transaction_id: i64,
    pub application_order: i16,
    pub event: Option<PoolEvent>,
    pub amount_a: Option<String>,
    pub amount_b: Option<String>,
    pub source_account: String,
    /// How many pools the whole operation crossed (`length(pool_ids)` off the
    /// same appearance seek that resolves the op source). `None` = unknowable
    /// (no appearance row), never guessed to `1`.
    pub pools_crossed: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// 7-day freshness window expressed in ledgers (~17280 ledgers/day at the
/// ~5 s mainnet cadence). The PG path uses `snapshots.created_at >= NOW() - 7d`;
/// CH `liquidity_pool_snapshots` carries no `created_at`, so the window is
/// approximated by a `ledger_sequence` floor relative to chain head. Exact
/// wall-clock parity is a documented tolerance (freshness is a stale/fresh
/// heuristic, not an exact cutoff).
const FRESHNESS_WINDOW_LEDGERS: i64 = 7 * 17_280;

/// `true` if `s` is a plain decimal string (digits, at most one `.`, optional
/// leading `-`). Cursor `shares` is decoded from an opaque payload and inlined
/// into the keyset SQL (to dodge the clickhouse-rs None-into-tuple bind defect,
/// same as accounts/contracts); validating it first keeps that inline safe.
fn is_decimal_str(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    !body.is_empty()
        && body.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && body.bytes().filter(|&b| b == b'.').count() <= 1
}

/// `true` if `s` is a 64-char lowercase-hex `pool_id` (the wire form, decoded
/// from the opaque list cursor). Guards the inlined keyset bound: a `pool_id`
/// from a tampered cursor that is not clean hex degrades to "no keyset" (first
/// page) rather than reaching the SQL string. Same rationale as
/// [`is_decimal_str`] for the participants cursor.
fn is_hex_pool_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `asset_type` SMALLINT → label, matching the PG `asset_type_name()` SQL
/// function (migration `20260422000000_enum_label_functions`) — the XDR
/// `AssetType`, which is what LP legs carry. NOT `asset_family_name`
/// (native/classic_credit/sac/soroban); the `PoolAssetLeg` doc-comment quotes
/// that sibling function and is misleading for pool legs. Box-confirmed: a
/// 9-char code (`WGUARDIAN`) is `asset_type = 2` = credit_alphanum12, not sac.
/// Out-of-range → `None` (PG `CASE` returns NULL with no `ELSE`).
fn asset_type_name(asset_type: i16) -> Option<String> {
    match asset_type {
        0 => Some("native".to_string()),
        1 => Some("credit_alphanum4".to_string()),
        2 => Some("credit_alphanum12".to_string()),
        3 => Some("pool_share".to_string()),
        _ => None,
    }
}

/// `fee_bps / 100` as a decimal string (e.g. 30 → "0.3", 25 → "0.25",
/// 100 → "1"). Computed in Rust to avoid CH integer-division / decimal-scale
/// quirks; trailing zeros are trimmed.
///
/// NOTE: PG emits `(fee_bps::numeric / 100)::text`; exact trailing-zero
/// parity is a documented box-smoke check (cosmetic field, FE re-renders).
fn fee_percent_str(fee_bps: i32) -> String {
    let whole = fee_bps / 100;
    let frac = (fee_bps % 100).abs();
    if frac == 0 {
        whole.to_string()
    } else if frac % 10 == 0 {
        format!("{whole}.{}", frac / 10)
    } else {
        format!("{whole}.{frac:02}")
    }
}

// ---------------------------------------------------------------------------
// USD analytics (task 0199, ADR 0053 compute-at-read).
//
// `tvl` / `volume` / `fee_revenue` are computed at read time from on-chain
// inputs (`reserve_a/b`, `gross_volume_a`, `fee_bps` — all indexer-written)
// joined against the prices service's `prices.*` views in the same CH
// cluster. Nothing is materialized back into `liquidity_pool_snapshots`
// (the RMT has no version column; a write-back would race live inserts).
//
// JOIN interop contract (prices views.sql header, pinned 2026-06-16):
// key = (asset_kind, asset_code, issuer_address) with
// asset_kind ∈ ('native','credit','contract'); native XLM is
// ('native','XLM',''); bucket is a grain-floored DateTime. Grains provided:
// 1h + 1d only — the 1w chart interval joins the DAILY series.
//
// Two deliberate traps documented in the task
// (notes/R-prices-freeze-incident-and-current-price-usd-v13.md):
// - never join raw `prices.assets` (153 empty-code rows silently price
//   native legs as an arbitrary asset);
// - never decode a `prices.*` view positionally / via `SELECT *` — the
//   views grow additively (current_price_usd went 6 → 13 columns).
//
// LEFT JOIN misses surface as DEFAULT values, not NULL (`join_use_nulls`
// is rejected for the readonly API user — CH gotcha list), so every read
// wraps the price in `nullIf(price, 0)`.
//
// The views do NOT guarantee `close_usd > 0`: a bucket whose only candles
// carry zero volume can publish `Decimal128::MIN` (≈ -1.7e24) instead of
// omitting the row (prices-side 0171, confirmed by the owner 2026-08-11).
// A negative close would print a -1e24-scale TVL and, through the chart's
// ASOF carry-forward, smear it over every later bucket — so every
// `close_usd` read here filters `close_usd > 0` itself and treats
// non-positive rows as absent.
//
// USD arithmetic is Float64, rounded to cents. The analytics carry a 1%
// verification tolerance by design (task AC); Float64 keeps the SQL free
// of Decimal128×Decimal128 scale-overflow (7+14 fractional digits).
// **Every money value is formatted by [`usd_str`] on the Rust side** — SQL
// returns raw Float64. CH's `toString(round(x, 2))` emits "25" / "0" /
// "1.5" (variable decimals), which would put the chart and the detail
// endpoint on two different wire shapes for the same field.
//
// **Bounded price carry-forward.** A price bucket exists only once the
// asset trades in it, so the in-progress bucket is routinely missing for an
// illiquid leg — an exact bucket-equality join then NULLs the newest chart
// point, the one users read as "current". Reads therefore take the most
// recent close at or before the wanted bucket (CH `ASOF LEFT JOIN`), but
// only within [`MAX_PRICE_CARRY_SECONDS`]. Unbounded carry-forward would be
// worse than a hole: it would paint the 2026-07-21..08-03 provider freeze
// with a 12-day-old price and present it as live (box-checked — the ASOF
// match for 07-28 is a 07-21 candle).
// ---------------------------------------------------------------------------

/// How stale a price candle may be before it stops standing in for a
/// missing one, in seconds (48 h).
///
/// Covers the routine gap — the current bucket has no candle yet because
/// the asset has not traded in it — without masking a real outage. Shared
/// by the chart (carry-forward bound) and the detail endpoint (lookback
/// window) so both surfaces answer "what is this pool worth" from the same
/// staleness rule.
const MAX_PRICE_CARRY_SECONDS: i64 = 48 * 3600;

/// Natural price identity of one pool leg in the exact column forms the
/// `prices.*` views expose. A leg that cannot be priced maps to empty
/// strings, which match no prices row → NULL analytics, never a wrong price.
///
/// `Hash` so it can key the [`fetch_last_closes`] result directly — the
/// per-row lookup on the list path then costs no allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PriceLeg {
    pub kind: &'static str,
    pub code: String,
    pub issuer: String,
}

/// Map an LP leg (XDR `AssetType` + code + issuer G-strkey) to its prices
/// identity. LP legs are classic-only (`LiquidityPoolEntry`), so only
/// native (0) and credit_alphanum4/12 (1|2) occur; anything else — or a
/// credit leg missing its code/issuer — is unpriceable by construction.
pub fn price_leg(asset_type: i16, code: Option<&str>, issuer: Option<&str>) -> PriceLeg {
    match asset_type {
        0 => PriceLeg {
            kind: "native",
            code: "XLM".to_string(),
            issuer: String::new(),
        },
        1 | 2 => match (code, issuer) {
            (Some(c), Some(i)) if !c.is_empty() && !i.is_empty() => PriceLeg {
                kind: "credit",
                code: c.to_string(),
                issuer: i.to_string(),
            },
            _ => PriceLeg {
                kind: "",
                code: String::new(),
                issuer: String::new(),
            },
        },
        _ => PriceLeg {
            kind: "",
            code: String::new(),
            issuer: String::new(),
        },
    }
}

/// Pool inputs the chart's USD computation needs besides the snapshots:
/// both leg identities + the pool fee. Fetched once per chart request
/// (doubles as the 404 existence gate).
#[derive(Debug, Clone)]
pub struct PoolPriceContext {
    pub leg_a: PriceLeg,
    pub leg_b: PriceLeg,
    pub fee_bps: i32,
}

/// SELECT column order MUST match this struct (clickhouse positional decode).
#[derive(Debug, Row, Deserialize)]
struct PriceContextChRow {
    asset_a_type: i16,
    asset_a_code: String,
    asset_a_issuer: String,
    asset_b_type: i16,
    asset_b_code: String,
    asset_b_issuer: String,
    fee_bps: i32,
}

/// Resolve the pool's leg identities + `fee_bps`. `None` = pool unknown
/// (the chart handler's 404 gate — replaces `pool_exists` there).
///
/// Issuer resolution reuses the detail query's restricted-`iss` idiom:
/// never `accounts FINAL` joins (14M-row hash build, box-confirmed
/// Code 241) — restrict to the pool's ≤2 issuer ids, `GROUP BY id` +
/// `any()` (account_id is stable across RMT versions).
pub async fn fetch_pool_price_context(
    client: &clickhouse::Client,
    pool_id_hex: &str,
) -> Result<Option<PoolPriceContext>, clickhouse::error::Error> {
    let row = client
        .query(
            "WITH legs AS ( \
                 SELECT asset_a_type, asset_a_code, asset_a_issuer_id, \
                        asset_b_type, asset_b_code, asset_b_issuer_id, fee_bps \
                 FROM liquidity_pools FINAL WHERE pool_id = unhex(?) \
             ), \
             iss AS ( \
                 SELECT id, any(account_id) AS account_id FROM accounts \
                 WHERE id IN (SELECT asset_a_issuer_id FROM legs WHERE asset_a_issuer_id != 0 \
                              UNION ALL SELECT asset_b_issuer_id FROM legs WHERE asset_b_issuer_id != 0) \
                 GROUP BY id \
             ) \
             SELECT \
                legs.asset_a_type            AS asset_a_type, \
                legs.asset_a_code            AS asset_a_code, \
                iss_a.account_id             AS asset_a_issuer, \
                legs.asset_b_type            AS asset_b_type, \
                legs.asset_b_code            AS asset_b_code, \
                iss_b.account_id             AS asset_b_issuer, \
                legs.fee_bps                 AS fee_bps \
             FROM legs \
             LEFT JOIN iss iss_a ON iss_a.id = legs.asset_a_issuer_id \
             LEFT JOIN iss iss_b ON iss_b.id = legs.asset_b_issuer_id \
             LIMIT 1",
        )
        .bind(pool_id_hex)
        .fetch_optional::<PriceContextChRow>()
        .await?;

    Ok(row.map(|r| PoolPriceContext {
        leg_a: price_leg(
            r.asset_a_type,
            Some(&r.asset_a_code),
            Some(&r.asset_a_issuer),
        ),
        leg_b: price_leg(
            r.asset_b_type,
            Some(&r.asset_b_code),
            Some(&r.asset_b_issuer),
        ),
        fee_bps: r.fee_bps,
    }))
}

/// Detail-endpoint USD analytics (task 0199 semantics, defined here because
/// the snapshot columns were never populated before this task):
/// - `tvl` — latest reserves × each leg's last hourly close
///   (`price_usd_series_1h`, [`MAX_PRICE_CARRY_SECONDS`] lookback); NULL
///   unless BOTH legs price (a one-leg TVL would silently halve the pool —
///   no-misleading-fallbacks rule).
/// - `volume` — last-24h `gross_volume_a` × the same leg-A close. One
///   price for the whole day, not per-trade (upgrade path: per-ledger join
///   as in the chart, if product needs it).
/// - `fee_revenue` — [`fee_revenue_usd`].
///
/// Why the 1h series and NOT `prices.current_price_usd`: box-measured
/// 2026-08-04, the spot view is live (3,316 assets, updater ticking) but
/// `price_usd = 0` — the "unavailable" sentinel — for native XLM itself,
/// so every XLM-leg pool (the majority) would read NULL TVL. The last 1h
/// close costs the same (112 ms / 1.6M read rows vs 92 ms / 1.2M on the
/// hottest pool) and actually returns data. Revisit spot when the
/// prices-side updater (their 0039) prices native.
///
/// This endpoint and the chart apply the SAME staleness rule
/// ([`MAX_PRICE_CARRY_SECONDS`]) but not the same grain: detail always
/// reads the hourly series, while the chart reads the grain its interval
/// asks for. So the two agree on whether a pool is priceable, and may
/// differ by up to one chart bucket on the value — a 1d bucket closes on
/// its own daily candle, not on the latest hour.
#[derive(Debug, Default)]
pub struct PoolUsdAnalytics {
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
}

/// SELECT column order MUST match this struct (clickhouse positional decode).
#[derive(Debug, Row, Deserialize)]
struct Vol24ChRow {
    vol24_a_units: Option<String>,
}

/// Last-24h gross trade volume for one pool, in asset-A units.
///
/// Deduped with `LIMIT 1 BY ledger_sequence` — same idiom as the chart —
/// because RMT duplicate versions of one `(pool, ledger)` row would double
/// the sum.
///
/// **Both ledger bounds are required, and the upper one is not redundant.**
/// `min()`/`max()` over an empty set return the type DEFAULT (`0`), not
/// NULL — box-verified. With only the `>=` floor, a 24h window containing
/// no ledgers degrades to `ledger_sequence >= 0`, i.e. the pool's ENTIRE
/// history, and the endpoint reports lifetime volume as "24h volume". That
/// is reachable: ingestion has stalled for >16h before (galexie
/// protocol-upgrade stall, 2026-07-08) while the independent prices service
/// kept serving, so the spot price would still resolve and the inflated
/// number would render as real. Pairing the bounds makes the empty window
/// self-cancelling (`>= 0 AND <= 0` matches nothing), which is exactly why
/// the chart's equivalent floor was safe.
async fn fetch_pool_volume_24h(
    client: &clickhouse::Client,
    pool_id_hex: &str,
) -> Result<Option<String>, clickhouse::error::Error> {
    let row = client
        .query(
            "SELECT toString(sum(gross_volume_a)) AS vol24_a_units FROM ( \
                 SELECT ledger_sequence, gross_volume_a \
                 FROM liquidity_pool_snapshots \
                 WHERE pool_id = unhex(?) \
                   AND ledger_sequence >= ( \
                       SELECT min(sequence) FROM ledgers \
                       WHERE closed_at >= now() - INTERVAL 24 HOUR) \
                   AND ledger_sequence <= ( \
                       SELECT max(sequence) FROM ledgers \
                       WHERE closed_at >= now() - INTERVAL 24 HOUR) \
                 ORDER BY ledger_sequence DESC \
                 LIMIT 1 BY ledger_sequence \
             )",
        )
        .bind(pool_id_hex)
        .fetch_one::<Vol24ChRow>()
        .await?;
    Ok(row.vol24_a_units)
}

/// Fetch last hourly closes + 24h gross volume, compute the detail USD
/// analytics in Rust (Float64 tolerance documented on the module block
/// above).
///
/// Prices come from the SAME [`fetch_last_closes`] primitive the list uses,
/// so the two surfaces cannot answer "is this pool priceable" differently —
/// and leg B stops re-scanning the window leg A already scanned. The two
/// queries are independent, so they overlap rather than run serially.
pub async fn fetch_pool_usd_analytics(
    client: &clickhouse::Client,
    pool_id_hex: &str,
    ctx: &PoolPriceContext,
    reserve_a: Option<&str>,
    reserve_b: Option<&str>,
) -> Result<PoolUsdAnalytics, clickhouse::error::Error> {
    let legs = priceable_legs(ctx);
    let (closes, vol24_raw) = tokio::join!(
        fetch_last_closes(client, &legs),
        fetch_pool_volume_24h(client, pool_id_hex),
    );
    let closes = closes?;
    let spot_a = closes.get(&ctx.leg_a).copied();
    let spot_b = closes.get(&ctx.leg_b).copied();
    // SQL NULL (no snapshot rows in the window, or no swaps among them) is a
    // genuine zero-volume day. A row that IS present but unparseable is NOT —
    // it is an unknown, and must not be reported as "$0.00 traded".
    let vol24_units = match vol24_raw?.as_deref() {
        None => Some(0.0),
        Some(raw) => parse_f64(raw),
    };

    let tvl = match (
        reserve_a.and_then(parse_f64),
        reserve_b.and_then(parse_f64),
        spot_a,
        spot_b,
    ) {
        (Some(ra), Some(rb), Some(pa), Some(pb)) => Some(ra * pa + rb * pb),
        _ => None,
    };
    let volume = match (spot_a, vol24_units) {
        (Some(pa), Some(units)) => Some(units * pa),
        _ => None,
    };
    let fee_revenue = volume.map(|v| fee_revenue_usd(v, ctx.fee_bps));

    Ok(PoolUsdAnalytics {
        tvl: tvl.map(usd_str),
        volume: volume.map(usd_str),
        fee_revenue: fee_revenue.map(usd_str),
    })
}

/// SELECT column order MUST match this struct (clickhouse positional decode).
#[derive(Debug, Row, Deserialize)]
struct LastCloseChRow {
    asset_kind: String,
    asset_code: String,
    issuer_address: String,
    close_usd: Option<String>,
}

/// Batched last-hourly-close lookup for a page of pools (Phase A2, list-side
/// TVL — issue #367's literal ask). ONE query per page, never per row: the
/// prices views cannot prune by identity anyway (computed columns), so the
/// cost is one bounded [`MAX_PRICE_CARRY_SECONDS`] window scan regardless of
/// how many identities the page carries; the OR-chain only trims the result
/// set. Unpriceable legs (empty `kind`) are filtered out by the caller.
///
/// **The in-progress hour is excluded on purpose.** The prices service
/// bakes `close_usd` in a pass that trails candle ingestion, so a bucket
/// still being formed is only partly enriched and its volume-weighted
/// close is taken over whichever rows happen to be done — on 2026-08-05
/// that made a 0.764-unit dust print the entire price of yXLM's 13:00
/// hour (1.3085 against a true ~0.170) and quadrupled the pool's TVL on
/// the page. The prices owner confirmed the mechanism, that only the
/// forming bucket is affected, and that it repairs once the bucket
/// closes; a coverage gate is coming, and this guard should be revisited
/// then. Cost of the guard is up to one hour of freshness against a
/// [`MAX_PRICE_CARRY_SECONDS`] budget — nothing.
///
/// Returns `(kind, code, issuer) → close_usd`; identities with no priced
/// candle in the window are simply absent.
async fn fetch_last_closes(
    client: &clickhouse::Client,
    legs: &[&PriceLeg],
) -> Result<std::collections::HashMap<PriceLeg, f64>, clickhouse::error::Error> {
    if legs.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let identity_or = std::iter::repeat_n(
        "(asset_kind = ? AND asset_code = ? AND issuer_address = ?)",
        legs.len(),
    )
    .collect::<Vec<_>>()
    .join(" OR ");
    let sql = format!(
        "SELECT asset_kind, asset_code, issuer_address, \
                toString(nullIf(argMaxIf(close_usd, bucket, close_usd > 0), 0)) AS close_usd \
         FROM prices.price_usd_series_1h \
         WHERE ({identity_or}) \
           AND bucket >= now() - INTERVAL {carry} SECOND \
           AND bucket <  toStartOfHour(now()) \
         GROUP BY asset_kind, asset_code, issuer_address",
        carry = MAX_PRICE_CARRY_SECONDS,
    );
    let mut query = client.query(&sql);
    for leg in legs {
        query = query
            .bind(leg.kind)
            .bind(leg.code.as_str())
            .bind(leg.issuer.as_str());
    }
    let rows = query.fetch_all::<LastCloseChRow>().await?;
    // Key by the CALLER's `PriceLeg`, not by the returned strings: the leg
    // owns the `&'static str` kind the caller will look up with, so callers
    // get an allocation-free `closes.get(leg)`. `legs` is at most two per
    // pool (≤ 2 × page), so the linear match back is trivial.
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let close = r.close_usd.as_deref().and_then(parse_f64)?;
            let leg = legs.iter().find(|l| {
                l.kind == r.asset_kind && l.code == r.asset_code && l.issuer == r.issuer_address
            })?;
            Some(((*leg).clone(), close))
        })
        .collect())
}

/// The two legs of a pool as a `fetch_last_closes` input, with unpriceable
/// legs (empty `kind`) dropped — they match no prices row by construction,
/// so asking for them is pure waste.
fn priceable_legs(ctx: &PoolPriceContext) -> Vec<&PriceLeg> {
    [&ctx.leg_a, &ctx.leg_b]
        .into_iter()
        .filter(|l| !l.kind.is_empty())
        .collect()
}

/// Strict decimal-string → f64 (the wire strings come from CH `toString`
/// over Decimal columns; anything non-parseable degrades to None, never 500).
fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// USD amount → wire string. The single formatter for every money field on
/// both LP surfaces — see the module note on why this is not done in SQL.
///
/// Cents for anything a cent or larger, and **significant digits below
/// that**, because a flat `{:.2}` reports a real value as `"0.00"` — a
/// number the client cannot tell from a genuine zero and cannot recover.
/// It is not a corner case: `fee_revenue` is 0.30% of the traded volume,
/// so any pool trading less than a few dollars a bucket serialises its
/// entire fee series as zeros (observed on prod — a pool with real volume
/// rendered every chart bucket and every axis tick as `$0`).
fn usd_str(v: f64) -> String {
    let abs = v.abs();
    if abs > 0.0 && abs < 0.01 {
        // Two significant digits: 0.003 → "0.0030", 0.00009 → "0.000090".
        // Capped so a denormal cannot produce an absurdly long string.
        let places = ((-abs.log10()).ceil() as usize + 1).min(12);
        format!("{v:.places$}")
    } else {
        format!("{v:.2}")
    }
}

/// `volume × fee_bps / 10000` — the pool's cut of the traded volume.
/// `fee_bps` is basis points (30 = 0.30%), so the divisor is 10 000, not
/// 100. Shared by chart and detail so the two cannot drift.
fn fee_revenue_usd(volume_usd: f64, fee_bps: i32) -> f64 {
    volume_usd * f64::from(fee_bps) / 10_000.0
}

/// SELECT column order MUST match this struct (clickhouse positional decode).
#[derive(Debug, Row, Deserialize)]
struct PoolDetailChRow {
    pool_id_hex: String,
    asset_a_type: i16,
    asset_a_code: Option<String>,
    asset_a_issuer: Option<String>,
    asset_a_contract_id: Option<String>,
    asset_a_icon_url: Option<String>,
    asset_b_type: i16,
    asset_b_code: Option<String>,
    asset_b_issuer: Option<String>,
    asset_b_contract_id: Option<String>,
    asset_b_icon_url: Option<String>,
    fee_bps: i32,
    created_at_ledger: i64,
    participant_count: i64,
    latest_snapshot_ledger: Option<i64>,
    reserve_a: Option<String>,
    reserve_b: Option<String>,
    total_shares: Option<String>,
    latest_snapshot_at_ms: Option<i64>,
}

/// `GET /v1/liquidity-pools/:id` — single-pool detail. Mirrors the PG
/// `fetch_pool_by_id` projection. `tvl`/`volume`/`fee_revenue` are NOT read
/// here — the snapshot columns were never populated (pre-0199 design); the
/// handler fills them from [`fetch_pool_usd_analytics`] (compute-at-read).
pub async fn fetch_pool_by_id(
    client: &clickhouse::Client,
    pool_id_hex: &str,
) -> Result<Option<PoolRow>, clickhouse::error::Error> {
    // `unhex(?)` appears 5×: the `legs` CTE, the created_at-ledger subquery, the
    // participant-count subquery, the latest-snapshot subquery, and the outer
    // WHERE. All scoped to the literal pool id (NOT correlated to `lp`) since
    // detail is single-pool and CH dislikes correlated subqueries. Each `?`
    // consumes one positional bind; all are the same value, so order is moot.
    //
    // `legs` resolves the pool's two `(code, issuer_id)` pairs once; `iss` and
    // `sac` both fan out from it.
    //
    // **Issuer resolution is a restricted `iss` CTE, NOT `accounts FINAL`
    // joins.** `accounts` is `ORDER BY (account_id)`, so the surrogate `id` is a
    // non-PK reverse lookup; a plain `LEFT JOIN accounts FINAL` builds the whole
    // 14M-row table into the hash — and detail does it for BOTH legs, blowing
    // the 3.73 GiB per-query cap (box-confirmed `Code 241`). Restricting to the
    // pool's ≤2 issuer ids + `GROUP BY id` (no FINAL — account_id is stable
    // across RMT versions, `any()` is safe) scans the id column but builds a
    // ≤2-row hash. Same shape as `fetch_pool_list`'s `iss` CTE.
    //
    // SAC mirror + icon_url (task 0263 + 0274 gap #5 → ADR 0051): the `sac` CTE
    // resolves `(asset_code, issuer_id)` → `(contract_id, icon_url)` once per leg,
    // deduped by GROUP BY so a leg cannot fan the result out (the inline-join
    // form did, masked only by the outer LIMIT 1). Post-ADR 0051 the SAC handle
    // is a FACET in the `asset_sac` side table (not a column on `assets`, and not
    // a separate `asset_type = 2`) — so the deployed SAC's `C…` StrKey resolves by
    // two hops: leg `(code, issuer)` → `asset_sac.sac_contract_id` (surrogate) →
    // `soroban_contracts.contract_id` (un-deployed SACs have no contract row →
    // NULL, as before). The classic carrier is `asset_type IN (0, 1)`.
    //
    // **Native legs are IN the join** (task 0470). They used to be excluded by an
    // `asset_code != ''` guard on every arm, on the assumption that an empty code
    // could match nothing — but native XLM has a deployed SAC like any other
    // classic asset, so the guard was hiding a real answer: the leg reported a
    // NULL `contract_id` for an asset that has one. `('', 0)` is a safe join key
    // here, measured: it is exactly one asset across `asset_type IN (0, 1)`,
    // since a classic credit code is 1–12 characters by protocol. The icon stays
    // NULL, but because `asset_enrichment` holds no native row at all — not
    // because a guard forbids the lookup.
    //
    // **Latest snapshot subquery — NO `FINAL`** (0356 / PR #318). The indexer now
    // writes exactly one deterministic row per `(pool_id, ledger_sequence)`, so
    // `FINAL` is redundant for dedup; dropping it turns the read into a bounded
    // reverse-PK seek (`ORDER BY ledger_sequence DESC LIMIT 1`) instead of a
    // whole-table merge. It stays a whole-row `LIMIT 1` (not per-column
    // `argMax`), so `reserve_a`/`reserve_b` can never tear across a stale
    // before/after pair in the pre-cleanup window. `created_at_ledger` already
    // reads without `FINAL` (`min(ledger_sequence)` is dup-invariant).
    //
    // **`ledgers` is SEEKED, never joined whole.** `LEFT JOIN ledgers l ON
    // l.sequence = s.ledger_sequence` hash-built the entire 26M-row table to
    // resolve ONE `closed_at` — 27.2M read_rows / 1.82 GiB / ~724 ms of CH per
    // request under load (96% of this endpoint's cost, measured 2026-07-17 at
    // 50M/mo). Restricting the right side to the single sequence the snapshot
    // subquery points at makes it a PK point read.
    //
    // The `s` join is an EQUI-join on `pool_id` (not `ON 1 = 1`): a constant ON
    // condition is only supported by `join_algorithm = 'hash'`, so the old form
    // 500'd (Code 48) the moment the server profile carried anything else.
    let row = client
        .query(
            "WITH legs AS ( \
                 SELECT asset_a_code, asset_a_issuer_id, asset_b_code, asset_b_issuer_id \
                 FROM liquidity_pools FINAL WHERE pool_id = unhex(?) \
             ), \
             iss AS ( \
                 SELECT id, any(account_id) AS account_id FROM accounts \
                 WHERE id IN (SELECT asset_a_issuer_id FROM legs WHERE asset_a_issuer_id != 0 \
                              UNION ALL SELECT asset_b_issuer_id FROM legs WHERE asset_b_issuer_id != 0) \
                 GROUP BY id \
             ), \
             sac AS ( \
                 SELECT a.asset_code AS asset_code, a.issuer_id AS issuer_id, \
                        max(sc.contract_id)      AS contract_id, \
                        nullIf(max(ae.icon_url), '') AS icon_url \
                 FROM assets a \
                 LEFT JOIN ( \
                     SELECT asset_type, asset_code, issuer_id, contract_id, \
                            max(sac_contract_id) AS sac_contract_id \
                     FROM asset_sac GROUP BY asset_type, asset_code, issuer_id, contract_id \
                 ) asac ON asac.asset_type = a.asset_type AND asac.asset_code = a.asset_code \
                       AND asac.issuer_id = a.issuer_id AND asac.contract_id = a.contract_id \
                 LEFT JOIN soroban_contracts sc ON sc.id = asac.sac_contract_id AND asac.sac_contract_id != 0 \
                 LEFT JOIN ( \
                     SELECT asset_type, asset_code, issuer_id, contract_id, \
                            argMax(icon_url, version) AS icon_url \
                     FROM asset_enrichment \
                     WHERE asset_type IN (0, 1) AND asset_code IN ( \
                         SELECT asset_a_code FROM legs \
                         UNION ALL SELECT asset_b_code FROM legs) \
                     GROUP BY asset_type, asset_code, issuer_id, contract_id \
                 ) ae ON ae.asset_type = a.asset_type AND ae.asset_code = a.asset_code \
                     AND ae.issuer_id = a.issuer_id AND ae.contract_id = a.contract_id \
                 WHERE a.asset_type IN (0, 1) \
                   AND (a.asset_code, a.issuer_id) IN ( \
                       SELECT asset_a_code, asset_a_issuer_id FROM legs \
                       UNION ALL \
                       SELECT asset_b_code, asset_b_issuer_id FROM legs) \
                 GROUP BY a.asset_code, a.issuer_id \
             ) \
             SELECT \
                lower(hex(lp.pool_id))               AS pool_id_hex, \
                lp.asset_a_type                      AS asset_a_type, \
                nullIf(lp.asset_a_code, '')          AS asset_a_code, \
                nullIf(iss_a.account_id, '')         AS asset_a_issuer, \
                nullIf(sac_a.contract_id, '')        AS asset_a_contract_id, \
                sac_a.icon_url                       AS asset_a_icon_url, \
                lp.asset_b_type                      AS asset_b_type, \
                nullIf(lp.asset_b_code, '')          AS asset_b_code, \
                nullIf(iss_b.account_id, '')         AS asset_b_issuer, \
                nullIf(sac_b.contract_id, '')        AS asset_b_contract_id, \
                sac_b.icon_url                       AS asset_b_icon_url, \
                lp.fee_bps                           AS fee_bps, \
                ifNull( \
                    (SELECT min(ledger_sequence) FROM liquidity_pool_snapshots \
                      WHERE pool_id = unhex(?)), \
                    lp.last_updated_ledger)          AS created_at_ledger, \
                toInt64(ifNull( \
                    (SELECT count() FROM lp_positions FINAL \
                      WHERE pool_id = unhex(?) AND shares > 0), 0)) AS participant_count, \
                s.ledger_sequence                    AS latest_snapshot_ledger, \
                toString(s.reserve_a)                AS reserve_a, \
                toString(s.reserve_b)                AS reserve_b, \
                toString(s.total_shares)             AS total_shares, \
                nullIf(toUnixTimestamp64Milli(l.closed_at), 0) AS latest_snapshot_at_ms \
             FROM liquidity_pools lp FINAL \
             LEFT JOIN iss iss_a ON iss_a.id = lp.asset_a_issuer_id \
             LEFT JOIN iss iss_b ON iss_b.id = lp.asset_b_issuer_id \
             LEFT JOIN sac sac_a ON sac_a.asset_code = lp.asset_a_code \
                                AND sac_a.issuer_id = lp.asset_a_issuer_id \
             LEFT JOIN sac sac_b ON sac_b.asset_code = lp.asset_b_code \
                                AND sac_b.issuer_id = lp.asset_b_issuer_id \
             LEFT JOIN ( \
                 SELECT pool_id, \
                        toNullable(ledger_sequence) AS ledger_sequence, \
                        toNullable(reserve_a)       AS reserve_a, \
                        toNullable(reserve_b)       AS reserve_b, \
                        toNullable(total_shares)    AS total_shares \
                 FROM liquidity_pool_snapshots \
                 WHERE pool_id = unhex(?) \
                 ORDER BY ledger_sequence DESC \
                 LIMIT 1 \
             ) s ON s.pool_id = lp.pool_id \
             LEFT JOIN ( \
                 SELECT sequence, closed_at FROM ledgers \
                 WHERE sequence = (SELECT max(ledger_sequence) FROM liquidity_pool_snapshots \
                                    WHERE pool_id = unhex(?)) \
             ) l ON l.sequence = s.ledger_sequence \
             WHERE lp.pool_id = unhex(?) \
             LIMIT 1",
        )
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .fetch_optional::<PoolDetailChRow>()
        .await?;

    Ok(row.map(|r| PoolRow {
        pool_id_hex: r.pool_id_hex,
        asset_a_type: r.asset_a_type,
        asset_a_type_name: asset_type_name(r.asset_a_type),
        asset_a_code: r.asset_a_code,
        asset_a_issuer: r.asset_a_issuer,
        asset_a_contract_id: r.asset_a_contract_id,
        asset_a_icon_url: r.asset_a_icon_url,
        asset_b_type: r.asset_b_type,
        asset_b_type_name: asset_type_name(r.asset_b_type),
        asset_b_code: r.asset_b_code,
        asset_b_issuer: r.asset_b_issuer,
        asset_b_contract_id: r.asset_b_contract_id,
        asset_b_icon_url: r.asset_b_icon_url,
        fee_bps: r.fee_bps,
        fee_percent: fee_percent_str(r.fee_bps),
        created_at_ledger: r.created_at_ledger,
        // Detail does not paginate; the field is set for struct completeness.
        cursor_ledger: r.created_at_ledger,
        participant_count: r.participant_count,
        latest_snapshot_ledger: r.latest_snapshot_ledger,
        reserve_a: r.reserve_a,
        reserve_b: r.reserve_b,
        total_shares: r.total_shares,
        // Filled by the handler from `fetch_pool_usd_analytics` (0199
        // compute-at-read); the snapshot columns are not read.
        tvl: None,
        volume: None,
        fee_revenue: None,
        latest_snapshot_at: r.latest_snapshot_at_ms.map(millis_to_utc),
    }))
}

#[derive(Debug, Row, Deserialize)]
struct CountRow {
    n: u64,
}

/// `true` if a real (non-sentinel) pool with this id exists. Gates 404 vs
/// 200-empty on participants/transactions/chart. CH `liquidity_pools` has no
/// `created_at_ledger` sentinel column (dropped); a row's presence is the
/// existence signal. No FINAL needed — existence is unaffected by un-merged
/// duplicate versions.
pub async fn pool_exists(
    client: &clickhouse::Client,
    pool_id_hex: &str,
) -> Result<bool, clickhouse::error::Error> {
    let row = client
        .query("SELECT count() AS n FROM liquidity_pools WHERE pool_id = unhex(?)")
        .bind(pool_id_hex)
        .fetch_one::<CountRow>()
        .await?;
    Ok(row.n > 0)
}

#[derive(Debug, Row, Deserialize)]
struct PoolLegsChRow {
    asset_a_type: i16,
    asset_a_code: String,
    asset_a_issuer_id: i64,
    asset_b_type: i16,
    asset_b_code: String,
    asset_b_issuer_id: i64,
}

/// The pool's two legs as `ids::asset_id` surrogates — the key
/// `lp_operation_amounts.asset_id` is written with (task 0279), so a row's
/// asset maps onto the A/B legs the page already renders. `None` when the pool
/// does not exist, which is also this seek's existence check (it replaces a
/// separate [`pool_exists`] round-trip on that path).
///
/// Resolved in Rust, not SQL: the surrogate is `cityhash_102_128`'s lower half
/// and CH's builtin `cityHash64` is a DIFFERENT algorithm (see the schema
/// header), so the writer's helper is the only way to reproduce the key.
///
/// Via [`ids::pool_leg_asset_id`], NOT `ids::asset_id` — `liquidity_pools`
/// stores the XDR asset type, where `2` is `credit_alphanum12`, while
/// `asset_id` reads `2` as the retired SAC facet and returns `0` for it.
pub async fn fetch_pool_asset_ids(
    client: &clickhouse::Client,
    pool_id_hex: &str,
) -> Result<Option<(i64, i64)>, clickhouse::error::Error> {
    let rows = client
        .query(
            "SELECT asset_a_type, asset_a_code, asset_a_issuer_id, \
                    asset_b_type, asset_b_code, asset_b_issuer_id \
             FROM liquidity_pools WHERE pool_id = unhex(?) \
             ORDER BY last_updated_ledger DESC LIMIT 1",
        )
        .bind(pool_id_hex)
        .fetch_all::<PoolLegsChRow>()
        .await?;
    Ok(rows.first().map(|r| {
        (
            ids::pool_leg_asset_id(r.asset_a_type, &r.asset_a_code, r.asset_a_issuer_id),
            ids::pool_leg_asset_id(r.asset_b_type, &r.asset_b_code, r.asset_b_issuer_id),
        )
    }))
}

#[derive(Debug, Row, Deserialize)]
struct ParticipantChRow {
    account_id_surrogate: i64,
    shares: String,
    share_percentage: Option<String>,
    first_deposit_ledger: i64,
    last_updated_ledger: i64,
}

/// `GET /v1/liquidity-pools/:id/participants` — active providers ordered by
/// `(shares DESC, account_id DESC)`. Mirrors the PG `fetch_participants`.
pub async fn fetch_participants(
    client: &clickhouse::Client,
    pool_id_hex: &str,
    cursor: Option<&SharesCursor>,
    limit: i64,
    direction: Direction,
) -> Result<Vec<ParticipantRow>, clickhouse::error::Error> {
    let (op, order) = keyset_sql_desc(direction);

    // Keyset expanded out of the natural `(shares, account_id) <op> (?, ?)`
    // tuple form on purpose: a Decimal128 inside a CH tuple comparison is the
    // documented "Decimal-tuple-compare" trap. The scalar `shares <op>
    // toDecimal128(...)` is proven safe. The bounds are inlined (not bound) for
    // the same reason accounts/contracts inline theirs — a `None` bound into a
    // keyset returns an empty page on clickhouse-rs 0.15. `shares` is validated
    // as a decimal string before inlining; a tampered cursor degrades to "no
    // keyset" (first page) rather than injecting.
    let keyset = match cursor {
        Some(c) if is_decimal_str(&c.shares) => format!(
            "AND ((lpp.shares {op} toDecimal128('{s}', 7)) \
                  OR (lpp.shares = toDecimal128('{s}', 7) AND lpp.account_id {op} {a}))",
            op = op,
            s = c.shares,
            a = c.account_id,
        ),
        _ => String::new(),
    };

    // `snap.ts` = total_shares of the latest snapshot within the freshness
    // window (NULL → stale pool → share_percentage NULL). The scalar subquery
    // is scoped to the literal pool (not correlated). CROSS JOIN broadcasts the
    // single value to every position row (PG `LEFT JOIN latest_snap ON TRUE`).
    let sql = format!(
        "SELECT \
            lpp.account_id                       AS account_id_surrogate, \
            toString(lpp.shares)                 AS shares, \
            if(snap.ts IS NULL OR snap.ts = toDecimal128(0, 7), NULL, \
               toString(lpp.shares * 100 / snap.ts)) AS share_percentage, \
            lpp.first_deposit_ledger             AS first_deposit_ledger, \
            lpp.last_updated_ledger              AS last_updated_ledger \
         FROM lp_positions lpp FINAL \
         CROSS JOIN ( \
            SELECT (SELECT total_shares FROM liquidity_pool_snapshots \
                     WHERE pool_id = unhex(?) \
                       AND ledger_sequence >= (SELECT max(sequence) FROM ledgers) - {fresh} \
                     ORDER BY ledger_sequence DESC LIMIT 1) AS ts \
         ) snap \
         WHERE lpp.pool_id = unhex(?) AND lpp.shares > 0 \
           {keyset} \
         ORDER BY lpp.shares {order}, lpp.account_id {order} \
         LIMIT ?",
        fresh = FRESHNESS_WINDOW_LEDGERS,
        keyset = keyset,
        order = order,
    );

    let rows = client
        .query(&sql)
        .bind(pool_id_hex)
        .bind(pool_id_hex)
        .bind(limit)
        .fetch_all::<ParticipantChRow>()
        .await?;

    // Resolve the provider StrKey by surrogate id (bloom seek) instead of a
    // whole-`accounts` `JOIN accounts acc FINAL` (task 0354). INNER-JOIN drop
    // semantics preserved via filter_map.
    //
    // "A position always has its account" holds today but is NOT guaranteed by
    // construction — it is maintained by operators. Measured on prod
    // (2026-08-04): all 6010 distinct `shares > 0` participants resolved, 0
    // missing. No non-test Rust path deletes an `accounts` row and prod's
    // retained mutation history has none, but two operator-driven paths can:
    //
    //   * `docs/runbooks/0225_backfill_crash_recovery.md` rolls back `accounts`
    //     on `last_seen_ledger` while rolling back `lp_positions` on
    //     `last_updated_ledger` — DIFFERENT watermarks, so an account touched
    //     inside the crashed range can lose its row while an older position
    //     survives. That is exactly the dangling surrogate below;
    //   * `repair_tier1::rebuild_accounts` replaces the whole table via
    //     `EXCHANGE TABLES` (`ch_staging::finalize`), where rows can disappear
    //     with no DELETE at all.
    //
    // So the log stays, and the failure mode is not proportional to the cause:
    // the drop happens BEFORE `finalize_page` reads the `limit + 1` sentinel,
    // so losing the sentinel row reports "no next page" and hides the REST of
    // the list, not one participant. Only 82 of the 26_489 pools with a live
    // participant hold more than one page, but the largest holds 684. `error!`
    // (not `debug!`) because the Lambda runs at `RUST_LOG=info` (0377 F3).
    let accounts = resolve_accounts(
        client,
        rows.iter().map(|r| r.account_id_surrogate).collect(),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let Some(account) = accounts.get(&r.account_id_surrogate).cloned() else {
                tracing::error!(
                    account_id_surrogate = r.account_id_surrogate,
                    pool_id = pool_id_hex,
                    "lp_positions row resolves to no accounts row: participant \
                     dropped, so participant_count disagrees with the list and \
                     pagination may terminate early"
                );
                return None;
            };
            Some(ParticipantRow {
                account,
                account_id_surrogate: r.account_id_surrogate,
                shares: r.shares,
                share_percentage: r.share_percentage,
                first_deposit_ledger: r.first_deposit_ledger,
                last_updated_ledger: r.last_updated_ledger,
            })
        })
        .collect())
}

/// One raw leg from `lp_operation_amounts` — the table's own grain, read in
/// sort-key order and paired in Rust. No `GROUP BY`: see
/// [`fetch_pool_activity`] for the measurement that removed it.
#[derive(Debug, Row, Deserialize)]
struct PoolLegChRow {
    ls: i64,
    tid: i64,
    ao: i16,
    asset_id: i64,
    amount: i64,
}

/// Transaction-level enrichment for the activity page's DISTINCT tx keys.
#[derive(Debug, Row, Deserialize)]
struct ActivityTxRow {
    id: i64,
    hash: String,
    source_id: i64,
    created_at_ms: i64,
}

/// The operation's own source account, `None` when it declares none (the XDR
/// default: the transaction's source).
#[derive(Debug, Row, Deserialize)]
struct OpSourceChRow {
    ls: i64,
    tid: i64,
    ao: i16,
    source_id: Option<i64>,
    /// `length(pool_ids)` — how many pools the whole operation crossed.
    pools_crossed: u64,
}

/// One operation's two legs, paired out of the key-ordered leg stream.
struct PairedOp {
    ls: i64,
    tid: i64,
    ao: i16,
    amount_a: Option<i64>,
    amount_b: Option<i64>,
}

impl PairedOp {
    /// `None` unless BOTH legs landed — the read stays total rather than
    /// classifying a half-row. `anyIf`-style defaulting would have made a
    /// missing leg read as `0` and turn a half-row into a "trade".
    fn event(&self) -> Option<PoolEvent> {
        match (self.amount_a, self.amount_b) {
            (Some(a), Some(b)) => Some(PoolEvent::from_signs(a, b)),
            _ => None,
        }
    }
}

/// Fold the key-ordered leg stream into operations.
///
/// The two legs of one operation are ADJACENT by construction: `asset_id` is
/// the last component of the sort key, so rows sharing
/// `(ledger_sequence, transaction_id, application_order)` are neighbours. That
/// is the whole reason this can be a fold instead of an aggregation.
///
/// `truncated` means the read hit its row cap, so the final group may be
/// missing a leg that simply did not fit — it is dropped and re-read from the
/// previous complete key on the next window.
fn pair_legs(rows: Vec<PoolLegChRow>, legs: (i64, i64), truncated: bool) -> Vec<PairedOp> {
    let (asset_a, asset_b) = legs;
    let mut out: Vec<PairedOp> = Vec::new();
    for r in rows {
        match out.last_mut() {
            Some(last) if last.ls == r.ls && last.tid == r.tid && last.ao == r.ao => {
                if r.asset_id == asset_a {
                    last.amount_a = Some(r.amount);
                } else if r.asset_id == asset_b {
                    last.amount_b = Some(r.amount);
                }
            }
            _ => out.push(PairedOp {
                ls: r.ls,
                tid: r.tid,
                ao: r.ao,
                amount_a: (r.asset_id == asset_a).then_some(r.amount),
                amount_b: (r.asset_id == asset_b).then_some(r.amount),
            }),
        }
    }
    if truncated {
        out.pop();
    }
    out
}

/// `GET /v1/liquidity-pools/:id/activity` — one row per (operation, pool),
/// task 0491.
///
/// **The driver table is the design.** `operation_pools` is keyed
/// `(pool_id, ledger_sequence, transaction_id)` with no `application_order`,
/// so it cannot page per operation. `lp_operation_amounts` is keyed
/// `(pool_id, ledger_sequence, transaction_id, application_order, asset_id)`
/// — the page's exact grain, reached by one PK-prefix seek.
///
/// **No `GROUP BY`, and that is measured, not stylistic.** The first cut of
/// this function pivoted the legs with `countIf`/`anyIf` and grouped by the
/// key triple. On prod's busiest pool (1.68M leg rows) that read **2.60M rows
/// / 109 ms / 182 MiB** to return 21 operations — a `GROUP BY` has to consume
/// the pool's whole slice before `ORDER BY … LIMIT` can pick the newest 21.
/// `optimize_aggregation_in_order` did not help (same rows, 253 ms). Reading
/// the same rows in sort-key order and pairing them here is **115k rows /
/// 9 ms / ~11 MiB** (median of 3), against **159k / 11 ms** for the
/// per-transaction endpoint this replaces: 22× off the first cut, and
/// slightly under the shape it supersedes.
///
/// Take the medians, not single runs — a cold run of EITHER shape reads
/// 0.7–1.0M rows, so one measurement each can invert the comparison. Measured
/// 2026-08-18 on prod; `log_comment` `lore0491-*` / `rep-*` in
/// `system.query_log`.
///
/// For the record: `FINAL` was never the cost — it added 22% (2.60M → 3.17M),
/// not an order of magnitude. It stays off because the producer is
/// deterministic (schema header's single-writer argument), so an unmerged
/// duplicate is byte-identical to its twin.
///
/// **Known consequence: an operation with no amount rows is not listed.** The
/// indexer writes `operation_pools` for an op that *declares* a pool whether
/// or not the transaction succeeded; amounts are written only for value that
/// actually moved. A failed explicit LP op therefore had a row under
/// `/transactions` and has none here — the page answers "what moved through
/// this pool", and a failed op moved nothing.
pub async fn fetch_pool_activity(
    client: &clickhouse::Client,
    pool_id_hex: &str,
    asset_ids: (i64, i64),
    limit: i64,
    cursor: Option<&PoolActivityCursor>,
    direction: Direction,
    event: Option<PoolEvent>,
) -> Result<Vec<PoolActivityRow>, clickhouse::error::Error> {
    let (op, order) = keyset_sql_desc(direction);

    // Where the next window resumes. `<` on the whole triple skips the last
    // kept operation outright — both its legs share that triple, so there is
    // no half-operation to step over.
    let mut after: Option<(i64, i64, i16)> =
        cursor.map(|c| (c.ledger_sequence, c.transaction_id, c.application_order));

    // Two legs per operation, plus slack so the cap rarely lands mid-op.
    let mut window = (limit * 2 + 2).max(64);
    let mut ops: Vec<PairedOp> = Vec::new();

    // One pass when unfiltered (the common case). With `filter[event]` the
    // matching rate is unknown up front — the event is only knowable once both
    // legs are in hand — so the window doubles until the page fills or the
    // pool runs out. Geometric growth keeps this O(log) round trips and never
    // reads more than ~2× the span it had to cover; a linear re-poll would be
    // the slow version of the same idea.
    loop {
        let keyset = match after {
            Some((ls, tid, ao)) => format!(
                " AND (ledger_sequence, transaction_id, application_order) {op} ({ls}, {tid}, {ao})"
            ),
            None => String::new(),
        };
        let sql = format!(
            "SELECT \
                ledger_sequence   AS ls, \
                transaction_id    AS tid, \
                application_order AS ao, \
                asset_id          AS asset_id, \
                amount            AS amount \
             FROM lp_operation_amounts \
             WHERE pool_id = toFixedString(unhex(?), 32) \
               AND ledger_sequence <= (SELECT max(sequence) FROM ledgers) {keyset} \
             ORDER BY ls {order}, tid {order}, ao {order} \
             LIMIT {window}"
        );
        let rows = client
            .query(&sql)
            .bind(pool_id_hex)
            .fetch_all::<PoolLegChRow>()
            .await?;

        let exhausted = (rows.len() as i64) < window;
        let batch = pair_legs(rows, asset_ids, !exhausted);
        if let Some(last) = batch.last() {
            after = Some((last.ls, last.tid, last.ao));
        }

        match event {
            Some(want) => ops.extend(batch.into_iter().filter(|o| o.event() == Some(want))),
            None => ops.extend(batch),
        }

        if exhausted || (ops.len() as i64) >= limit {
            break;
        }
        window *= 2;
    }
    ops.truncate(limit as usize);
    if ops.is_empty() {
        return Ok(Vec::new());
    }

    // Enrich the page's DISTINCT transactions — several operations of one
    // transaction share a row here, so this set is smaller than the page.
    // Keys inlined (i64) with the partition prune that turns the
    // `(ledger_sequence, id) IN (…)` filter into a tight PK seek, same shape
    // as `common::ch::fetch_tx_list_aggregates`.
    let tx_keys: std::collections::BTreeSet<(i64, i64)> =
        ops.iter().map(|o| (o.ls, o.tid)).collect();
    let in_tuples = tx_keys
        .iter()
        .map(|(ls, tid)| format!("({ls},{tid})"))
        .collect::<Vec<_>>()
        .join(",");
    let partitions = tx_keys
        .iter()
        .map(|(ls, _)| ls / 500_000)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let detail_sql = format!(
        "SELECT \
            t.id                                 AS id, \
            lower(hex(t.hash))                   AS hash, \
            t.source_id                          AS source_id, \
            toUnixTimestamp64Milli(l.closed_at)  AS created_at_ms \
         FROM transactions t \
         INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
         WHERE (t.ledger_sequence, t.id) IN ({in_tuples}) \
           AND intDiv(t.ledger_sequence, 500000) IN ({partitions}) \
         LIMIT 1 BY t.id"
    );
    let txs = client
        .query(&detail_sql)
        .fetch_all::<ActivityTxRow>()
        .await?;
    let by_tx: HashMap<i64, &ActivityTxRow> = txs.iter().map(|t| (t.id, t)).collect();

    // The OPERATION's own source account. A Stellar operation may declare one,
    // and then it — not the transaction's source — is who performed this
    // operation; `operations_appearances.source_id` is NULL when it does not,
    // which per the XDR means "same as the transaction's". Showing the
    // transaction's source on a per-operation row names the wrong account
    // whenever they differ (measured on prod: 41% of ops in a recent ledger
    // window declare their own, and stellar.expert shows that one).
    //
    // `(ledger_sequence, transaction_id, application_order)` IS this table's
    // sort key, so the page's bounded IN-list is a PK seek with the same
    // partition prune. `max()` rather than `LIMIT 1 BY`: the table holds one
    // row per APPEARANCE, so an operation has several, and aggregation skips
    // the NULLs instead of picking one arbitrarily.
    //
    // `pools_crossed` rides the same seek for free: `pool_ids` is the op's
    // sorted+deduped crossing list, written identically on every appearance
    // row (stage.rs fans the one list out), so `max(length(...))` is just
    // "the length". It is what lets a row say "this trade was one hop of an
    // N-pool route" without carrying the route itself — the route lives on
    // the op's detail page, which the row already links to.
    let op_sources_sql = format!(
        "SELECT \
            ledger_sequence   AS ls, \
            transaction_id    AS tid, \
            application_order AS ao, \
            max(source_id)    AS source_id, \
            max(length(pool_ids)) AS pools_crossed \
         FROM operations_appearances \
         WHERE (ledger_sequence, transaction_id) IN ({in_tuples}) \
           AND intDiv(ledger_sequence, 500000) IN ({partitions}) \
         GROUP BY ls, tid, ao"
    );
    let op_sources = client
        .query(&op_sources_sql)
        .fetch_all::<OpSourceChRow>()
        .await?;
    let by_op: HashMap<(i64, i64, i16), (Option<i64>, u64)> = op_sources
        .iter()
        .map(|r| ((r.ls, r.tid, r.ao), (r.source_id, r.pools_crossed)))
        .collect();

    // Source StrKeys by surrogate id (bloom seek) rather than a whole-
    // `accounts` INNER JOIN — task 0354. One resolve for both kinds of source.
    let account_ids = txs
        .iter()
        .map(|t| t.source_id)
        .chain(by_op.values().filter_map(|(src, _)| *src))
        .collect();
    let accounts = resolve_accounts(client, account_ids).await?;

    // A page row whose transaction did not resolve is DROPPED, not rendered
    // half-blank: it would have no hash to link and no timestamp to sort by.
    // Unreachable unless the tx tables lag the amounts table, and the
    // `max(sequence)` fence above already keeps the seek behind the commit
    // marker.
    Ok(ops
        .into_iter()
        .filter_map(|o| {
            let tx = by_tx.get(&o.tid)?;
            // The operation's own source, falling back to the transaction's —
            // which is what the XDR's absent `sourceAccount` means.
            let (op_source, pools_crossed) = by_op
                .get(&(o.ls, o.tid, o.ao))
                .copied()
                .map_or((None, None), |(src, n)| (src, Some(n as i64)));
            let source_id = op_source.unwrap_or(tx.source_id);
            let source_account = accounts.get(&source_id)?.clone();
            let event = o.event();
            Some(PoolActivityRow {
                transaction_hash: tx.hash.clone(),
                ledger_sequence: o.ls,
                transaction_id: o.tid,
                application_order: o.ao,
                event,
                amount_a: event.and(o.amount_a).map(|v| v.to_string()),
                amount_b: event.and(o.amount_b).map(|v| v.to_string()),
                source_account,
                pools_crossed,
                created_at: millis_to_utc(tx.created_at_ms),
            })
        })
        .collect())
}

/// Money arrives as raw `Nullable(Float64)` and is formatted by [`usd_str`]
/// on the Rust side; `fee_revenue` is derived from `volume` here rather
/// than in SQL, so chart and detail run identical arithmetic.
#[derive(Debug, Row, Deserialize)]
struct ChartChRow {
    bucket_ms: i64,
    tvl: Option<f64>,
    volume: Option<f64>,
    samples_in_bucket: u64,
}

/// `GET /v1/liquidity-pools/:id/chart` — time-bucketed TVL / volume / fee
/// series, USD computed at read (task 0199, ADR 0053).
///
/// CH translation choices:
/// - **Bucket truncation** maps the `1h | 1d | 1w` allowlist to
///   `toStartOfHour` / `toStartOfDay` / `toMonday`. `toMonday` is the
///   Monday-start week, matching PG's ISO `date_trunc('week', …)` — the
///   contract the endpoint launched with. CH's other spellings both miss
///   it (box-verified 2026-08-05): `toStartOfWeek` defaults to SUNDAY
///   (mode 0, one day off ISO), and the reference SQL's epoch-aligned
///   `toStartOfInterval(…, INTERVAL 604800 SECOND)` buckets on THURSDAYS —
///   1970-01-01 was a Thursday, so 7-day blocks from epoch all are
///   (2026-08-04 → bucket 2026-07-30, toDayOfWeek = 4). An earlier version
///   of this comment claimed "Sunday-aligned"; that was wrong.
/// - **No `created_at` on CH snapshots** — the window is filtered on the
///   joined `ledgers.closed_at` (bijection with `ledger_sequence`), so the
///   `from`/`to` API contract (RFC3339 timestamps) is preserved unchanged
///   rather than switched to the ledger-bound form the reference SQL used.
/// - **`pool_id = unhex(?)`** is a leading-PK seek on
///   `liquidity_pool_snapshots` (`ORDER BY (pool_id, ledger_sequence)`), so
///   the scan is bounded to this pool's snapshots — box-measured 14.5 M rows
///   / 237 MB for the hottest pool (1.84 M snapshots) over a 90-day 1d window.
///
/// USD semantics (per bucket):
/// - **TVL** is a state quantity — the last snapshot in the bucket whose
///   own price bucket is priced: `reserve_a·close_usd_a + reserve_b·close_usd_b`.
///   NULL unless BOTH legs price (a one-leg TVL silently halves the pool).
///   `argMaxIf(…, isNotNull(tvl_row))` deliberately falls back to the last
///   PRICEABLE snapshot in the bucket (≤ one bucket of intra-bucket
///   staleness) instead of NULLing the bucket on a missing tip price.
/// - **volume** is a flow quantity — `sum(gross_volume_a × close_usd_a)`,
///   each ledger priced at its OWN price bucket. If any swap row in the
///   bucket lacks a leg-A price the bucket's volume is NULL (an honest
///   hole), never a silent partial sum. `sum()` over an all-NULL bucket
///   (no swaps) yields NULL (box-confirmed), matching the PG contract.
///   NOTE: the veto is all-or-nothing per bucket, so at `1w` a single
///   unpriced ledger discards the week — deliberate for now (an unmarked
///   partial sum reads as a real number), revisit with a coverage field.
/// - **fee_revenue** is derived in Rust from `volume` ([`fee_revenue_usd`]).
///
/// Prices join (contract: prices views.sql, pinned 2026-06-16):
/// - Grain follows the interval: `1h` → `prices.price_usd_series_1h` on
///   `toStartOfHour(closed_at)`; `1d`/`1w` → `prices.price_usd_series` on
///   `toStartOfDay(closed_at)` (weekly candles are not provided — a 1w
///   bucket's TVL prices at its last snapshot's DAY).
/// - **`ASOF LEFT JOIN` on `price.bucket <= ledger.price_bucket`**, capped
///   at [`MAX_PRICE_CARRY_SECONDS`]: a candle exists only once the asset
///   trades in that bucket, so exact equality left the newest point of
///   every illiquid-leg pool NULL (box-reproduced). ASOF needs an equi-join
///   column, hence the constant `k` on both sides — a real column, not the
///   `ON 1 = 1` form that pins the join algorithm to `hash`. The
///   subqueries' lower bound is widened by the same cap so the FIRST bucket
///   can carry forward too.
/// - The staleness cap is what keeps carry-forward honest: without it the
///   2026-07-21..08-03 provider freeze would render as live TVL priced off
///   a 12-day-old candle.
/// - Identity + bucket-range predicates live INSIDE the right-side
///   subqueries: the bucket range is what bounds the view's scan of
///   `price_ohlcv_*` (their header's pushdown note); one identity per side
///   keeps the hash tables at ≤ one row per bucket.
/// - A join miss yields DEFAULT (epoch `bucket`, `0` close), not NULL
///   (`join_use_nulls` is rejected for the readonly user) — the staleness
///   test rejects it, since an epoch bucket is always further back than the
///   cap. `nullIf(close_usd, 0)` guards the priced-but-zero case; the views
///   already filter `close_usd > 0`.
/// - The **in-progress price bucket is excluded** (`least(to, grain(now))`).
///   It is only partly enriched, so its weighted close can be a dust print —
///   see [`fetch_last_closes`] for the measured case and the prices owner's
///   confirmation. The ASOF carry then prices the newest chart bucket off
///   the last CLOSED price bucket, which is exactly what the carry is for.
pub async fn fetch_pool_chart(
    client: &clickhouse::Client,
    pool_id_hex: &str,
    ctx: &PoolPriceContext,
    interval: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<ChartDataPoint>, clickhouse::error::Error> {
    // Defensive second gate (the handler validates against the allowlist
    // first) — fail loud on allowlist drift rather than emit a wrong bucket.
    assert!(
        matches!(interval, "1h" | "1d" | "1w"),
        "fetch_pool_chart called with non-allowlisted interval `{interval}` — \
         handler validation drift; expected 1h | 1d | 1w"
    );
    let bucket_fn = match interval {
        "1h" => "toStartOfHour",
        "1d" => "toStartOfDay",
        "1w" => "toMonday",
        _ => unreachable!("interval validated against the 1h|1d|1w allowlist above"),
    };
    let (series_view, price_bucket_fn) = match interval {
        "1h" => ("prices.price_usd_series_1h", "toStartOfHour"),
        "1d" | "1w" => ("prices.price_usd_series", "toStartOfDay"),
        _ => unreachable!("interval validated against the 1h|1d|1w allowlist above"),
    };

    // `bucket_ms`: each truncated bucket is coerced to a UTC `DateTime64(3)`
    // then to epoch millis, so `millis_to_utc` round-trips it on the Rust side
    // (matches the `DateTime<Utc>` shape PG returns from `date_trunc`).
    //
    // **NO `FINAL`** (0356 / PR #318): `sum()`/`count()` over bucketed snapshots
    // must see exactly one row per ledger, so a bare `FROM … FINAL` can't just be
    // dropped — pre-cleanup before/after duplicates would double-count volume /
    // fee_revenue / samples. Instead the inner subquery collapses to one row per
    // ledger (`LIMIT 1 BY ledger_sequence`) with no merge; tvl/volume/fee are
    // identical across a duplicate pair, so which row survives is irrelevant. The
    // outer bucket aggregation is then byte-identical to the old `FINAL` form.
    //
    // The outer `ledgers` read is ALSO deduped — via a `LIMIT 1 BY sequence`
    // subquery, not a bare `JOIN ledgers` (lore-0420). `ledgers` is itself a
    // ReplacingMergeTree with unmerged duplicate rows, so a bare join doubled
    // every snapshot (measured 2806 samples vs 1403 distinct) → doubled
    // `sum(volume)`/`sum(fee_revenue)`/`samples_in_bucket`. The join needs
    // `l.closed_at` for the bucket key, so a pure `IN` semi-join won't do;
    // `LIMIT 1 BY sequence` keeps one row per ledger (closed_at is identical
    // across a dup pair, so which survives is irrelevant) — same idiom as the
    // snapshot dedup above, and it keeps `closed_at` a plain column so the
    // window filter can stay in the subquery WHERE and drive minmax pruning.
    //
    // The inner read is bounded to the window's ledger range, resolved from
    // `[from, to]` against `ledgers.closed_at` (minmax skip index).
    //
    // The upper-bound subquery carries a lower bound too. That reads as
    // redundant — it is not. A minmax index can only skip granules that
    // *cannot* match, and `closed_at < to` alone matches all of history, so
    // that subquery scanned the full 26M-row table to find the last ledger
    // before `to` — 37% of the box's total read work at 50M req/month. The
    // extra `closed_at >= from` is what gives the index something to prune
    // (26M → ~209k). The value is unchanged either way: ledgers close every
    // ~5 s with no gaps, so the last ledger before `to` always falls inside
    // `[from, to)`.
    let sql = format!(
        "SELECT \
            bucket_ms, \
            argMaxIf(tvl_row, ledger_sequence, isNotNull(tvl_row)) AS tvl, \
            if(countIf(unpriced_swap) > 0, NULL, sum(vol_row))     AS volume, \
            count()                                                AS samples_in_bucket \
         FROM ( \
             SELECT \
                toUnixTimestamp64Milli(toDateTime64({bucket_fn}(l.closed_at), 3, 'UTC')) AS bucket_ms, \
                lps.ledger_sequence                              AS ledger_sequence, \
                if(dateDiff('second', pa.bucket, l.price_bucket) <= {carry}, \
                   nullIf(toFloat64(pa.close_usd), 0), NULL)      AS pa_usd, \
                if(dateDiff('second', pb.bucket, l.price_bucket) <= {carry}, \
                   nullIf(toFloat64(pb.close_usd), 0), NULL)      AS pb_usd, \
                toFloat64(lps.reserve_a) * pa_usd \
                    + toFloat64(lps.reserve_b) * pb_usd          AS tvl_row, \
                toFloat64(lps.gross_volume_a) * pa_usd           AS vol_row, \
                isNotNull(lps.gross_volume_a) AND isNull(pa_usd) AS unpriced_swap \
             FROM ( \
                 SELECT ledger_sequence, reserve_a, reserve_b, gross_volume_a \
                 FROM liquidity_pool_snapshots \
                 WHERE pool_id = unhex(?) \
                   AND ledger_sequence >= (SELECT min(sequence) FROM ledgers WHERE closed_at >= fromUnixTimestamp64Milli(?)) \
                   AND ledger_sequence <= (SELECT max(sequence) FROM ledgers WHERE closed_at >= fromUnixTimestamp64Milli(?) AND closed_at < fromUnixTimestamp64Milli(?)) \
                 ORDER BY ledger_sequence DESC \
                 LIMIT 1 BY ledger_sequence \
             ) lps \
             JOIN ( \
                 SELECT 1 AS k, sequence, closed_at, {price_bucket_fn}(closed_at) AS price_bucket \
                 FROM ledgers \
                 WHERE closed_at >= fromUnixTimestamp64Milli(?) \
                   AND closed_at <  fromUnixTimestamp64Milli(?) \
                 LIMIT 1 BY sequence \
             ) l ON l.sequence = lps.ledger_sequence \
             ASOF LEFT JOIN ( \
                 SELECT 1 AS k, bucket, close_usd \
                 FROM {series_view} \
                 WHERE asset_kind = ? AND asset_code = ? AND issuer_address = ? \
                   AND bucket >= {price_bucket_fn}(fromUnixTimestamp64Milli(?)) - INTERVAL {carry} SECOND \
                   AND bucket <  least(fromUnixTimestamp64Milli(?), {price_bucket_fn}(now())) \
                   AND close_usd > 0 \
             ) pa ON pa.k = l.k AND pa.bucket <= l.price_bucket \
             ASOF LEFT JOIN ( \
                 SELECT 1 AS k, bucket, close_usd \
                 FROM {series_view} \
                 WHERE asset_kind = ? AND asset_code = ? AND issuer_address = ? \
                   AND bucket >= {price_bucket_fn}(fromUnixTimestamp64Milli(?)) - INTERVAL {carry} SECOND \
                   AND bucket <  least(fromUnixTimestamp64Milli(?), {price_bucket_fn}(now())) \
                   AND close_usd > 0 \
             ) pb ON pb.k = l.k AND pb.bucket <= l.price_bucket \
         ) \
         GROUP BY bucket_ms \
         ORDER BY bucket_ms ASC",
        bucket_fn = bucket_fn,
        price_bucket_fn = price_bucket_fn,
        series_view = series_view,
        carry = MAX_PRICE_CARRY_SECONDS,
    );

    let rows = client
        .query(&sql)
        .bind(pool_id_hex)
        .bind(from.timestamp_millis()) // min(sequence): closed_at >= from
        .bind(from.timestamp_millis()) // max(sequence): closed_at >= from
        .bind(to.timestamp_millis()) // max(sequence): closed_at <  to
        .bind(from.timestamp_millis()) // ledgers dedup subquery: closed_at >= from
        .bind(to.timestamp_millis()) // ledgers dedup subquery: closed_at <  to
        .bind(ctx.leg_a.kind) // pa: identity
        .bind(ctx.leg_a.code.as_str())
        .bind(ctx.leg_a.issuer.as_str())
        .bind(from.timestamp_millis()) // pa: bucket >= floor(from)
        .bind(to.timestamp_millis()) // pa: bucket < to
        .bind(ctx.leg_b.kind) // pb: identity
        .bind(ctx.leg_b.code.as_str())
        .bind(ctx.leg_b.issuer.as_str())
        .bind(from.timestamp_millis()) // pb: bucket >= floor(from)
        .bind(to.timestamp_millis()) // pb: bucket < to
        .fetch_all::<ChartChRow>()
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| ChartDataPoint {
            bucket: millis_to_utc(r.bucket_ms),
            tvl: r.tvl.map(usd_str),
            volume: r.volume.map(usd_str),
            fee_revenue: r.volume.map(|v| usd_str(fee_revenue_usd(v, ctx.fee_bps))),
            samples_in_bucket: r.samples_in_bucket as i64,
        })
        .collect())
}

/// SELECT column order MUST match this struct (clickhouse positional decode).
#[derive(Debug, Row, Deserialize)]
struct PoolListChRow {
    pool_id_hex: String,
    asset_a_type: i16,
    asset_a_code: Option<String>,
    asset_a_issuer_id: i64,
    asset_a_contract_id: Option<String>,
    asset_a_icon_url: Option<String>,
    asset_b_type: i16,
    asset_b_code: Option<String>,
    asset_b_issuer_id: i64,
    asset_b_contract_id: Option<String>,
    asset_b_icon_url: Option<String>,
    fee_bps: i32,
    created_at_ledger: i64,
    /// `last_updated_ledger` — the list sort/cursor key (see fn doc).
    cursor_ledger: i64,
    participant_count: i64,
    latest_snapshot_ledger: Option<i64>,
    reserve_a: Option<String>,
    reserve_b: Option<String>,
    total_shares: Option<String>,
    latest_snapshot_at_ms: Option<i64>,
}

/// `GET /v1/liquidity-pools` — paginated pool list. Mirrors the PG
/// `fetch_pool_list` projection, with two CH-specific structural choices
/// driven by the box-measured read cost (`liquidity_pool_snapshots` = 268 M
/// rows):
///
/// - **Order key = `last_updated_ledger` (NOT `created_at_ledger`).** PG keys
///   on `created_at_ledger` (pool creation). CH `liquidity_pools` dropped that
///   column (PR #175); its only in-window proxy — `min(snapshot
///   ledger_sequence)` — is clamped to the frozen backfill floor (≈ L50.4M)
///   for every pre-window pool, so it is useless as an order key (mass ties)
///   *and* would force a full 268 M-row snapshot GROUP BY just to derive it.
///   `last_updated_ledger` is a native non-NULL column → the list pages the
///   small `liquidity_pools` table (51 k rows) FIRST, then seeks snapshots /
///   positions for only the page's ≤ limit+1 pools (`pool_id` is their leading
///   PK). Box-measured ≈ 55 M rows/page vs ≈ 268 M for the full-scan shape.
///   The wire `created_at_ledger` field still reports the min-snapshot proxy
///   (parity with detail); only the *ordering* differs, and the FE does not
///   consume the list yet, so there is no live ordering regression.
/// - **No `min_tvl` pre-filter.** It used to exist as a `tvl_pools` CTE doing
///   a full-scan `argMax(tvl)` over the snapshot column — a column task 0199
///   established is never written, so it matched nothing. The parameter is
///   now rejected with 400 at the handler rather than silently returning an
///   empty page that contradicts the per-row `tvl` this function computes.
///   Restoring it needs TVL for ALL pools per request (it changes page
///   membership, so it cannot ride the per-page price lookup) — i.e. the
///   prices-side identity-keyed materialized series.
///
/// Read-cost note for the eventual flag flip: the per-page ≈ 55 M is dominated
/// by the `accounts` id→strkey issuer resolution (14 M, non-PK reverse lookup)
/// and the `ledgers` closed_at join; both are bounded and the list is
/// user-initiated (not polled). The `operations_appearances` projection that
/// blocks the transactions endpoint does NOT block the list.
pub async fn fetch_pool_list(
    client: &clickhouse::Client,
    params: &ResolvedPoolListParams,
    direction: Direction,
) -> Result<Vec<PoolRow>, clickhouse::error::Error> {
    let (op, order) = keyset_sql_desc(direction);

    // Keyset on `(last_updated_ledger, pool_id)`, expanded to scalar
    // comparisons. The cursor's `created_at_ledger` slot carries
    // `last_updated_ledger` on the CH path (opaque, ADR 0008). Bounds inlined:
    // `cursor_ledger` is i64 (no injection); `pool_id_hex` is validated hex.
    // A tampered/non-hex cursor degrades to "no keyset" (first page).
    let keyset = match params.cursor.as_ref() {
        Some(c) if is_hex_pool_id(&c.pool_id_hex) => format!(
            "AND ((lp.last_updated_ledger {op} {cl}) \
                  OR (lp.last_updated_ledger = {cl} \
                      AND lower(hex(lp.pool_id)) {op} '{ph}'))",
            op = op,
            cl = c.created_at_ledger,
            ph = c.pool_id_hex,
        ),
        _ => String::new(),
    };

    // Asset filters are bound (untrusted free-text codes / handler-validated
    // issuer StrKeys — clickhouse-rs escapes them). Each `?` appears in the
    // `page` CTE WHERE in this exact push order. Issuer StrKey → surrogate id
    // resolves via an `accounts` PK seek (`ORDER BY (account_id)`), cheap.
    // NO relevance ranking anywhere on the pools path, by decision (task
    // 0485). Measured on production, the first page of
    // `filter[asset_codes]=XLM` is already 20 of 25 real native-leg pools —
    // they are the busiest on the network, so activity surfaces them without
    // a rule. A tier over the legs was built and taken back out: 46 lines of
    // the densest SQL in the change, for five look-alikes on page one.
    let mut binds: Vec<String> = Vec::new();
    let mut filters = String::new();
    if let Some(code) = params.asset_a_code.as_ref() {
        filters.push_str(" AND lp.asset_a_code = ?");
        binds.push(code.clone());
    }
    if let Some(iss) = params.asset_a_issuer.as_ref() {
        filters.push_str(
            " AND lp.asset_a_issuer_id = \
              (SELECT id FROM accounts FINAL WHERE account_id = ? LIMIT 1)",
        );
        binds.push(iss.clone());
    }
    if let Some(code) = params.asset_b_code.as_ref() {
        filters.push_str(" AND lp.asset_b_code = ?");
        binds.push(code.clone());
    }
    if let Some(iss) = params.asset_b_issuer.as_ref() {
        filters.push_str(
            " AND lp.asset_b_issuer_id = \
              (SELECT id FROM accounts FINAL WHERE account_id = ? LIMIT 1)",
        );
        binds.push(iss.clone());
    }
    // Asset-code needles (0440 / issue #366).
    //
    // Substring, not equality: `USD` has to match the `USDC` pools the user can
    // see on the page. `position` takes the needle literally — no LIKE wildcards
    // and no regex to escape, so caller free-text cannot widen its own match.
    // Case-insensitive here rather than `upper()` on the column: same result,
    // one pass, and it keeps working if the needle ever arrives un-normalized.
    //
    // Native legs are stored with an empty code (`asset_type = 0`, code `''`)
    // while every surface — this list included — renders them as `XLM`. Without
    // the alias, `XLM` matches none of the 11.7k pools that actually hold native
    // XLM, and instead returns ~3.7k pools of credit assets someone minted under
    // the code `XLM` (they exist, including `XLM/XLM` pairs). That is not an
    // empty result, it is a confident wrong one — so the predicate searches what
    // the row displays as.
    //
    // A pair assigns each needle its OWN leg, in either order, rather than
    // asking each needle independently whether it matches somewhere. The
    // difference only shows when the needles overlap, and then it is the whole
    // answer: `USDC/USDC` means the 72 pools with USDC on both sides, not the
    // 2 912 with USDC anywhere. Same for a needle that is a prefix of the other
    // (`USD/USDC`) — one asset must not satisfy both halves of the query.
    //
    // The predicate itself lives in `common::pool_asset_codes` because global
    // search matches pools with the SAME rule (task 0470); two copies would
    // drift, and the native arm above is exactly where a second one goes wrong.
    //
    // A pool identifier in the same box wins outright: it names one pool, so
    // it is a point seek on the primary key rather than a scan, and there is
    // nothing left for a code match to narrow. Before this, the identifier was
    // matched as a substring of an asset code and the page said "no pools".
    if let Some(pool_hex) = params.pool_id_hex.as_ref() {
        filters.push_str(" AND lp.pool_id = unhex(?)");
        binds.push(pool_hex.clone());
    } else if let Some((clause, clause_binds)) =
        asset_codes_predicate(params.asset_codes.as_slice())
    {
        filters.push_str(&format!(" AND {clause}"));
        binds.extend(clause_binds);
    }

    // Latest-snapshot fields via `argMax(...) GROUP BY pool_id` over a bounded
    // `ledger_sequence` band around the page's `last_updated_ledger` range (the
    // `band` CTE, ±10k). Page pools are the most-recently-updated, so their
    // latest snapshot sits in that band — a bounded seek (~0.5M rows / ~50ms)
    // instead of a full per-pool history scan (30M rows, which OOMed the 4 GB
    // read-only profile as PR #335's `LIMIT 1 BY` sort). NO `FINAL`: the band's
    // max ledger per page pool is recent (post-0356/#318 single-image) so
    // per-column `argMax` can't tear; only a pool whose latest snapshot predates
    // #318 (inactive for weeks → deep pages) could, which is accepted.
    // `created_at_ledger` = `min(ledger_sequence)` in the `cr` subquery (cheap
    // narrow streaming scan, dup-invariant → no `FINAL`); `l_snap` seeks
    // `ledgers` by the page's ~20 `last_updated_ledger`s (a full `ledgers` join
    // built a 26M-row / 3.3 GB hash); `sac`/`asset_sac` prune to the page codes.
    //
    // Do NOT rewrite as `ORDER BY ledger_sequence DESC LIMIT 1 BY pool_id`
    // (PR #335, reverted): `LIMIT 1 BY` is NOT a seek — it fully materialises +
    // sorts every snapshot of the page's pools (~30M rows for the busiest 20),
    // OOMing the 4 GB read-only CH profile. A future perf pass must keep the
    // O(page pools) shape (e.g. `argMax` over a whole-row tuple), not a sort.
    //
    // Aggregates wrap in `toNullable(...)` so a no-snapshot pool yields NULL (not
    // the 0/'' default) on the LEFT JOIN miss — `join_use_nulls` is rejected for
    // the read-only CH user, so this is the readonly-safe NULL path. (Every
    // current pool has ≥ 1 snapshot, so this is defensive.) `nullIf(...)` does the
    // same for the empty-string-sentinel string columns. Native legs
    // (asset_code = '') are excluded from the SAC join by the `lp.asset_*_code !=
    // ''` guard so they surface a NULL `contract_id`, matching PG (NULL code → no
    // SAC match).
    let sql = format!(
        "WITH \
         page AS ( \
             SELECT lp.pool_id AS pool_id, lp.asset_a_type AS asset_a_type, \
                    lp.asset_a_code AS asset_a_code, \
                    lp.asset_a_issuer_id AS asset_a_issuer_id, \
                    lp.asset_b_type AS asset_b_type, lp.asset_b_code AS asset_b_code, \
                    lp.asset_b_issuer_id AS asset_b_issuer_id, lp.fee_bps AS fee_bps, \
                    lp.last_updated_ledger AS last_updated_ledger \
             FROM liquidity_pools lp FINAL \
             WHERE 1 = 1{filters} {keyset} \
             ORDER BY last_updated_ledger {order}, pool_id {order} \
             LIMIT {limit} \
         ), \
         band AS ( \
             SELECT min(last_updated_ledger) - 10000 AS lo, \
                    max(last_updated_ledger) + 10000 AS hi FROM page \
         ), \
         codes AS ( \
             SELECT asset_a_code AS c FROM page \
             UNION ALL SELECT asset_b_code FROM page \
         ), \
         sac AS ( \
             SELECT a.asset_code AS asset_code, a.issuer_id AS issuer_id, \
                    max(sc.contract_id)      AS contract_id, \
                    nullIf(max(ae.icon_url), '') AS icon_url \
             FROM assets a \
             LEFT JOIN ( \
                 SELECT asset_type, asset_code, issuer_id, contract_id, \
                        max(sac_contract_id) AS sac_contract_id \
                 FROM asset_sac \
                 WHERE asset_type IN (0, 1) AND asset_code IN (SELECT c FROM codes) \
                 GROUP BY asset_type, asset_code, issuer_id, contract_id \
             ) asac ON asac.asset_type = a.asset_type AND asac.asset_code = a.asset_code \
                   AND asac.issuer_id = a.issuer_id AND asac.contract_id = a.contract_id \
             LEFT JOIN soroban_contracts sc ON sc.id = asac.sac_contract_id AND asac.sac_contract_id != 0 \
             LEFT JOIN ( \
                 SELECT asset_type, asset_code, issuer_id, contract_id, \
                        argMax(icon_url, version) AS icon_url \
                 FROM asset_enrichment \
                 WHERE asset_type IN (0, 1) AND asset_code IN (SELECT c FROM codes) \
                 GROUP BY asset_type, asset_code, issuer_id, contract_id \
             ) ae ON ae.asset_type = a.asset_type AND ae.asset_code = a.asset_code \
                 AND ae.issuer_id = a.issuer_id AND ae.contract_id = a.contract_id \
             WHERE a.asset_type IN (0, 1) AND a.asset_code IN (SELECT c FROM codes) \
               AND (a.asset_code, a.issuer_id) IN ( \
                   SELECT asset_a_code, asset_a_issuer_id FROM page \
                   UNION ALL SELECT asset_b_code, asset_b_issuer_id FROM page) \
             GROUP BY a.asset_code, a.issuer_id \
         ) \
         SELECT \
             lower(hex(lp.pool_id))                          AS pool_id_hex, \
             lp.asset_a_type                                 AS asset_a_type, \
             nullIf(lp.asset_a_code, '')                     AS asset_a_code, \
             lp.asset_a_issuer_id                            AS asset_a_issuer_id, \
             nullIf(sac_a.contract_id, '')                   AS asset_a_contract_id, \
             sac_a.icon_url                                  AS asset_a_icon_url, \
             lp.asset_b_type                                 AS asset_b_type, \
             nullIf(lp.asset_b_code, '')                     AS asset_b_code, \
             lp.asset_b_issuer_id                            AS asset_b_issuer_id, \
             nullIf(sac_b.contract_id, '')                   AS asset_b_contract_id, \
             sac_b.icon_url                                  AS asset_b_icon_url, \
             lp.fee_bps                                      AS fee_bps, \
             ifNull(cr.created_at_ledger, lp.last_updated_ledger) AS created_at_ledger, \
             lp.last_updated_ledger                          AS cursor_ledger, \
             toInt64(ifNull(pc.participant_count, 0))        AS participant_count, \
             s.latest_ledger_sequence                        AS latest_snapshot_ledger, \
             toString(s.reserve_a)                           AS reserve_a, \
             toString(s.reserve_b)                           AS reserve_b, \
             toString(s.total_shares)                        AS total_shares, \
             nullIf(toUnixTimestamp64Milli(l_snap.closed_at), 0) AS latest_snapshot_at_ms \
         FROM page lp \
         LEFT JOIN ( \
             SELECT pool_id, \
                toNullable(max(ledger_sequence))                  AS latest_ledger_sequence, \
                argMax(toNullable(reserve_a), ledger_sequence)    AS reserve_a, \
                argMax(toNullable(reserve_b), ledger_sequence)    AS reserve_b, \
                argMax(toNullable(total_shares), ledger_sequence) AS total_shares \
             FROM liquidity_pool_snapshots \
             WHERE pool_id IN (SELECT pool_id FROM page) \
               AND ledger_sequence BETWEEN (SELECT lo FROM band) AND (SELECT hi FROM band) \
             GROUP BY pool_id \
         ) s ON s.pool_id = lp.pool_id \
         LEFT JOIN ( \
             SELECT pool_id, toNullable(min(ledger_sequence)) AS created_at_ledger \
             FROM liquidity_pool_snapshots \
             WHERE pool_id IN (SELECT pool_id FROM page) \
             GROUP BY pool_id \
         ) cr ON cr.pool_id = lp.pool_id \
         LEFT JOIN ( \
             SELECT pool_id, count() AS participant_count FROM lp_positions FINAL \
             WHERE shares > 0 AND pool_id IN (SELECT pool_id FROM page) \
             GROUP BY pool_id \
         ) pc ON pc.pool_id = lp.pool_id \
         LEFT JOIN sac sac_a ON sac_a.asset_code = lp.asset_a_code \
                            AND sac_a.issuer_id = lp.asset_a_issuer_id \
         LEFT JOIN sac sac_b ON sac_b.asset_code = lp.asset_b_code \
                            AND sac_b.issuer_id = lp.asset_b_issuer_id \
         /* `GROUP BY sequence` dedups `ledgers` (ReplacingMergeTree, unmerged \
            duplicate rows): without it this LEFT JOIN doubled every page row \
            whose latest snapshot ledger falls in the duplicated range, doubling \
            UI rows and breaking keyset pagination. `any(closed_at)` is exact \
            (measured: closed_at identical across every dup pair). lore-0420 \
            \
            The filter is `page.last_updated_ledger` while the join key is \
            `s.latest_ledger_sequence` — these look mismatched but are the same \
            set, because a pool's last update always writes a snapshot at that \
            ledger: measured 52,284 of 52,284 pools with \
            `last_updated_ledger = max(ledger_sequence)`. If that invariant ever \
            breaks the join simply misses and `latest_snapshot_at_ms` is null — \
            degraded, never wrong. */ \
         LEFT JOIN ( \
             SELECT sequence, any(closed_at) AS closed_at FROM ledgers \
             WHERE sequence IN (SELECT last_updated_ledger FROM page) \
             GROUP BY sequence \
         ) l_snap ON l_snap.sequence = s.latest_ledger_sequence \
         ORDER BY lp.last_updated_ledger {order}, lp.pool_id {order}",
        filters = filters,
        keyset = keyset,
        order = order,
        limit = params.limit,
    );

    let mut query = client.query(&sql);
    for b in &binds {
        query = query.bind(b.as_str());
    }
    let rows = query.fetch_all::<PoolListChRow>().await?;

    // Resolve issuer StrKeys by surrogate id (bloom seek). The old in-query `iss`
    // CTE used `WHERE id IN (SELECT … FROM page)` — the subquery form does not
    // trigger the `idx_acc_id` bloom, so it scanned `accounts.id` (task 0345).
    let issuer_ids = rows
        .iter()
        .flat_map(|r| [r.asset_a_issuer_id, r.asset_b_issuer_id])
        // Exclude the native sentinel `0` — the old `iss` CTE filtered
        // `WHERE … != 0`. A no-op on real data (`accounts.id = cityhash64(strkey)`
        // is never 0), but keeps the resolution unconditionally identical.
        .filter(|&id| id != 0)
        .collect();
    let accounts = resolve_accounts(client, issuer_ids).await?;

    // Phase A2 (issue #367): per-row USD TVL, computed like the detail
    // endpoint (latest reserves × last 1h close per leg; both legs required)
    // from ONE batched price lookup over the page's distinct identities.
    // `volume`/`fee_revenue` stay NULL on the list — detail-only semantics.
    // A prices error degrades every row to NULL TVL (error-logged), it does
    // not fail the list: same resilience contract as the detail endpoint.
    let page_legs: Vec<(PriceLeg, PriceLeg)> = rows
        .iter()
        .map(|r| {
            let issuer_a = accounts.get(&r.asset_a_issuer_id).map(String::as_str);
            let issuer_b = accounts.get(&r.asset_b_issuer_id).map(String::as_str);
            (
                price_leg(r.asset_a_type, r.asset_a_code.as_deref(), issuer_a),
                price_leg(r.asset_b_type, r.asset_b_code.as_deref(), issuer_b),
            )
        })
        .collect();
    let mut unique_legs: Vec<&PriceLeg> = page_legs
        .iter()
        .flat_map(|(a, b)| [a, b])
        .filter(|l| !l.kind.is_empty())
        .collect();
    unique_legs
        .sort_unstable_by(|a, b| (a.kind, &a.code, &a.issuer).cmp(&(b.kind, &b.code, &b.issuer)));
    unique_legs.dedup();
    let closes = match fetch_last_closes(client, &unique_legs).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("DB error in fetch_last_closes (list TVL degraded to NULL): {e}");
            std::collections::HashMap::new()
        }
    };

    Ok(rows
        .into_iter()
        .zip(page_legs)
        .map(|(r, (leg_a, leg_b))| {
            let tvl = match (
                r.reserve_a.as_deref().and_then(parse_f64),
                r.reserve_b.as_deref().and_then(parse_f64),
                closes.get(&leg_a).copied(),
                closes.get(&leg_b).copied(),
            ) {
                (Some(ra), Some(rb), Some(pa), Some(pb)) => Some(usd_str(ra * pa + rb * pb)),
                _ => None,
            };
            PoolRow {
                pool_id_hex: r.pool_id_hex,
                asset_a_type: r.asset_a_type,
                asset_a_type_name: asset_type_name(r.asset_a_type),
                asset_a_code: r.asset_a_code,
                asset_a_issuer: accounts
                    .get(&r.asset_a_issuer_id)
                    .cloned()
                    .filter(|s| !s.is_empty()),
                asset_a_contract_id: r.asset_a_contract_id,
                asset_a_icon_url: r.asset_a_icon_url,
                asset_b_type: r.asset_b_type,
                asset_b_type_name: asset_type_name(r.asset_b_type),
                asset_b_code: r.asset_b_code,
                asset_b_issuer: accounts
                    .get(&r.asset_b_issuer_id)
                    .cloned()
                    .filter(|s| !s.is_empty()),
                asset_b_contract_id: r.asset_b_contract_id,
                asset_b_icon_url: r.asset_b_icon_url,
                fee_bps: r.fee_bps,
                fee_percent: fee_percent_str(r.fee_bps),
                created_at_ledger: r.created_at_ledger,
                cursor_ledger: r.cursor_ledger,
                participant_count: r.participant_count,
                latest_snapshot_ledger: r.latest_snapshot_ledger,
                reserve_a: r.reserve_a,
                reserve_b: r.reserve_b,
                total_shares: r.total_shares,
                tvl,
                volume: None,
                fee_revenue: None,
                latest_snapshot_at: r.latest_snapshot_at_ms.map(millis_to_utc),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_pool_id_validation() {
        assert!(is_hex_pool_id(&"a".repeat(64)));
        assert!(is_hex_pool_id(&"0123456789abcdef".repeat(4)));
        assert!(!is_hex_pool_id(&"a".repeat(63)));
        assert!(!is_hex_pool_id(&"a".repeat(65)));
        assert!(!is_hex_pool_id(&"A".repeat(64)), "uppercase rejected");
        assert!(!is_hex_pool_id("xyz"));
        assert!(!is_hex_pool_id(&"'; DROP--".repeat(8)));
    }

    /// The pool-leg surrogates this module computes from `liquidity_pools`
    /// columns MUST equal the ones the indexer writes into
    /// `lp_operation_amounts.asset_id` from a claim atom's asset string
    /// (`stage.rs::claim_atom_asset_id` → `ids::credit_asset_id` /
    /// `NATIVE_ASSET_ID`). They meet only through this equality: if it breaks,
    /// no row ever matches a leg and the Amount column silently goes blank
    /// instead of failing. The bridge is `asset_a_issuer_id`, which the writer
    /// fills with `ids::account_id(issuer_strkey)`.
    ///
    /// Every XDR asset type a pool leg can hold is covered here on purpose.
    /// The first version of this test used `"TF"` — `credit_alphanum4`, XDR
    /// type 1 — and so agreed with the buggy resolution: type 2 is
    /// `credit_alphanum12` in `liquidity_pools`, but the retired SAC facet in
    /// `ids::asset_id`, which answered `0` for it. 59% of pools carry a type-2
    /// leg and the suite stayed green (task 0489). A code of each width is now
    /// pinned, so the next type-space mix-up fails here.
    #[test]
    fn pool_leg_surrogates_match_the_written_asset_ids() {
        const ISSUER: &str = "GB5WIXCUO5DWAJSVLVIJH5SBWGIRKGD27YYHLPOISGBO7MW2UH3EJXLM";
        let issuer_id = ids::account_id(ISSUER);
        // Native leg: type 0, empty code, issuer_id 0.
        assert_eq!(ids::pool_leg_asset_id(0, "", 0), ids::NATIVE_ASSET_ID);
        // credit_alphanum4 (XDR type 1) and credit_alphanum12 (XDR type 2) are
        // both classic credit, so both must land on the surrogate the writer
        // computes from the StrKey the claim atom carries.
        for (asset_type, code) in [(1i16, "TF"), (2i16, "CETES")] {
            assert_eq!(
                ids::pool_leg_asset_id(asset_type, code, issuer_id),
                ids::credit_asset_id(code, ISSUER),
                "leg {code} (XDR type {asset_type}) must match the written asset_id",
            );
        }
        // The bug this replaced: a type-2 leg resolved to 0, and 0 is an id no
        // row is ever stored under, so the leg could never match.
        assert_ne!(ids::pool_leg_asset_id(2, "CETES", issuer_id), 0);
    }

    /// The same equality against REAL production values, so the pin does not
    /// rest on this module's own arithmetic being self-consistent.
    ///
    /// Pool `8CA53441…` (yXLM / CETES) is the one that exposed task 0489: a
    /// `credit_alphanum4` leg beside a `credit_alphanum12` one, so the page
    /// rendered the first and dropped the second. Left column read from
    /// `liquidity_pools`, right column the `DISTINCT asset_id` that
    /// `lp_operation_amounts` actually holds for that pool — both captured
    /// from prod on 2026-08-17. Static values, no network.
    #[test]
    fn pool_leg_surrogates_match_production_rows() {
        // (asset_type, code, issuer_id) -> the asset_id stored on prod
        for (asset_type, code, issuer_id, stored) in [
            (
                1i16,
                "yXLM",
                -5_950_609_493_839_131_376i64,
                258_332_573_254_456_524i64,
            ),
            (
                2i16,
                "CETES",
                1_238_723_897_090_515_379i64,
                4_032_595_941_348_833_451i64,
            ),
        ] {
            assert_eq!(
                ids::pool_leg_asset_id(asset_type, code, issuer_id),
                stored,
                "leg {code} must resolve to the asset_id production stores",
            );
        }
    }

    /// The SAC joins on both pool reads must not filter a leg out for having an
    /// empty `asset_code` (task 0470).
    ///
    /// An empty code is native XLM's real, stored identity — not a missing
    /// value — and native has a deployed SAC. An `asset_code != ''` guard was
    /// added deliberately in `a19ac8f6` to match Postgres, which returned NULL
    /// there; Postgres is retired and `/v1/assets/native` publishes that same
    /// SAC, so the guard left one asset describing itself two ways depending on
    /// the endpoint.
    ///
    /// Pinned on the module source because both queries are inline string
    /// literals — there is no builder to call. That is the honest limit of this
    /// guard: it catches the exact regression (a re-added `!= ''` on a leg
    /// code) and nothing subtler. A behavioural test needs the queries
    /// extracted first, which is recorded as an acceptance criterion on 0470.
    #[test]
    fn no_leg_code_guard_can_exclude_the_native_leg_from_its_sac() {
        // Only the production half — the test module below quotes the guard it
        // is looking for, and would match itself.
        let src = include_str!("queries.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        // Count only the leg-code guards; other `!= ''` comparisons in this
        // module are about different columns and are none of this test's
        // business.
        let guards = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| l.contains("asset_a_code != ''") || l.contains("asset_b_code != ''"))
            .count();
        assert_eq!(
            guards, 0,
            "a leg-code guard is back: it silently drops native XLM's SAC, \
             which /v1/assets/native still reports"
        );
    }

    #[test]
    fn fee_percent_formats() {
        assert_eq!(fee_percent_str(30), "0.3");
        assert_eq!(fee_percent_str(25), "0.25");
        assert_eq!(fee_percent_str(100), "1");
        assert_eq!(fee_percent_str(0), "0");
        assert_eq!(fee_percent_str(5), "0.05");
    }

    #[test]
    fn decimal_str_validation() {
        assert!(is_decimal_str("0"));
        assert!(is_decimal_str("123.4567890"));
        assert!(is_decimal_str("-5.5"));
        assert!(!is_decimal_str(""));
        assert!(!is_decimal_str("1.2.3"));
        assert!(!is_decimal_str("1e9"));
        assert!(!is_decimal_str("'; DROP"));
        assert!(!is_decimal_str("abc"));
    }

    #[test]
    fn asset_type_names() {
        assert_eq!(asset_type_name(0).as_deref(), Some("native"));
        assert_eq!(asset_type_name(1).as_deref(), Some("credit_alphanum4"));
        assert_eq!(asset_type_name(2).as_deref(), Some("credit_alphanum12"));
        assert_eq!(asset_type_name(3).as_deref(), Some("pool_share"));
        assert_eq!(asset_type_name(9), None);
    }

    /// The prices JOIN key contract (views.sql, pinned 2026-06-16):
    /// native = ('native','XLM',''), classic = ('credit', code, issuer).
    /// A wrong mapping here silently prices legs off the wrong row — the
    /// exact failure mode the raw-`prices.assets` join produced (task 0199
    /// activation note, bogus 96.4% coverage).
    #[test]
    fn price_leg_mapping() {
        let native = price_leg(0, None, None);
        assert_eq!(
            (native.kind, native.code.as_str(), native.issuer.as_str()),
            ("native", "XLM", "")
        );
        // Native ignores whatever code/issuer the row carries ('' / surrogate-0 artifacts).
        let native2 = price_leg(0, Some(""), Some(""));
        assert_eq!(native2.kind, "native");

        let usdc = price_leg(
            1,
            Some("USDC"),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"),
        );
        assert_eq!(usdc.kind, "credit");
        assert_eq!(usdc.code, "USDC");
        assert!(usdc.issuer.starts_with('G'));

        let alphanum12 = price_leg(2, Some("WGUARDIAN"), Some("GABC"));
        assert_eq!(alphanum12.kind, "credit");

        // Unpriceable degradations: missing identity parts or unexpected type
        // must match NO prices row (empty kind), never guess.
        assert_eq!(price_leg(1, None, Some("GABC")).kind, "");
        assert_eq!(price_leg(1, Some("USDC"), None).kind, "");
        assert_eq!(price_leg(1, Some(""), Some("GABC")).kind, "");
        assert_eq!(price_leg(3, Some("X"), Some("G")).kind, "");
        assert_eq!(price_leg(9, None, None).kind, "");
    }

    #[test]
    fn usd_helpers() {
        assert_eq!(parse_f64("123.4567890"), Some(123.456789));
        assert_eq!(parse_f64("0"), Some(0.0));
        assert_eq!(parse_f64(""), None);
        assert_eq!(parse_f64("abc"), None);
        assert_eq!(parse_f64("inf"), None, "non-finite rejected");
        assert_eq!(usd_str(1234.5678), "1234.57");
        assert_eq!(usd_str(0.0), "0.00");
        // Sub-cent values must not collapse to "0.00" — a client cannot
        // tell that apart from a genuine zero (fee_revenue lives here).
        assert_eq!(usd_str(0.003), "0.0030");
        assert_eq!(usd_str(0.00009), "0.000090");
        assert_eq!(usd_str(-0.003), "-0.0030");
        // At or above a cent the plain money form still applies.
        assert_eq!(usd_str(0.01), "0.01");
        assert_eq!(usd_str(0.5), "0.50");
        // Fixed 2 decimals on every path — CH's toString(round(x, 2)) would
        // emit "25" / "1.5" / "0" here and split the wire shape between the
        // chart and the detail endpoint.
        assert_eq!(usd_str(25.0), "25.00");
        assert_eq!(usd_str(1.5), "1.50");
    }

    /// `fee_bps` is basis points: 30 bps = 0.30%, so the divisor is 10 000.
    /// A /100 or /1000 slip inflates reported LP earnings 100× / 10×.
    #[test]
    fn fee_revenue_math() {
        assert_eq!(fee_revenue_usd(1_000_000.0, 30), 3_000.0);
        assert_eq!(fee_revenue_usd(1_000.0, 100), 10.0);
        assert_eq!(fee_revenue_usd(0.0, 30), 0.0);
        assert_eq!(fee_revenue_usd(500.0, 0), 0.0);
    }
}

/// Live-CH **decode** smoke for the LP read path.
///
/// The curl `FORMAT TSV/Vertical/JSON` box smokes do NOT exercise the
/// clickhouse-rs RowBinary decoder, so a wire-type↔struct mismatch — e.g. a
/// scalar `(SELECT count() …)` typed `Nullable(UInt64)` decoded into an `i64`
/// field (the detail `participant_count` bug, task 0243) — passes a curl check
/// yet 500s the live endpoint with `schema mismatch`. A pure-Rust round-trip
/// can't catch it either (the struct serializes consistently with itself). The
/// only real guard is decoding rows that an actual CH produced.
///
/// This test runs each cheap LP CH fetch fn against a real CH and asserts the
/// rows decode (no error). It **skips cleanly when `CH_URL` is unset**, so CI
/// (no CH access) is unaffected. Run it against a reachable CH — a local
/// replica or an SSH tunnel to the box:
///
/// ```text
/// CH_URL=http://127.0.0.1:8123 CH_DATABASE=default \
///   cargo test -p api --lib decode_smoke -- --nocapture
/// ```
///
/// `transactions` is intentionally excluded: its driver scans the whole
/// `operations_appearances` table (~7.87B rows) until the `pool_id` projection
/// lands, so exercising it here would blow the read quota. Its row struct is all
/// direct, non-null columns (audited — no Nullable-decode risk).
#[cfg(test)]
mod decode_smoke {
    use super::ResolvedPoolListParams;
    use super::*;
    use crate::common::cursor::Direction;

    fn client() -> Option<clickhouse::Client> {
        let url = std::env::var("CH_URL").ok()?;
        let mut c = clickhouse::Client::default().with_url(url);
        if let Ok(u) = std::env::var("CH_USER") {
            c = c.with_user(u);
        }
        if let Ok(p) = std::env::var("CH_PASSWORD") {
            c = c.with_password(p);
        }
        if let Ok(d) = std::env::var("CH_DATABASE") {
            c = c.with_database(d);
        }
        Some(c)
    }

    /// `ChartChRow` reads money as `Nullable(Float64)` (task 0199 moved
    /// formatting to Rust so chart and detail share one wire shape). That is
    /// precisely the wire-type↔struct contract a pure-Rust test cannot check,
    /// so assert it against a real server — including the NULL arm, which is
    /// what an unpriced bucket returns.
    ///
    /// Needs no schema, so any ClickHouse will do:
    /// `docker run -d --rm -p 8123:8123 -e CLICKHOUSE_PASSWORD=probe clickhouse/clickhouse-server:26.3`
    #[tokio::test]
    async fn chart_row_decodes_nullable_floats() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping chart row decode smoke");
            return;
        };

        // The union is wrapped: ClickHouse resolves a top-level ORDER BY
        // against the union's own scope, where the branch aliases are not
        // visible (`Unknown expression identifier`).
        let rows = ch
            .query(
                "SELECT bucket_ms, tvl, volume, samples_in_bucket FROM ( \
                     SELECT toInt64(1700000000000)     AS bucket_ms, \
                            CAST(?, 'Nullable(Float64)')    AS tvl, \
                            CAST(?, 'Nullable(Float64)')    AS volume, \
                            toUInt64(7)                AS samples_in_bucket \
                     UNION ALL \
                     SELECT toInt64(1700000086400000)  AS bucket_ms, \
                            CAST(NULL, 'Nullable(Float64)') AS tvl, \
                            CAST(NULL, 'Nullable(Float64)') AS volume, \
                            toUInt64(0)                AS samples_in_bucket \
                 ) ORDER BY bucket_ms",
            )
            .bind(25.31_f64)
            .bind(1.985_f64)
            .fetch_all::<ChartChRow>()
            .await
            .expect("ChartChRow decodes Nullable(Float64) from a real CH");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tvl, Some(25.31));
        assert_eq!(rows[0].volume, Some(1.985));
        assert_eq!(rows[0].samples_in_bucket, 7);
        // The unpriced bucket: NULL must survive as None, not decode as 0.0.
        assert_eq!(rows[1].tvl, None);
        assert_eq!(rows[1].volume, None);
    }

    /// Every LP CH row struct must decode the rows a real CH emits.
    #[tokio::test]
    async fn lp_ch_rows_decode() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping LP CH decode smoke");
            return;
        };

        // `list` returns rows on any populated CH → always exercises the
        // `PoolListChRow` decode, and bootstraps a guaranteed-real pool id for
        // the per-pool fetches below (an env-default pool might not exist on the
        // target CH → detail would return None and skip the decode entirely).
        let params = ResolvedPoolListParams {
            limit: 5,
            cursor: None,
            asset_a_code: None,
            asset_a_issuer: None,
            asset_b_code: None,
            asset_b_issuer: None,
            pool_id_hex: None,
            asset_codes: Vec::new(),
        };
        let pools = fetch_pool_list(&ch, &params, Direction::Next)
            .await
            .expect("list rows decode");

        let pool = match std::env::var("CH_TEST_POOL_HEX") {
            Ok(h) => h,
            Err(_) => match pools.first() {
                Some(r) => r.pool_id_hex.clone(),
                None => {
                    eprintln!("CH has no liquidity pools — skipping per-pool decode");
                    return;
                }
            },
        };

        // detail — `PoolDetailChRow`, incl. the Nullable-scalar `participant_count`.
        fetch_pool_by_id(&ch, &pool)
            .await
            .expect("detail row decodes");

        // participants — `ParticipantChRow`.
        fetch_participants(&ch, &pool, None, 5, Direction::Next)
            .await
            .expect("participant rows decode");

        // price context — `PriceContextChRow` (chart's 404 gate).
        let ctx = fetch_pool_price_context(&ch, &pool)
            .await
            .expect("price-context row decodes")
            .expect("bootstrapped pool exists");

        // The remaining two read `prices.*`, which the explorer does not own
        // and `schema/init.sql` does not create — a CH bootstrapped from this
        // repo alone has no such database. Probe once and skip rather than
        // fail, so the documented local-replica run still validates every
        // explorer-owned decode above. Against prod (or any CH with the
        // prices tenant) the probe passes and both are exercised — which also
        // proves the API user can read that database. No grant is needed
        // there: `api_reader` carries no `<grants>` block in
        // `users.d/services.xml` (unlike `prices_writer`/`prices_reader`,
        // where grants NARROW access), verified on the box 2026-08-04.
        if ch
            .query("SELECT 1 FROM prices.price_usd_series_1h LIMIT 1")
            .fetch_all::<u8>()
            .await
            .is_err()
        {
            eprintln!("`prices` database unreachable — skipping USD-analytics + chart decode");
            return;
        }

        // detail USD analytics — `Vol24ChRow` + `LastCloseChRow`.
        fetch_pool_usd_analytics(&ch, &pool, &ctx, None, None)
            .await
            .expect("usd-analytics rows decode");

        // chart — `ChartChRow`, incl. the `samples_in_bucket` UInt64.
        let to = chrono::Utc::now();
        let from = to - chrono::Duration::days(90);
        fetch_pool_chart(&ch, &pool, &ctx, "1d", from, to)
            .await
            .expect("chart rows decode");
    }

    /// `filter[asset_code]` is a substring of either leg, not an exact code
    /// (0440 / issue #366). The regression this guards is the original
    /// behaviour: `USD` returning nothing while the list is full of `USDC`
    /// pools. Asserting the returned legs actually contain the needle also
    /// catches the opposite failure — a predicate that stopped filtering.
    #[tokio::test]
    async fn asset_code_filter_matches_substring() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping LP asset-code substring smoke");
            return;
        };

        let params = ResolvedPoolListParams {
            limit: 10,
            cursor: None,
            asset_a_code: None,
            asset_a_issuer: None,
            asset_b_code: None,
            asset_b_issuer: None,
            // Deliberately a proper prefix of a real code: an exact-match
            // predicate returns zero rows here, a substring one does not.
            pool_id_hex: None,
            asset_codes: vec!["USD".to_string()],
        };
        let pools = fetch_pool_list(&ch, &params, Direction::Next)
            .await
            .expect("filtered list decodes");

        assert!(
            !pools.is_empty(),
            "`USD` matched no pool — substring filter regressed to exact match"
        );
        for p in &pools {
            let a = p.asset_a_code.as_deref().unwrap_or_default().to_uppercase();
            let b = p.asset_b_code.as_deref().unwrap_or_default().to_uppercase();
            assert!(
                a.contains("USD") || b.contains("USD"),
                "pool {} has neither leg containing USD ({a:?} / {b:?}) — filter not applied",
                p.pool_id_hex
            );
        }
    }

    /// `XLM` must reach the pools that hold *native* XLM. Native legs carry an
    /// empty stored code, so a plain column match silently returns only the
    /// credit assets minted under the code `XLM` — a wrong answer that looks
    /// like a right one. Guards the `if(asset_type = 0, 'XLM', code)` alias.

    #[tokio::test]
    async fn asset_code_filter_finds_native_xlm() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping LP native-XLM smoke");
            return;
        };

        let params = ResolvedPoolListParams {
            limit: 25,
            cursor: None,
            asset_a_code: None,
            asset_a_issuer: None,
            asset_b_code: None,
            asset_b_issuer: None,
            pool_id_hex: None,
            asset_codes: vec!["XLM".to_string()],
        };
        let pools = fetch_pool_list(&ch, &params, Direction::Next)
            .await
            .expect("filtered list decodes");

        assert!(
            pools
                .iter()
                .any(|p| p.asset_a_type == 0 || p.asset_b_type == 0),
            "`XLM` returned {} pool(s) but none holds native XLM — the native \
             alias regressed and the filter is answering with look-alike \
             credit assets only",
            pools.len()
        );
    }

    /// A pair query constrains both legs and does not care which order the user
    /// typed, nor which leg the chain assigned. Runs the same pair twice,
    /// reversed, and requires identical results — the cheapest way to catch a
    /// predicate that quietly became order-sensitive.
    #[tokio::test]
    async fn asset_code_filter_pair_is_order_insensitive() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping LP pair-filter smoke");
            return;
        };

        let pair = |a: &str, b: &str| ResolvedPoolListParams {
            limit: 25,
            cursor: None,
            asset_a_code: None,
            asset_a_issuer: None,
            asset_b_code: None,
            asset_b_issuer: None,
            pool_id_hex: None,
            asset_codes: vec![a.to_string(), b.to_string()],
        };

        let ids = |rows: Vec<PoolRow>| {
            let mut v: Vec<String> = rows.into_iter().map(|r| r.pool_id_hex).collect();
            v.sort();
            v
        };

        let forward = ids(fetch_pool_list(&ch, &pair("XLM", "USDC"), Direction::Next)
            .await
            .expect("forward pair decodes"));
        let reversed = ids(fetch_pool_list(&ch, &pair("USDC", "XLM"), Direction::Next)
            .await
            .expect("reversed pair decodes"));

        assert_eq!(forward, reversed, "pair filter is order-sensitive");
        assert!(
            !forward.is_empty(),
            "`XLM/USDC` matched no pool — the AND-ed needles are over-constraining"
        );

        // Both needles must bind: a pair that shares only one leg with any pool
        // has to come back empty, otherwise the second needle is being dropped.
        let impossible = fetch_pool_list(&ch, &pair("USDC", "ZZZZNOPE"), Direction::Next)
            .await
            .expect("impossible pair decodes");
        assert!(
            impossible.is_empty(),
            "pair with an unmatchable second needle returned {} pool(s) — the \
             needles are OR-ed, not AND-ed",
            impossible.len()
        );

        // Three codes. `normalize_asset_codes` splits `USDC/XLM/BTC` into
        // `USDC` and the literal `XLM/BTC` (see its unit tests); a pool has two
        // legs, so no asset code can carry that second needle and the answer is
        // empty. Asserted here so the query side cannot start "helpfully"
        // ignoring the remainder.
        let three = fetch_pool_list(&ch, &pair("USDC", "XLM/BTC"), Direction::Next)
            .await
            .expect("three-code query decodes");
        assert!(
            three.is_empty(),
            "a three-code query returned {} pool(s) — the third code is being \
             dropped instead of narrowing to nothing",
            three.len()
        );

        // Each needle claims its own leg. Repeating one therefore means "both
        // legs", not "matches somewhere, twice" — a pool with USDC on one side
        // and anything else on the other must not come back.
        let both_legs = fetch_pool_list(&ch, &pair("USDC", "USDC"), Direction::Next)
            .await
            .expect("repeated needle decodes");
        for p in &both_legs {
            let a = p.asset_a_code.as_deref().unwrap_or_default().to_uppercase();
            let b = p.asset_b_code.as_deref().unwrap_or_default().to_uppercase();
            assert!(
                a.contains("USDC") && b.contains("USDC"),
                "pool {} came back for `USDC/USDC` with legs {a:?} / {b:?} — one \
                 asset is satisfying both needles",
                p.pool_id_hex
            );
        }
    }
}
