//! ClickHouse queries for the assets endpoints (task 0243).
//!
//! Returns the `AssetRow` shape, so
//! the handler stays backend-agnostic after the fetch. Assets are addressed by
//! the composite identity (contract StrKey / `CODE-ISSUER` / the reserved
//! `native` token); the numeric surrogate was dropped (PR #175), and CH keys
//! `assets` on `(asset_type, asset_code, issuer_id, contract_id)` with no
//! surrogate at all.
//!
//! Notable CH-vs-PG divergences:
//!
//! - **`asset_family_name()` is a PG SQL function.** CH has no equivalent,
//!   so the `asset_type` → label mapping is done in Rust ([`asset_type_name`]),
//!   identical to the PG migration `20260422000000_enum_label_functions`.
//! - **`assets a FINAL`** collapses re-ingested asset versions (ReplacingMergeTree
//!   keyed on the 4-tuple). The `soroban_contracts` lookup join carries no FINAL
//!   — the identity columns it projects (contract StrKey, `deployed_at_ledger`)
//!   are write-once / stable across versions. The `accounts` table is NOT joined
//!   (task 0319 for the list, task 0334 for the detail): a full `accounts` hash
//!   side (~18M rows) was the dominant read cost. The issuer is resolved by a
//!   separate `accounts.id` key-seek instead. `home_domain` IS mutable
//!   (SET_OPTIONS), so the seek picks the latest version
//!   (`ORDER BY last_seen_ledger DESC LIMIT 1`); a 16M-row `accounts FINAL`
//!   would be ruinous.
//! - **`nullIf(...)` on the joined columns** (contract_id → `nullIf(_, '')`,
//!   deployed_at_ledger → `nullIf(_, 0)`) makes a LEFT-JOIN miss decode as
//!   `None`. We do NOT use `SETTINGS join_use_nulls = 1` — the `api_reader` CH
//!   user runs under `readonly = 1` (RBAC profile `read_only`), which rejects
//!   per-query setting overrides. Same convention as stkrolikiewicz's CH modules
//!   (accounts `fetch_balances`, LP). The issuer StrKey / home_domain are emptied
//!   to `None` in Rust ([`list_row_to_asset_row`]), not via `nullIf`.
//! - **Cursor** is the composite `AssetKeyCursor`; its keyset clause is only
//!   present on continuation pages, so the clickhouse 0.15 "None bound into a
//!   tuple keyset returns 0 rows" defect is sidestepped (the bound values are
//!   always real). The free-text `asset_code` filter and the cursor values are
//!   `.bind()`-ed (user-controlled); `asset_type` is interpolated (typed `i16`).
//!
//! `/transactions` (canonical 10) keys on the datasource-tagged `TxListCursor`
//! (`Ch { ledger_sequence, tiebreak }`), mirroring the accounts sub-resource. It
//! seeks the `operation_asset_appearances` fan-out on its `asset_id`-leading PK
//! (task 0359) — a bounded PK-prefix range read behind the `max(sequence)` commit
//! fence, so a hot asset early-terminates near the tip and a rare asset stays a
//! bounded seek, replacing the old NON-leading identity-predicate density-scan
//! over `operations_appearances` (ORDER BY `ledger_sequence`) that walked back by
//! descending ledger until the page filled.

use std::collections::{BTreeSet, HashMap};

use clickhouse::Row;
use serde::Deserialize;

use chrono::{DateTime, Utc};

use crate::common::ch::{self, millis_to_utc, resolve_accounts};
use crate::common::cursor::{Direction, SortOrder, keyset_sql, keyset_sql_desc};
use crate::transactions::dto::TxListCursor;

use super::dto::AssetKeyCursor;

// ---------------------------------------------------------------------------
// Internal query-result rows + helpers (not serialized; the handler maps these
// into the public response DTOs).
// ---------------------------------------------------------------------------

/// Detail/list row for an asset. Handler maps this to the wire `AssetItem`.
#[derive(Debug, Clone)]
pub struct AssetRow {
    pub asset_type: i16,
    /// Pre-decoded via `asset_family_name()` SQL helper. `None` only
    /// when the discriminant is outside the schema CHECK range — defensive
    /// against future schema drift.
    pub asset_type_name: Option<String>,
    pub asset_code: Option<String>,
    /// Already resolved through `accounts.account_id` join.
    pub issuer: Option<String>,
    /// Already resolved through `soroban_contracts.contract_id` join.
    pub contract_id: Option<String>,
    pub name: Option<String>,
    /// On-chain SEP-41 token symbol from `soroban_contract_metadata` (task 0297);
    /// `None` for classic/native.
    pub symbol: Option<String>,
    /// Display decimals — on-chain `METADATA` for Soroban tokens, else 7
    /// (Stellar classic precision).
    pub decimals: u32,
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
    /// `soroban_contracts.deployed_at_ledger` — populated for SAC and
    /// Soroban-native rows; `None` for native and classic_credit.
    pub deployed_at_ledger: Option<i64>,
    /// `accounts.home_domain` for the issuer, used as the SEP-1 lookup
    /// key in `get_asset` runtime enrichment (task 0188). `None` for
    /// native, no-issuer, and issuer accounts that did not set
    /// `home_domain` on-chain.
    pub issuer_home_domain: Option<String>,
    /// Surrogate key columns — cursor keyset only, never on the wire. These
    /// are the 4-tuple CH orders `assets` by `(asset_type, asset_code,
    /// issuer_id, contract_id)`; `0` / `''` stand in for "absent" (native has
    /// no issuer_id, classic-credit has no contract_id), matching CH defaults.
    pub issuer_id: i64,
    pub contract_surrogate_id: i64,
    /// SAC facet (ADR 0051): the surrogate of the wrapping SAC's `C…` StrKey,
    /// or `0` when the asset has no observed SAC. Never on the wire — the
    /// handler re-derives the display StrKey from `code:issuer` when non-zero.
    pub sac_contract_surrogate: i64,
    /// Whether the `sac_contract_surrogate` SAC is deployed on-chain (ADR 0051).
    pub sac_deployed: bool,
    /// `assets.id` — the `ids::asset_id` surrogate (task 0331). The single asset
    /// key for the `operation_asset_appearances` fan-out seek (task 0359).
    pub id: i64,
}

#[derive(Debug)]
pub struct AssetTxRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
}

/// Resolved, validated `GET /v1/assets` list params handed to `fetch_list`.
pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<AssetKeyCursor>,
    pub asset_type: Option<i16>,
    /// Raw substring (no `%` / `_` from the caller). The SQL builder
    /// wraps it in `%...%` for the trigram match.
    pub asset_code: Option<String>,
    /// SAC property filter (ADR 0051): restrict to assets with a SAC
    /// (`sac_contract_id != 0`) — the old `filter[type]=sac` view.
    pub sac_only: bool,
}

/// `asset_type` SMALLINT → canonical label, matching the PG
/// `asset_family_name` function. `None` for an out-of-range code (the PG
/// `CASE` returns NULL with no `ELSE`).
fn asset_type_name(asset_type: i16) -> Option<String> {
    match asset_type {
        0 => Some("native"),
        1 => Some("classic_credit"),
        // 2 (`sac`) retired — ADR 0051; a SAC is a facet of classic_credit / native.
        3 => Some("soroban"),
        _ => None,
    }
    .map(str::to_string)
}

// Projection / enrichment semantics shared by the list + detail reads (task
// 0334 collapsed the detail paths onto the same accounts-join-free SELECT).
// Column order MUST match the row struct (positional decode); the `nullIf(...)`
// wraps make a LEFT-JOIN miss decode as `None` (readonly-safe; no
// `SETTINGS join_use_nulls`).
// Enrichment (icon_url + classic/SAC name) is read from the `asset_enrichment`
// side table (ADR 0050 / task 0231), NOT the indexer-owned `assets.{icon_url,
// name}` placeholders (dropped, task 0231 step 8). Per Option C the name has a
// single owner per `asset_type`, composed disjointly at read:
//   classic/SAC (1,2) → `asset_enrichment.name`
//   soroban (3)       → `soroban_contract_metadata.name` (on-chain instance
//                       `METADATA` struct; the legacy `soroban_contracts.name`
//                       column is dead — no writer since task 0297)
//   native (0)        → the `"Stellar Lumens"` literal
// `asset_enrichment` is `ReplacingMergeTree(version)`; the `argMax(_, version)`
// sub-aggregate collapses it to one latest row per key so the LEFT JOIN can't
// multiply asset rows on un-merged duplicates. `''` is the sentinel
// ("tried, nothing"), neutralised with `nullIf`.
// `total_supply` / `holder_count` come from `balance_aggregates` (task 0331,
// Option A) — the single pre-computed aggregate over the unified `balances`
// table, keyed by the `assets.id` surrogate (`bagg.asset_id = a.id`), one 1:1
// LEFT JOIN for ALL asset types. `total_supply` = Σ per-holder `amount` over G+C
// holders: a mint always credits a holder balance (often a contract treasury,
// summed because we sum contracts too), so the sum equals the token's real
// supply; the narrow residue is the accepted non-100% cost of one universal
// method — no per-token `TotalSupply` key read (see the task 0331 Option-A
// decision). The residue is everything the chain holds WITHOUT a holder, since
// this sum is keyed by one: native LP reserves + claimable balances (task 0210,
// measured 0.001% on USDT0 but unbounded for an AMM-heavy asset), plus the
// TTL-archived tail and true rebasing. Soroban-DEX reserves are contract-held,
// so ADR 0051 already counts them. RAW `Int128` (the API returns it raw;
// clients scale by `decimals`, classic = 7). `Nullable` columns, so a JOIN miss
// (no holders — reads NULL under the readonly `api_reader`, where
// `join_use_nulls = 0` defaults a Nullable to NULL) renders "—", not a fake 0.
// Refreshed by the MV on a cadence (eventually consistent). Requires `assets.id`
// populated (prod: ALTER + backfill — see init.sql).
// NOTE: post-ADR-0051 (task 0339, merged) a SAC is a FACET of its classic/native
// asset, not a separate `assets` row (type-2 retired). Contract-held SAC balances are
// re-keyed onto the WRAPPED classic id in `build_balance_rows`, so they sum INTO that
// classic asset's `balance_aggregates` row — ONE unified supply per asset. (Replaces
// the retired `asset_aggregates`, which keyed `(asset_code, issuer_id)`.)

/// Row decoded from [`hydrate_sql`] — the `assets`-side header (identity,
/// enrichment name/icon, balances, SAC facet). The `soroban_contracts`-derived
/// display fields (`contract_id` StrKey, `name` fallback, `symbol`, `decimals`,
/// `deployed_at_ledger`) are filled in Rust from [`resolve_soroban_contracts`],
/// then assembled into [`AssetListChRow`] by [`assemble_asset_row`] (task 0364
/// 2c — moved out of SQL so `soroban_contracts` is read once, not twice).
#[derive(Debug, Row, Deserialize)]
struct AssetHydrateRow {
    asset_type: i16,
    asset_code: Option<String>,
    /// `asset_enrichment.name` only — the `soroban_contract_metadata` fallback +
    /// the native default are applied during assembly.
    name_enrichment: Option<String>,
    total_supply: Option<String>,
    holder_count: Option<i32>,
    icon_url: Option<String>,
    issuer_id_key: i64,
    contract_id_key: i64,
    sac_contract_surrogate: i64,
    sac_deployed: bool,
    id: i64,
}

/// The asset header WITHOUT the join-resolved `issuer` / `issuer_home_domain`,
/// which are key-seeked separately (per-page for the list, task 0319;
/// per-request for the detail, task 0334). Assembled from an [`AssetHydrateRow`]
/// plus the [`SorobanCtxRow`] map by [`assemble_asset_row`].
#[derive(Debug)]
struct AssetListChRow {
    asset_type: i16,
    asset_code: Option<String>,
    contract_id: Option<String>,
    name: Option<String>,
    symbol: Option<String>,
    decimals: u32,
    total_supply: Option<String>,
    holder_count: Option<i32>,
    deployed_at_ledger: Option<i64>,
    icon_url: Option<String>,
    issuer_id_key: i64,
    contract_id_key: i64,
    sac_contract_surrogate: i64,
    sac_deployed: bool,
    id: i64,
}

/// Issuer resolve row: `accounts` → `id` (surrogate) + `account_id` StrKey +
/// `home_domain`. Decoded from [`seek_latest_account`], reached two ways: by `id`
/// (rides the `idx_acc_id` bloom skip-index — list per-page seek, task 0319, and
/// the detail `resolve_issuer`, task 0334) or by `account_id` (the table's
/// `ORDER BY account_id` primary key — the CODE-ISSUER detail form, task 0334).
#[derive(Debug, Row, Deserialize)]
struct AssetIssuerRow {
    id: i64,
    account_id: String,
    home_domain: Option<String>,
}

/// Build an [`AssetRow`] from an [`AssetListChRow`] projection row plus the
/// separately key-seeked issuer (`(StrKey, home_domain)`), consumed by value (no
/// clone). `iss` is `None` for native / no-issuer assets or a seek miss. Shared
/// by the list page map and the three detail fetches (task 0334).
fn list_row_to_asset_row(r: AssetListChRow, iss: Option<(String, Option<String>)>) -> AssetRow {
    let (issuer, issuer_home_domain) = match iss {
        Some((account_id, home_domain)) => (
            Some(account_id).filter(|s| !s.is_empty()),
            home_domain.filter(|s| !s.is_empty()),
        ),
        None => (None, None),
    };
    AssetRow {
        asset_type: r.asset_type,
        asset_type_name: asset_type_name(r.asset_type),
        asset_code: r.asset_code,
        issuer,
        contract_id: r.contract_id,
        name: r.name,
        symbol: r.symbol,
        decimals: r.decimals,
        total_supply: r.total_supply,
        holder_count: r.holder_count,
        icon_url: r.icon_url,
        deployed_at_ledger: r.deployed_at_ledger,
        issuer_home_domain,
        issuer_id: r.issuer_id_key,
        contract_surrogate_id: r.contract_id_key,
        sac_contract_surrogate: r.sac_contract_surrogate,
        sac_deployed: r.sac_deployed,
        id: r.id,
    }
}

/// Single-row `accounts` seek → ([`AssetIssuerRow`]: `id`, `account_id` StrKey,
/// `home_domain`), latest version. `where_col` MUST be a trusted static column
/// name — `"id"` (rides the `idx_acc_id` bloom skip-index) or `"account_id"`
/// (the `ORDER BY account_id` primary key) — NEVER user input (it is
/// interpolated, the value is bound). `home_domain` is mutable (SET_OPTIONS), so
/// pick the latest version (`ORDER BY last_seen_ledger DESC LIMIT 1`, no FINAL).
async fn seek_latest_account(
    client: &clickhouse::Client,
    where_col: &str,
    value: impl serde::Serialize,
) -> Result<Option<AssetIssuerRow>, clickhouse::error::Error> {
    let sql = format!(
        "SELECT id AS id, account_id AS account_id, home_domain AS home_domain \
         FROM accounts WHERE {where_col} = ? \
         ORDER BY last_seen_ledger DESC LIMIT 1"
    );
    client
        .query(&sql)
        .bind(value)
        .fetch_optional::<AssetIssuerRow>()
        .await
}

/// Resolve one issuer surrogate `id` → `(StrKey, home_domain)` via the
/// `accounts.id` bloom-pruned key-seek (task 0334) — NOT a full `accounts` scan /
/// hash join. `id == 0` (native / no issuer) returns `None` without a query.
async fn resolve_issuer(
    client: &clickhouse::Client,
    issuer_id: i64,
) -> Result<Option<(String, Option<String>)>, clickhouse::error::Error> {
    if issuer_id == 0 {
        return Ok(None);
    }
    Ok(seek_latest_account(client, "id", issuer_id)
        .await?
        .map(|r| (r.account_id, r.home_domain)))
}

/// Finish a detail fetch whose asset row was read from [`AssetListChRow`]:
/// resolve the issuer by an `accounts.id` key-seek, then map to [`AssetRow`].
/// Shared by the contract-StrKey and native forms (the CODE-ISSUER form resolves
/// its issuer up front, so it maps directly). A `None` row ⇒ asset not found.
async fn finish_detail(
    client: &clickhouse::Client,
    row: Option<AssetListChRow>,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    let Some(row) = row else {
        return Ok(None);
    };
    let iss = resolve_issuer(client, row.issuer_id_key).await?;
    Ok(Some(list_row_to_asset_row(row, iss)))
}

// ---------------------------------------------------------------------------
// Phase-1 seek + phase-2 hydration (task 0364)
// ---------------------------------------------------------------------------

/// Identity key for one `assets` version row, read WITHOUT `FINAL` in the
/// phase-1 seek. The 4-tuple IS the `assets` primary key; `id` is its
/// deterministic `cityhash64` surrogate. All five columns are immutable across a
/// key's physical versions (task 0364 — the mutable columns are externalised to
/// `balance_aggregates` / `asset_enrichment`), so a consecutive-dedup in PK
/// order is deterministic with no version tiebreak. `asset_code` is the RAW
/// value (empty for native / type-3), not the `nullIf`'d display form.
#[derive(Debug, Clone, Row, Deserialize)]
struct AssetKeyChRow {
    asset_type: i16,
    asset_code: String,
    issuer_id: i64,
    contract_id: i64,
    id: i64,
}

impl AssetKeyChRow {
    /// The identity 4-tuple — the dedup / ordering key.
    fn tuple(&self) -> (i16, &str, i64, i64) {
        (
            self.asset_type,
            &self.asset_code,
            self.issuer_id,
            self.contract_id,
        )
    }
}

/// Build the `(asset_type,'asset_code',issuer_id,contract_id)` IN-list for the
/// phase-2 hydration seek. Stellar asset codes are protocol-restricted to ≤12
/// alphanumeric ASCII chars (native / type-3 carry an empty code), and these
/// values are read back from our own `assets` table in phase 1 — never user
/// input — so the single-quoted `asset_code` literal has no injection surface
/// (debug-asserted). Numeric members are inlined `i64`.
fn asset_key_tuples(keys: &[AssetKeyChRow]) -> String {
    keys.iter()
        .map(|k| {
            debug_assert!(
                k.asset_code.chars().all(|c| c.is_ascii_alphanumeric()),
                "asset_code must be alphanumeric (from assets): {:?}",
                k.asset_code
            );
            // Defence-in-depth: codes are protocol-alphanumeric + from our own
            // rows, but escape anyway (CH string-literal rules) so a malformed
            // ingested code can never break or inject the inlined literal — the
            // `debug_assert` above is compiled out in release.
            let code = k.asset_code.replace('\\', "\\\\").replace('\'', "\\'");
            format!(
                "({},'{}',{},{})",
                k.asset_type, code, k.issuer_id, k.contract_id
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Consecutive-dedup the over-fetched phase-1 rows by the identity 4-tuple,
/// keeping the first version of each key, and truncate to `limit`. Correct
/// because the seek's `ORDER BY` IS the `assets` primary key, so a key's physical
/// versions are contiguous; the projected columns are byte-identical across
/// them, so "keep first" is deterministic (task 0364, approach-B — NOT `LIMIT 1
/// BY`, which would defeat `optimize_read_in_order` on the seek).
fn dedup_consecutive(raw: Vec<AssetKeyChRow>, limit: usize) -> Vec<AssetKeyChRow> {
    let mut out: Vec<AssetKeyChRow> = Vec::with_capacity(limit.min(raw.len()));
    for r in raw {
        if out.last().is_none_or(|p| p.tuple() != r.tuple()) {
            out.push(r);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// Phase-2 hydration query for a resolved page/lookup key set (task 0364).
///
/// `assets` is seeked by its primary key (the identity 4-tuple, no `FINAL`) and
/// the side tables are bounded to that key set: `asset_enrichment` / `asset_sac`
/// by the 4-tuple before their GROUP BY, `balance_aggregates` by `asset_id`. The
/// `soroban_contracts` header (contract StrKey, deploy ledger) + its
/// `soroban_contract_metadata` (name / symbol / decimals) are NOT joined here —
/// they resolve ONCE, off the surrogate ids, in [`resolve_soroban_contracts`]
/// (one bloom seek reused for own + SAC-wrapper, not two inlined joins CH could
/// not de-duplicate). `tuples` inlines the 4-tuples as literals (so CH uses the
/// PK index; a subquery IN would full-scan); `asset_ids` inlines the surrogate
/// ids. `LIMIT 1 BY` collapses the versions that dropping `FINAL` on `assets`
/// re-exposes — deterministic (every projected `a` column is immutable across
/// a key's versions).
fn hydrate_sql(tuples: &str, asset_ids: &str) -> String {
    format!(
        "SELECT \
             a.asset_type                AS asset_type, \
             nullIf(a.asset_code, '')    AS asset_code, \
             nullIf(ae.name, '')         AS name_enrichment, \
             toString(bagg.total_supply) AS total_supply, \
             bagg.holder_count           AS holder_count, \
             nullIf(ae.icon_url, '')     AS icon_url, \
             a.issuer_id                 AS issuer_id_key, \
             a.contract_id               AS contract_id_key, \
             sac.sac_contract_id         AS sac_contract_surrogate, \
             sac.sac_deployed            AS sac_deployed, \
             a.id                        AS id \
         FROM assets a \
         LEFT JOIN ( \
             SELECT asset_id, total_supply, holder_count \
             FROM balance_aggregates WHERE asset_id IN ({asset_ids}) \
         ) bagg ON bagg.asset_id = a.id \
         LEFT JOIN ( \
             SELECT asset_type, asset_code, issuer_id, contract_id, \
                    argMax(icon_url, version) AS icon_url, \
                    argMax(name, version)     AS name \
             FROM asset_enrichment \
             WHERE (asset_type, asset_code, issuer_id, contract_id) IN ({tuples}) \
             GROUP BY asset_type, asset_code, issuer_id, contract_id \
         ) ae ON ae.asset_type  = a.asset_type  AND ae.asset_code  = a.asset_code \
             AND ae.issuer_id   = a.issuer_id   AND ae.contract_id = a.contract_id \
         LEFT JOIN ( \
             SELECT asset_type, asset_code, issuer_id, contract_id, \
                    max(sac_contract_id)        AS sac_contract_id, \
                    toBool(max(sac_deployed))   AS sac_deployed \
             FROM asset_sac \
             WHERE (asset_type, asset_code, issuer_id, contract_id) IN ({tuples}) \
             GROUP BY asset_type, asset_code, issuer_id, contract_id \
         ) sac ON sac.asset_type  = a.asset_type  AND sac.asset_code  = a.asset_code \
             AND sac.issuer_id   = a.issuer_id   AND sac.contract_id = a.contract_id \
         WHERE (a.asset_type, a.asset_code, a.issuer_id, a.contract_id) IN ({tuples}) \
         LIMIT 1 BY a.asset_type, a.asset_code, a.issuer_id, a.contract_id"
    )
}

/// Phase 2: hydrate a resolved key set into full [`AssetListChRow`] headers,
/// re-ordered to match `keys`. Every side dimension is bounded to `keys` (see
/// [`hydrate_sql`]). Returns rows keyed back to the phase-1 order via `id`
/// (unique per asset); a hydration miss (should not happen — the keys came from
/// `assets`) is skipped. `with_sac_wrapper` resolves a SAC-wrapper's deploy
/// ledger for `deployed_at_ledger` — `true` for the detail (which serialises it),
/// `false` for the list (which drops it, so it skips the extra wrapper seek).
async fn hydrate_assets(
    client: &clickhouse::Client,
    keys: &[AssetKeyChRow],
    with_sac_wrapper: bool,
) -> Result<Vec<AssetListChRow>, clickhouse::error::Error> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let tuples = asset_key_tuples(keys);
    let asset_ids = id_in_list(&keys.iter().map(|k| k.id).collect());

    // Resolve the `soroban_contracts` context (StrKey + deploy ledger + metadata)
    // ONCE, off the surrogate ids — replacing the pre-2c pair of inlined joins.
    // For the DETAIL (`with_sac_wrapper`) it also seeks a SAC-wrapper's surrogate
    // from `asset_sac` (for the wrapper's deploy ledger); the list drops
    // `deployed_at_ledger`, so it skips that. Chained in `ctx_fut` so the resolve
    // overlaps the header hydration below (one `soroban_contracts` seek total).
    let ctx_fut = async {
        let mut sc_ids: BTreeSet<i64> = keys
            .iter()
            .map(|k| k.contract_id)
            .filter(|&c| c != 0)
            .collect();
        if with_sac_wrapper {
            let sac_wrappers = client
                .query(&format!(
                    "SELECT DISTINCT sac_contract_id AS id \
                     FROM asset_sac \
                     WHERE (asset_type, asset_code, issuer_id, contract_id) IN ({tuples}) \
                       AND sac_contract_id != 0"
                ))
                .fetch_all::<SurrogateIdRow>()
                .await?;
            sc_ids.extend(sac_wrappers.into_iter().map(|r| r.id));
        }
        resolve_soroban_contracts(client, &sc_ids).await
    };

    let hydrate = hydrate_sql(&tuples, &asset_ids);
    let (rows, ctx) = tokio::join!(
        client.query(&hydrate).fetch_all::<AssetHydrateRow>(),
        ctx_fut,
    );
    let rows = rows?;
    let ctx = ctx?;

    // Re-emit in phase-1 order (the seek's page/tiebreak order), assembling the
    // `soroban_contracts`-derived display fields from `ctx`. Index by id.
    let mut by_id: HashMap<i64, AssetListChRow> = HashMap::with_capacity(rows.len());
    for row in rows {
        by_id.insert(row.id, assemble_asset_row(row, &ctx));
    }
    Ok(keys.iter().filter_map(|k| by_id.remove(&k.id)).collect())
}

/// Assemble one [`AssetListChRow`] from its `assets`-side header and the
/// `soroban_contracts` context map (task 0364 2c). Replicates the pre-2c SQL
/// projection exactly: `name = coalesce(ae.name, metadata.name, native-default)`,
/// `decimals = coalesce(metadata.decimals, 7)`, `deployed_at_ledger` prefers the
/// own contract then the SAC-wrapper (both `nullIf 0`), StrKey / symbol are
/// non-empty-or-`None`.
fn assemble_asset_row(h: AssetHydrateRow, ctx: &HashMap<i64, SorobanCtxRow>) -> AssetListChRow {
    let nonempty = |s: Option<String>| s.filter(|v| !v.is_empty());
    // Own contract (type-3); `None` for classic/native (`contract_id_key == 0`,
    // never a `soroban_contracts` id, so absent from `ctx`).
    let own = ctx.get(&h.contract_id_key);
    let deploy_of = |id: i64| {
        ctx.get(&id)
            .and_then(|c| c.deployed_at_ledger)
            .filter(|&v| v != 0)
    };

    AssetListChRow {
        asset_type: h.asset_type,
        asset_code: h.asset_code,
        contract_id: nonempty(own.map(|c| c.contract_id.clone())),
        // ae.name, then on-chain metadata name, then the native singleton label.
        name: h
            .name_enrichment
            .or_else(|| nonempty(own.and_then(|c| c.name.clone())))
            .or_else(|| (h.asset_type == 0).then(|| "Stellar Lumens".to_string())),
        symbol: nonempty(own.and_then(|c| c.symbol.clone())),
        decimals: own.and_then(|c| c.decimals).unwrap_or(7),
        total_supply: h.total_supply,
        holder_count: h.holder_count,
        // Own contract's deploy ledger, else the SAC-wrapper's (ADR 0051).
        deployed_at_ledger: deploy_of(h.contract_id_key)
            .or_else(|| deploy_of(h.sac_contract_surrogate)),
        icon_url: h.icon_url,
        issuer_id_key: h.issuer_id_key,
        contract_id_key: h.contract_id_key,
        sac_contract_surrogate: h.sac_contract_surrogate,
        sac_deployed: h.sac_deployed,
        id: h.id,
    }
}

/// One-column surrogate-id row (`asset_sac.sac_contract_id` seek).
#[derive(Debug, Row, Deserialize)]
struct SurrogateIdRow {
    id: i64,
}

/// `soroban_contracts` context for one surrogate `id`, resolved once per
/// page/lookup ([`resolve_soroban_contracts`]) and reused for both the asset's
/// own contract and its SAC-wrapper. Metadata (`name`/`symbol`/`decimals`) is
/// folded in from the `soroban_contract_metadata` join in the same read.
#[derive(Debug, Row, Deserialize)]
struct SorobanCtxRow {
    id: i64,
    contract_id: String,
    deployed_at_ledger: Option<i64>,
    name: Option<String>,
    symbol: Option<String>,
    decimals: Option<u32>,
}

/// Inline an `i64` set as an `IN (…)` list, with a `0` sentinel for the empty
/// set (surrogate ids are non-zero `cityhash64`, so `IN (0)` matches nothing but
/// stays valid SQL).
fn id_in_list(it: &BTreeSet<i64>) -> String {
    if it.is_empty() {
        "0".to_string()
    } else {
        it.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
    }
}

/// Resolve `soroban_contracts` context by surrogate `id` (task 0364 2c) — one
/// bloom seek (`idx_sc_id`) that replaces the pre-2c pair of inlined joins (the
/// asset's own contract AND its SAC-wrapper both read `soroban_contracts`). The
/// `soroban_contract_metadata FINAL` (name/symbol/decimals) is folded in via a
/// LEFT JOIN on the bounded `contract_id`, so no extra round-trip. Version-
/// correct dedup with `argMax(_, wasm_uploaded_at_ledger)` skips the low-version
/// NULL stub rows (see [`hydrate_sql`] history); `id` is not the sort key, so the
/// read is a bloom seek (~1 granule/id), not a whole-table scan.
async fn resolve_soroban_contracts(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<HashMap<i64, SorobanCtxRow>, clickhouse::error::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let in_list = id_in_list(ids);
    Ok(client
        .query(&format!(
            "SELECT sc.id AS id, \
                    argMax(sc.contract_id, sc.wasm_uploaded_at_ledger)        AS contract_id, \
                    argMax(sc.deployed_at_ledger, sc.wasm_uploaded_at_ledger) AS deployed_at_ledger, \
                    any(m.name)     AS name, \
                    any(m.symbol)   AS symbol, \
                    any(m.decimals) AS decimals \
             FROM soroban_contracts sc \
             LEFT JOIN ( \
                 SELECT contract_id, name, symbol, decimals \
                 FROM soroban_contract_metadata FINAL \
             ) m ON m.contract_id = sc.contract_id \
             WHERE sc.id IN ({in_list}) \
             GROUP BY sc.id"
        ))
        .fetch_all::<SorobanCtxRow>()
        .await?
        .into_iter()
        .map(|r| (r.id, r))
        .collect())
}

// ---------------------------------------------------------------------------
// List — GET /v1/assets (canonical 08)
// ---------------------------------------------------------------------------

/// Over-fetch factor for the phase-1 seek: read `limit * OVERFETCH` raw versions
/// so the consecutive-dedup still yields a full page under the `assets` version
/// bloat. **8** holds while the max versions-per-key stays below it. Version count
/// is a LIVE property (a `ReplacingMergeTree` merge collapses versions, a re-ingest
/// grows them): measured avg ~2.3×, max 4–7 depending on merge state (prod,
/// 2026-07-16), so `limit * 8` contains `limit` distinct keys with headroom. Not a
/// hard guarantee — the loud under-fill guard in [`fetch_list`] is the backstop if
/// a re-ingest burst ever pushes a hot key past 8.
const SEEK_OVERFETCH: i64 = 8;

/// Build the phase-1 list seek: the page's identity keys (4-tuple + `id`) from
/// `assets` WITHOUT `FINAL`, in PK order, over-fetched by [`SEEK_OVERFETCH`] for
/// the Rust consecutive-dedup. Pure (no client) so the predicate shape + `?`
/// count stay unit-testable: `code_clause` (0 or **3**), then `cursor_clause`
/// (0 or 4). The trailing `LIMIT` is inlined. Only `sac_only` / search pull side
/// tables into the seek; the default page is a pure `assets` PK walk.
/// The displayed code of an asset row — native's `XLM` standing in for its
/// empty stored code. Mirrored in `search::queries` and `pool_asset_codes`.
const SHOWN: &str = "lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code)))";

fn build_list_seek_sql(params: &ResolvedListParams, direction: Direction) -> String {
    // ASC, both paths (task 0485). Native XLM is `asset_type = 0` with an EMPTY
    // code — the MINIMUM of the identity 4-tuple this keyset walks — so under
    // DESC it sorts onto the LAST page: `filter[code]=xlm` answered with
    // `zXLMr, zXLM, …`, and the unfiltered list opened on six codeless Soroban
    // contracts instead of XLM.
    //
    // The old DESC was not a decision. It came from `keyset_sql_desc`, whose
    // own doc calls it the helper for endpoints with a single NEWEST-FIRST
    // order — but this key holds no time. It read "newest first" and delivered
    // reverse-alphabetical. Nothing pinned it: no `?order=` param, no sort
    // control in the frontend, no claim in `docs/architecture/**`.
    //
    // Same cost either way — the order is still the `assets` primary key, so
    // the unfiltered page stays a PK-prefix walk. `op` comes from the same
    // call as `order`, so the cursor comparator always points into the walk;
    // that pairing is what a keyset needs and why they are never chosen apart.
    let (op, order) = keyset_sql(SortOrder::Asc, direction);
    let lim_over = params.limit * SEEK_OVERFETCH;

    let type_clause = params
        .asset_type
        .map_or_else(String::new, |t| format!(" AND a.asset_type = {t}"));
    // SAC property filter (ADR 0051): the old `filter[type]=sac` view. Was a
    // post-join predicate `sac.sac_deployed`; now a semi-join over the identity
    // 4-tuple so the seek stays `assets`-driven (the deployed set comes straight
    // from `asset_sac`; task 0339 UX: deployed-only excludes reserved addresses).
    let sac_clause = if params.sac_only {
        " AND (a.asset_type, a.asset_code, a.issuer_id, a.contract_id) IN ( \
             SELECT asset_type, asset_code, issuer_id, contract_id FROM asset_sac \
             GROUP BY asset_type, asset_code, issuer_id, contract_id \
             HAVING toBool(max(sac_deployed)))"
    } else {
        ""
    };
    // Match the needle against the short `asset_code` AND the on-chain
    // name/symbol from `soroban_contract_metadata` (`m`). type-3 assets carry an
    // empty `asset_code`, so a code-only match leaves them unfindable (task
    // 0370).
    //
    // **Native XLM is stored with an EMPTY code too**, so it needs the same
    // `if(type = 0, 'XLM', code)` arm the pools predicate uses
    // (`common::pool_asset_codes`, task 0470). Without it `XLM` returned 6 404
    // credit assets minted under a code containing "XLM" and not the one asset
    // everybody meant — measured on production, where the native row matched 0
    // times. Search the needle against what the row DISPLAYS as, not against
    // what happens to be stored.
    //
    // In the FINAL select `m` was always joined; the seek only joins it
    // (+ `sc`) WHEN searching, so the default page stays a bare `assets` walk. We
    // do NOT match `asset_enrichment.name` — substring-matching the classic
    // SEP-1 names adds noise for no gain (classic assets are findable by code).
    //
    // ponytail: `sc` is NOT deduped — `soroban_contracts` is version-full (~95% of
    // ids carry >1 version), so this join multiplies matching rows and can dilute
    // the over-fetch window on a dense type-3 search page → a rare under-fill
    // (short page, no next cursor; the guard in `fetch_list` warns). Left as-is:
    // deduping via `GROUP BY id` measured +30ms on EVERY search (44→78ms) to fix a
    // near-unreachable edge (assets max versions = 4). Revisit (dedup, or a search
    // over-fetch bump) if version bloat grows (task 0364 audit F1).
    // The needle is matched against the code the row DISPLAYS, never the one it
    // stores: native XLM stores an EMPTY code and renders as `XLM` (task 0485).
    // Name and symbol stay separate — they are free text from on-chain
    // metadata, not asset codes, so the native alias has nothing to say there.
    //
    // The same expression appears in `search::queries` and in
    // `common::pool_asset_codes`; `native_is_matched_by_type_not_by_stored_code`
    // below is the test that fails if this copy drifts from them.
    let (search_join, code_clause) = if params.asset_code.is_some() {
        (
            " LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id \
              LEFT JOIN (SELECT contract_id, name, symbol FROM soroban_contract_metadata FINAL) m \
                  ON m.contract_id = sc.contract_id",
            format!(
                " AND (position({SHOWN}, lower(?)) > 0 \
                   OR positionCaseInsensitive(coalesce(m.name, ''), ?) > 0 \
                   OR positionCaseInsensitive(coalesce(m.symbol, ''), ?) > 0)"
            ),
        )
    } else {
        ("", String::new())
    };
    // NO relevance ranking here, deliberately (task 0485). A tier order would
    // have to be carried IN THE CURSOR — a keyset must resume in the order it
    // walked — which drags in a rank column, a second copy of the tier rule in
    // Rust to mint that cursor, and a page-walk test to catch the two drifting.
    // That machinery was built, measured and taken back out: this is a browse
    // list with a filter, and the ask was "native first", which the walk
    // direction already gives on its own. Relevance lives in `/v1/search`,
    // which has no pagination and therefore pays none of it.
    let cursor_clause = if params.cursor.is_some() {
        format!(" AND (a.asset_type, a.asset_code, a.issuer_id, a.contract_id) {op} (?, ?, ?, ?)")
    } else {
        String::new()
    };

    format!(
        "SELECT a.asset_type AS asset_type, a.asset_code AS asset_code, \
                a.issuer_id AS issuer_id, a.contract_id AS contract_id, a.id AS id \
         FROM assets a{search_join} \
         WHERE 1{type_clause}{sac_clause}{code_clause}{cursor_clause} \
         ORDER BY a.asset_type {order}, a.asset_code {order}, \
                  a.issuer_id {order}, a.contract_id {order} \
         LIMIT {lim_over}"
    )
}

/// **Read-cost caveat:** the keyset/`ORDER BY` is the identity 4-tuple — which
/// IS the `assets` primary key — so the page is a PK-prefix walk, cheap. The
/// `FINAL` + lookup joins widen it; needs an operator smoke before the prod
/// flag flip, same as the other CH lists.
pub async fn fetch_list(
    client: &clickhouse::Client,
    params: &ResolvedListParams,
    direction: Direction,
) -> Result<Vec<AssetRow>, clickhouse::error::Error> {
    let sql = build_list_seek_sql(params, direction);

    let mut query = client.query(&sql);
    if let Some(code) = &params.asset_code {
        // Three placeholders in `code_clause`: the shared code test, m.name,
        // m.symbol. All take the same needle.
        query = query.bind(code).bind(code).bind(code);
    }
    if let Some(c) = &params.cursor {
        query = query
            .bind(c.asset_type)
            .bind(&c.asset_code)
            .bind(c.issuer_id)
            .bind(c.contract_id);
    }
    let raw = query.fetch_all::<AssetKeyChRow>().await?;
    let raw_len = raw.len();
    // `params.limit` is the handler's `fetch_limit()` (already the peek +1);
    // consecutive-dedup the over-fetched versions to the page, then hydrate it.
    let keys = dedup_consecutive(raw, params.limit as usize);
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    // Under-fill guard: the over-fetch must still yield a full page after
    // version-dedup (holds while max versions-per-key < `SEEK_OVERFETCH`). If a
    // FULL raw fetch (`raw_len == lim_over`, so more rows existed) still deduped
    // to < `limit`, bloat exceeded the ceiling and the page silently truncated —
    // log it loudly instead of dropping the tail.
    if (keys.len() as i64) < params.limit && raw_len as i64 == params.limit * SEEK_OVERFETCH {
        tracing::warn!(
            limit = params.limit,
            got = keys.len(),
            over_fetch = SEEK_OVERFETCH,
            "assets list under-filled after version-dedup — raise SEEK_OVERFETCH"
        );
    }
    // Hydrate the page AND resolve the page's issuers concurrently: both depend
    // only on the phase-1 keys (`issuer_id` IS part of the identity 4-tuple), so
    // the issuer seek need not wait for hydration — one fewer serial round-trip
    // to CH (task 0364; same `tokio::join!` shape as the tx-list two-step).
    let issuer_ids = keys
        .iter()
        .map(|k| k.issuer_id)
        .filter(|&i| i != 0)
        .collect::<BTreeSet<_>>();
    let (rows, issuers) = tokio::join!(
        hydrate_assets(client, &keys, false),
        resolve_page_issuers(client, issuer_ids)
    );
    let rows = rows?;
    let issuers = issuers?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let iss = issuers.get(&r.issuer_id_key).cloned();
            list_row_to_asset_row(r, iss)
        })
        .collect())
}

/// Resolve a page's issuer surrogates → `(StrKey, home_domain)` by the
/// bloom-pruned `accounts.id` key-seek (task 0319), replacing the full-table
/// `accounts iss` join. `id = 0` (native / no issuer) MUST be excluded by the
/// caller. `ORDER BY last_seen_ledger DESC LIMIT 1 BY id` picks the latest
/// version deterministically (no `FINAL`) — `home_domain` is mutable
/// (SET_OPTIONS). Runs off the phase-1 keys so it can overlap hydration.
async fn resolve_page_issuers(
    client: &clickhouse::Client,
    ids: BTreeSet<i64>,
) -> Result<HashMap<i64, (String, Option<String>)>, clickhouse::error::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let in_list = id_in_list(&ids);
    Ok(client
        .query(&format!(
            "SELECT id AS id, account_id AS account_id, home_domain AS home_domain \
             FROM accounts WHERE id IN ({in_list}) \
             ORDER BY last_seen_ledger DESC LIMIT 1 BY id"
        ))
        .fetch_all::<AssetIssuerRow>()
        .await?
        .into_iter()
        .map(|r| (r.id, (r.account_id, r.home_domain)))
        .collect())
}

// ---------------------------------------------------------------------------
// Detail — GET /v1/assets/:id (canonical 09), three resolution forms
// ---------------------------------------------------------------------------

/// Resolve by contract StrKey (`C…`) — a bespoke `soroban` token, where the
/// contract IS the asset (type-3).
///
/// A SAC-wrapped classic/native asset is deliberately NOT resolved here: it is a
/// FACET of its classic/native asset (ADR 0051), addressed by its canonical
/// `CODE-ISSUER` (or `native`). Its SAC `C…` is shown only as a contract
/// reference, never as an `/assets/` link — nothing in the app hits
/// `/assets/{SAC C…}` (task 0364 dropped the redundant SAC-facet arm; a pasted
/// SAC address now 404s instead of aliasing the wrapped asset). Issuer StrKey +
/// home_domain resolve by a single `accounts.id` key-seek (task 0334).
pub async fn fetch_by_contract_id(
    client: &clickhouse::Client,
    contract_id: &str,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    // A bespoke token IS its own contract: on prod every type-3 asset has an empty
    // code + no issuer, so its identity is the FULL primary key `(3, '', 0,
    // surrogate)`, and `asset_id(3, "", 0, surrogate) == surrogate` (the type-3 arm
    // of `ids::asset_id`), so the `id` is known too. The whole key is determined by
    // the StrKey — skip the phase-1 seek and hydrate it directly, like
    // `fetch_native` (task 0364 audit F2). A non-existent contract hydrates to zero
    // rows → None → 404, identical to an empty seek. `surrogate` is `hash64` of the
    // StrKey, the same value stored as `assets.contract_id`.
    let contract_surrogate = db_clickhouse::persist::ids::contract_id(contract_id);
    let key = AssetKeyChRow {
        asset_type: 3,
        asset_code: String::new(),
        issuer_id: 0,
        contract_id: contract_surrogate,
        id: contract_surrogate,
    };
    let row = hydrate_assets(client, &[key], true)
        .await?
        .into_iter()
        .next();
    finish_detail(client, row).await
}

/// Resolve by `CODE-ISSUER` (classic credit / classic-wrap SAC). `issuer` is the
/// G-StrKey. Resolve it to the surrogate `issuer_id` first via the
/// `accounts.account_id` PK seek (accounts is `ORDER BY account_id`), then filter
/// `assets` by `(asset_code, issuer_id)` — no full `accounts` join (task 0334).
/// The same seek yields the issuer StrKey + home_domain, so no second lookup.
/// `ORDER BY a.asset_type` is a deterministic tiebreak; post-ADR 0051 a
/// `(code, issuer)` maps to a single classic_credit row (its SAC is a facet on
/// that same row, not a separate type=2), so at most one row matches anyway.
pub async fn fetch_by_code_issuer(
    client: &clickhouse::Client,
    asset_code: &str,
    issuer: &str,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    let Some(issuer_row) = seek_latest_account(client, "account_id", issuer).await? else {
        return Ok(None);
    };

    // Phase 1 — seek the key by `(asset_code, issuer_id)` (no `FINAL`), then
    // hydrate bounded (task 0364). `ORDER BY a.asset_type` is the deterministic
    // tiebreak (post-ADR 0051 a `(code, issuer)` maps to a single classic_credit
    // row anyway).
    let keys = client
        .query(
            "SELECT a.asset_type AS asset_type, a.asset_code AS asset_code, \
                    a.issuer_id AS issuer_id, a.contract_id AS contract_id, a.id AS id \
             FROM assets a \
             WHERE a.asset_code = ? AND a.issuer_id = ? \
             ORDER BY a.asset_type LIMIT 1",
        )
        .bind(asset_code)
        .bind(issuer_row.id)
        .fetch_all::<AssetKeyChRow>()
        .await?;
    let Some(row) = hydrate_assets(client, &keys, true)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    // The issuer StrKey + home_domain came from the up-front seek — map directly,
    // no second `accounts` lookup.
    let iss = (issuer_row.account_id, issuer_row.home_domain);
    Ok(Some(list_row_to_asset_row(row, Some(iss))))
}

/// Resolve the classic native XLM singleton (`asset_type = 0`). Native has no
/// composite identity, so it is addressed by the reserved `/assets/native`
/// token rather than a StrKey or CODE-ISSUER pair. `issuer_id = 0` → no issuer
/// seek (task 0334).
pub async fn fetch_native(
    client: &clickhouse::Client,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    // Native XLM is the fixed singleton `(asset_type = 0, code = '', issuer = 0,
    // contract = 0)` — its identity 4-tuple and surrogate `id` are compile-time
    // constants, so there is no key to *discover*: skip the phase-1 seek and
    // hydrate the known key directly (task 0364).
    let key = AssetKeyChRow {
        asset_type: 0,
        asset_code: String::new(),
        issuer_id: 0,
        contract_id: 0,
        id: db_clickhouse::persist::ids::NATIVE_ASSET_ID,
    };
    let row = hydrate_assets(client, &[key], true)
        .await?
        .into_iter()
        .next();
    finish_detail(client, row).await
}

// ---------------------------------------------------------------------------
// Transactions — GET /v1/assets/:id/transactions (canonical 10, two-step)
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AssetTxKeyChRow {
    ledger_sequence: i64,
    transaction_id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct AssetTxPageChRow {
    id: i64,
    hash: String,
    ledger_sequence: i64,
    source_id: i64,
    fee_charged: i64,
    successful: bool,
    operation_count: i16,
    has_soroban: bool,
    created_at: i64,
}

/// Transactions touching a single asset, newest first, keyset-paginated over the
/// `operation_asset_appearances` fan-out (task 0359 — replaces the old
/// per-`asset_type` predicate composition over `operations_appearances`).
///
/// `asset_id` is the resolved `ids::asset_id` surrogate (native = a first-class
/// non-zero key; the caller guards the unresolved `id == 0` sentinel). Two-step
/// like the accounts sub-resource: a leading-key seek over
/// `operation_asset_appearances` (`asset_id` IS the leading PK) collapses any
/// multi-op-per-tx fan-out (`LIMIT 1 BY (ledger_sequence, transaction_id)`) behind
/// the `max(sequence)` commit fence, then the ≤`limit` transaction headers are
/// fetched by `(ledger_sequence, id) IN (keys)` (PK-prefix prune,
/// multi-partition-safe) and the `operation_types` aggregate is merged. The caller
/// passes the handler's `fetch_limit()` (already the `+1` `finalize_page` peek
/// row), inlined raw — `asset_id` is a provably-numeric i64.
pub async fn fetch_transactions(
    client: &clickhouse::Client,
    asset_id: i64,
    contract_surrogate: Option<i64>,
    limit: i64,
    cursor: Option<&TxListCursor>,
    direction: Direction,
) -> Result<Vec<AssetTxRow>, clickhouse::error::Error> {
    let (op, order) = keyset_sql_desc(direction);

    // CH cursor only (the handler's cross-source guard rejects a `Pg` variant).
    // Inlined i64 — no injection surface; omitted on the first page so no NULL is
    // bound. Unqualified columns: each seek is a single-table read, no ambiguity.
    let cursor_clause = match cursor {
        Some(TxListCursor::Ch {
            ledger_sequence,
            tiebreak,
        }) => {
            format!(" AND (ledger_sequence, transaction_id) {op} ({ledger_sequence}, {tiebreak})")
        }
        _ => String::new(),
    };

    // Step 1 — one or two keyset arms, merged (task 0359 composed read):
    //   A. `operation_asset_appearances` on `asset_id` — classic / native activity
    //      (empty for a pure type-3 token: no classic op names it).
    //   B. `soroban_invocations_appearances` on the asset's single associated
    //      contract (ADR 0051 — a type-3 token's OWN contract, or a classic/native
    //      asset's wrapping SAC, never both). Serves the type-3 asset page (its
    //      fan-out is empty — the #1 regression fix) and a classic asset's SAC
    //      activity (F-F). `None` → arm A only.
    // Same seek both arms: leading-PK range behind the `max(sequence)` fence,
    // `LIMIT 1 BY` per tx. The union's top-`limit` (sort + dedup + truncate) is the
    // global page; the outer `LIMIT 1 BY` drops a tx returned by both arms.
    let seek = |table: &str, key_col: &str, key: i64| {
        format!(
            "SELECT ledger_sequence, transaction_id FROM {table} \
             WHERE {key_col} = {key} \
               AND ledger_sequence <= (SELECT max(sequence) FROM ledgers){cursor_clause} \
             ORDER BY ledger_sequence {order}, transaction_id {order} \
             LIMIT 1 BY ledger_sequence, transaction_id \
             LIMIT {limit}"
        )
    };

    let arm_a = seek("operation_asset_appearances", "asset_id", asset_id);
    let sql = match contract_surrogate {
        // Arm A alone already IS the page — no wrapper to add.
        None => arm_a,
        Some(contract_id) => {
            let arm_b = seek(
                "soroban_invocations_appearances",
                "contract_id",
                contract_id,
            );
            union_keyset_arms(&arm_a, &arm_b, order, limit)
        }
    };

    let keys: Vec<(i64, i64)> = client
        .query(&sql)
        .fetch_all::<AssetTxKeyChRow>()
        .await?
        .iter()
        .map(|r| (r.ledger_sequence, r.transaction_id))
        .collect();

    if keys.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: transaction headers for the page keys + the operation_types
    // aggregate, concurrently. Keys are i64 — inlined.
    let in_tuples = keys
        .iter()
        .map(|(ledger, tx)| format!("({ledger},{tx})"))
        .collect::<Vec<_>>()
        .join(",");
    let partitions = keys
        .iter()
        .map(|(ledger, _)| ledger / 500_000)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let page_sql = format!(
        "SELECT \
            t.id AS id, \
            lower(hex(t.hash)) AS hash, \
            t.ledger_sequence AS ledger_sequence, \
            t.source_id AS source_id, \
            t.fee_charged AS fee_charged, \
            t.successful AS successful, \
            t.operation_count AS operation_count, \
            t.has_soroban AS has_soroban, \
            l.closed_at AS created_at \
         FROM transactions t \
         INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
         WHERE (t.ledger_sequence, t.id) IN ({in_tuples}) \
           AND intDiv(t.ledger_sequence, 500000) IN ({partitions})"
    );
    let (page_rows, aggregates) = tokio::join!(
        client.query(&page_sql).fetch_all::<AssetTxPageChRow>(),
        ch::fetch_tx_list_aggregates(client, &keys),
    );
    let page_rows = page_rows?;
    let aggregates = aggregates?;
    // Resolve source StrKeys by surrogate id (bloom seek) instead of a
    // whole-`accounts` `LEFT JOIN accounts src` (task 0354).
    let accounts =
        resolve_accounts(client, page_rows.iter().map(|r| r.source_id).collect()).await?;

    // Index by id, then emit in the driver's keyset order, merging
    // operation_types (the only aggregate an asset-transaction item carries).
    let mut by_id: HashMap<i64, AssetTxPageChRow> = HashMap::with_capacity(page_rows.len());
    for row in page_rows {
        by_id.insert(row.id, row);
    }
    let mut out = Vec::with_capacity(keys.len());
    for (_, tx_id) in &keys {
        let Some(row) = by_id.remove(tx_id) else {
            continue;
        };
        let agg = aggregates.get(tx_id);
        let operation_types = agg.map(|a| a.operation_types.clone()).unwrap_or_default();
        out.push(AssetTxRow {
            id: row.id,
            hash: row.hash,
            ledger_sequence: row.ledger_sequence,
            source_account: accounts
                .get(&row.source_id)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_default(),
            successful: row.successful,
            fee_charged: row.fee_charged,
            created_at: millis_to_utc(row.created_at),
            operation_count: row.operation_count,
            has_soroban: row.has_soroban,
            operation_types,
        });
    }
    Ok(out)
}

/// Fold the keyset arms into ONE statement (task 0446).
///
/// The arms read different tables and neither consumes the other's output, so
/// they used to be awaited one at a time and merged in Rust — two serial round
/// trips to a ClickHouse that sits outside the AWS network (measured 14.2 ms
/// each). ClickHouse does the same merge itself: the outer `ORDER BY` is the
/// page direction, the outer `LIMIT 1 BY` drops a transaction both arms
/// returned, and the outer `LIMIT` truncates to the page. Each arm keeps its own
/// `ORDER BY`/`LIMIT` so it still returns at most `limit` rows to the union.
///
/// Takes the two arms directly rather than a slice: there are exactly one or
/// two, and a slice would model that as "zero or more" — an empty one would
/// build `FROM ()`, invalid SQL no type checks. The lone-arm case is the
/// caller's `None`, which needs no wrapper at all (`seek` already emits its own
/// `ORDER BY` / `LIMIT 1 BY` / `LIMIT`).
fn union_keyset_arms(arm_a: &str, arm_b: &str, order: &str, limit: i64) -> String {
    format!(
        "SELECT ledger_sequence, transaction_id FROM (({arm_a}) UNION ALL ({arm_b})) \
         ORDER BY ledger_sequence {order}, transaction_id {order} \
         LIMIT 1 BY ledger_sequence, transaction_id \
         LIMIT {limit}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_keyset_arms_merges_both_arms_in_one_statement() {
        // task 0446: the two arms are independent reads; they must cost ONE round
        // trip, with the merge (order, cross-arm dedup, truncate) pushed into CH.
        let sql = union_keyset_arms("SELECT a", "SELECT b", "DESC", 21);
        assert!(sql.contains("(SELECT a) UNION ALL (SELECT b)"));
        assert!(sql.contains("ORDER BY ledger_sequence DESC, transaction_id DESC"));
        // Drops a transaction returned by BOTH arms — the old Rust `keys.dedup()`.
        assert!(sql.contains("LIMIT 1 BY ledger_sequence, transaction_id"));
        assert!(sql.ends_with("LIMIT 21"));
    }

    #[test]
    fn asset_key_tuples_inlines_type_code_issuer_contract() {
        let keys = vec![
            AssetKeyChRow {
                asset_type: 1,
                asset_code: "USDC".to_string(),
                issuer_id: 42,
                contract_id: 0,
                id: 7,
            },
            AssetKeyChRow {
                asset_type: 0,
                asset_code: String::new(), // native — empty code
                issuer_id: 0,
                contract_id: 0,
                id: 9,
            },
        ];
        assert_eq!(asset_key_tuples(&keys), "(1,'USDC',42,0),(0,'',0,0)");
    }

    #[test]
    fn dedup_consecutive_keeps_first_version_per_key_and_truncates() {
        let key = |code: &str, id: i64| AssetKeyChRow {
            asset_type: 1,
            asset_code: code.to_string(),
            issuer_id: 1,
            contract_id: 0,
            id,
        };
        // Two physical versions of AAA (contiguous in PK order), then BBB, CCC.
        let raw = vec![key("AAA", 1), key("AAA", 1), key("BBB", 2), key("CCC", 3)];
        let out = dedup_consecutive(raw, 2);
        // Collapses AAA's versions, then truncates to the page limit.
        assert_eq!(
            out.iter()
                .map(|k| k.asset_code.as_str())
                .collect::<Vec<_>>(),
            ["AAA", "BBB"]
        );
    }

    #[test]
    fn asset_type_name_matches_pg_function() {
        assert_eq!(asset_type_name(0).as_deref(), Some("native"));
        assert_eq!(asset_type_name(1).as_deref(), Some("classic_credit"));
        // 2 (`sac`) retired — ADR 0051.
        assert_eq!(asset_type_name(2), None);
        assert_eq!(asset_type_name(3).as_deref(), Some("soroban"));
        assert_eq!(asset_type_name(99), None);
    }

    #[test]
    fn list_sql_searches_name_and_symbol_when_code_present() {
        // task 0370: type-3 (Soroban-native) assets have an empty `asset_code`;
        // their name/symbol live in the joined contract metadata, so the list
        // search must match those columns too or they are unfindable by name.
        let params = ResolvedListParams {
            limit: 10,
            cursor: None,
            asset_type: None,
            asset_code: Some("solv".to_string()),
            sac_only: false,
        };
        let sql = build_list_seek_sql(&params, Direction::Next);
        assert!(sql.contains("toString(a.asset_code)"));
        assert!(sql.contains("coalesce(m.name, '')"));
        assert!(sql.contains("coalesce(m.symbol, '')"));
        // Classic enrichment names (ae.name) are intentionally NOT matched —
        // substring-matching them adds noise ("Opulent Insolvent" ~ "solv").
        assert!(!sql.contains("coalesce(ae.name, '')"));
        // 3 needle placeholders (the shared code test + m.name + m.symbol);
        // the LIMIT is inlined, so this MUST equal the 3×`.bind(code)` in
        // `fetch_list`. A drift here is a runtime-only failure.
        assert_eq!(sql.matches('?').count(), 3);
    }

    #[test]
    fn native_is_matched_by_type_not_by_stored_code() {
        // Task 0470. Native XLM is stored with an EMPTY `asset_code`, so a bare
        // `positionCaseInsensitive(a.asset_code, 'XLM')` matched 6 404 credit
        // assets minted under a code containing "XLM" and missed the real one
        // (measured on production: the native row matched 0 times). Same defect
        // and same fix as the pools predicate — see the guard test beside it in
        // `common::pool_asset_codes`.
        //
        // Pinned on the SQL rather than on a result set because the CH-backed
        // tests only run with `CH_URL` set; this one runs everywhere.
        let params = ResolvedListParams {
            limit: 10,
            cursor: None,
            asset_type: None,
            asset_code: Some("XLM".to_string()),
            sac_only: false,
        };
        let sql = build_list_seek_sql(&params, Direction::Next);
        assert!(
            sql.contains(SHOWN),
            "the needle must be matched against the DISPLAYED code, so native \
             XLM is reachable; got: {sql}"
        );
        // The bare form is what made native unfindable — it must not come back.
        assert!(!sql.contains("positionCaseInsensitive(a.asset_code, ?)"));
    }

    #[test]
    fn the_walk_is_ascending_so_native_opens_the_list() {
        // Task 0485. Native XLM is `asset_type = 0` with an empty code — the
        // MINIMUM of the identity 4-tuple this keyset walks — so the old DESC
        // order buried it on the LAST page (measured on production: page 1 of
        // `filter[code]=xlm` was `zXLMr, zXLMr, zXLM, zXLM, zXLM`, and the
        // unfiltered list opened on codeless Soroban contracts).
        let mut params = ResolvedListParams {
            limit: 10,
            cursor: Some(AssetKeyCursor {
                asset_type: 1,
                asset_code: "XLM".to_string(),
                issuer_id: 1,
                contract_id: 0,
            }),
            asset_type: None,
            asset_code: Some("xlm".to_string()),
            sac_only: false,
        };
        for sql in [build_list_seek_sql(&params, Direction::Next), {
            params.asset_code = None;
            build_list_seek_sql(&params, Direction::Next)
        }] {
            assert!(sql.contains("ORDER BY a.asset_type ASC"), "{sql}");
            // The comparator must point INTO the walk, or the cursor pages
            // backwards on every "next".
            assert!(sql.contains(") > (?, ?, ?, ?)"), "{sql}");
            // No relevance machinery on this surface, by decision: ranking it
            // means carrying the rank in the cursor.
            assert!(!sql.contains("rank_tier"), "{sql}");
            assert!(!sql.contains("balance_aggregates"), "{sql}");
        }
    }

    #[test]
    fn list_sql_has_no_search_predicate_without_a_term() {
        let params = ResolvedListParams {
            limit: 10,
            cursor: None,
            asset_type: None,
            asset_code: None,
            sac_only: false,
        };
        let sql = build_list_seek_sql(&params, Direction::Next);
        assert!(!sql.contains("positionCaseInsensitive"));
        // No filter → pure `assets` PK walk: no side-table joins, no placeholders
        // (the LIMIT is inlined).
        assert!(!sql.contains("JOIN"));
        assert_eq!(sql.matches('?').count(), 0);
    }
}

/// Live-CH **decode** smoke for the asset-transactions keyset read.
///
/// The two-arm `UNION ALL` (task 0446) is the class of SQL an offline build
/// cannot validate: the outer `ORDER BY … LIMIT 1 BY … LIMIT` over a
/// parenthesised union parses only against a real server, and the unit tests
/// above assert its shape as a string, not that ClickHouse accepts it. The
/// other read modules guard their generated SQL the same way.
///
/// Exercises BOTH shapes — the lone arm (no associated contract) and the union
/// (an asset with one) — in both page directions.
///
/// **Skips cleanly when `CH_URL` is unset**, so CI (no CH access) is green.
/// Run against a reachable CH (local replica or SSH tunnel):
///
/// ```text
/// CH_URL=http://127.0.0.1:8123 CH_DATABASE=default \
///   cargo test -p api --lib assets::queries::decode_smoke -- --nocapture
/// ```
#[cfg(test)]
mod decode_smoke {
    use super::*;

    /// Bootstrap row: any asset, plus its associated contract surrogate if it
    /// has one (ADR 0051 — a type-3 token's own contract, else a SAC wrapper).
    #[derive(Debug, Row, Deserialize)]
    struct BootAssetRow {
        id: i64,
        contract_surrogate: i64,
    }

    /// Task 0485. The ranking that puts native XLM first is a SORT DIRECTION,
    /// and a direction is invisible to the SQL-shape tests — they pin the
    /// string, not what comes back. This runs the real read and looks at row 1.
    #[tokio::test]
    async fn code_search_returns_native_first() {
        let Some(ch) = crate::common::ch::test_client_from_env() else {
            eprintln!("CH_URL unset — skipping assets native-first smoke");
            return;
        };
        let has_native: u64 = ch
            .query("SELECT count() FROM assets WHERE asset_type = 0")
            .fetch_one()
            .await
            .expect("native probe must run");
        if has_native == 0 {
            eprintln!("no native row in this CH — native-first smoke not exercised");
            return;
        }

        let mut params = ResolvedListParams {
            limit: 10,
            cursor: None,
            asset_type: None,
            asset_code: Some("xlm".to_string()),
            sac_only: false,
        };
        let page = fetch_list(&ch, &params, Direction::Next)
            .await
            .expect("code-search page decodes");

        let first = page
            .first()
            .expect("a corpus with native XLM cannot be empty");
        assert_eq!(
            first.asset_type, 0,
            "`xlm` answered with {:?} first — native XLM is the MINIMUM of the \
             identity 4-tuple, so a DESC walk buries it on the last page",
            first.asset_code
        );

        // And with NO filter at all: the asset list of a Stellar explorer
        // opening on anything other than XLM was the same defect wearing a
        // different hat (it opened on codeless Soroban contracts).
        params.asset_code = None;
        let browse = fetch_list(&ch, &params, Direction::Next)
            .await
            .expect("unfiltered page decodes");
        assert_eq!(
            browse.first().map(|r| r.asset_type),
            Some(0),
            "the unfiltered list must open on native XLM; got {:?}",
            browse.first().map(|r| r.asset_code.clone())
        );
    }

    #[tokio::test]
    async fn asset_tx_keyset_union_decodes() {
        let Some(ch) = crate::common::ch::test_client_from_env() else {
            eprintln!("CH_URL unset — skipping asset-tx keyset decode smoke");
            return;
        };

        // An asset that HAS an associated contract, so the union arm is real.
        let boot = ch
            .query(
                "SELECT a.id AS id, max(sc.sac_contract_id) AS contract_surrogate \
                 FROM assets a INNER JOIN asset_sac sc \
                   ON sc.asset_type = a.asset_type AND sc.asset_code = a.asset_code \
                  AND sc.issuer_id = a.issuer_id AND sc.contract_id = a.contract_id \
                 WHERE sc.sac_contract_id != 0 \
                 GROUP BY a.id LIMIT 1",
            )
            .fetch_optional::<BootAssetRow>()
            .await
            .expect("bootstrap asset query must run");

        for direction in [Direction::Next, Direction::Prev] {
            // Lone arm: `None` skips the wrapper entirely.
            fetch_transactions(&ch, 1, None, 21, None, direction)
                .await
                .unwrap_or_else(|e| panic!("lone-arm keyset failed ({direction:?}): {e}"));

            if let Some(b) = &boot {
                fetch_transactions(&ch, b.id, Some(b.contract_surrogate), 21, None, direction)
                    .await
                    .unwrap_or_else(|e| panic!("union keyset failed ({direction:?}): {e}"));
            }
        }
        if boot.is_none() {
            eprintln!("no SAC-backed asset in this CH — union arm not exercised");
        }
    }
}
