//! `assets.id` surrogate -> the identity a page can render.
//!
//! The fourth instantiation of the id-IN resolver pattern (tasks 0344 / 0345)
//! and the first one shared: `resolve_accounts` and `resolve_contracts` each
//! map a surrogate to ONE StrKey, while an asset needs its whole identity —
//! family, code, issuer, contract, symbol, decimals. Task 0540 wrote this for
//! the account value-flow read; task 0374 needs the same resolution for
//! liquidity-pool legs, which is the "third consumer needing richer fields"
//! that the domain map held the consolidation behind.
//!
//! It is shared rather than copied because the statement below is not obvious
//! code — every shape in it is a paid-for lesson, and a second copy would drift
//! away from them one edit at a time:
//!
//! 1. The `assets` join is LEFT, or every NFT collection's transfers vanish.
//! 2. The contract leg keys on the SAME id list, not through
//!    `assets.contract_id` — that shape ran the scan-only `assets` leg twice
//!    (209 ms / 2.5M rows against 44 ms / 268k).
//! 3. `toBool(...)`, because a bare comparison is `UInt8` on the wire and the
//!    driver wants CH `Bool` — the mismatch class that took account detail down.
//! 4. `CAST(… AS Array(Int64))`, because ClickHouse types an array literal from
//!    its values and an all-positive page yields `Array(UInt64)` — a 500 on
//!    exactly those accounts and no others.
//! 5. `LIMIT 1 BY id` / `argMax(…, version)` instead of `FINAL`, measured at
//!    4.7x fewer rows.
//!
//! The output is the RAW identity. Each consumer projects it into its own wire
//! shape (the account read builds a `CODE-ISSUER` link, a pool leg builds an
//! avatar) — that projection is a surface concern and stays with the surface.

use std::collections::{BTreeSet, HashMap};

use clickhouse::Row;
use serde::Deserialize;

#[derive(Debug, Row, Deserialize)]
pub(crate) struct AssetIdentityChRow {
    pub(crate) id: i64,
    /// `false` = no `assets` row. NOT an error: the three busiest NFT
    /// collections on production have zero rows there, so an `INNER JOIN`
    /// would silently drop every one of their transfers. The contract
    /// surrogate IS the `asset_id` in that case, which is what the fallback
    /// leg below resolves.
    pub(crate) known: bool,
    pub(crate) asset_type: i16,
    pub(crate) asset_code: Option<String>,
    pub(crate) issuer_id: i64,
    pub(crate) contract_strkey: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) decimals: u32,
}

/// One asset's identity as the dimension knows it, with the issuer StrKey
/// already resolved so a caller never has to make a second batched lookup.
///
/// `known == false` is a real state, not an error: `assets` only gets a row for
/// a contract classified `Fungible`, so an NFT collection and any contract the
/// classifier could not identify legitimately have none. Render that absence —
/// never substitute a plausible identity for it.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedAsset {
    pub(crate) known: bool,
    /// `assets.asset_type` — the project's AssetFamily domain (0 native,
    /// 1 classic credit, 3 Soroban token), NOT the XDR `AssetType`.
    pub(crate) asset_type: i16,
    pub(crate) asset_code: Option<String>,
    /// `G…` StrKey of the issuer, when there is one and it resolves.
    pub(crate) issuer: Option<String>,
    pub(crate) contract_strkey: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) decimals: u32,
}

/// Resolve a bounded set of `asset_transfers.asset_id` surrogates to a link
/// identity + display code + decimals.
///
/// The `assets` join is a **LEFT** join on purpose. `asset_id` is the emitting
/// contract's surrogate for a bespoke token, and a token nobody registered has
/// no `assets` row at all — measured on production, the three busiest NFT
/// collections (17 816 / 1 541 / 900 ownership rows) have none, so an inner
/// join would drop every transfer they ever made without a trace.
///
/// **The contract leg needs no `assets` row either**, which is what keeps this
/// to one scan: a Soroban asset's surrogate IS its contract's
/// (`assets.id = assets.contract_id` for 4 422 of 4 422 type-3 rows on
/// production; types 0 and 1 have no contract at all), so `soroban_contracts`
/// is seeked on the same id list whether or not `assets` knew the asset. An
/// earlier shape joined it through `assets.contract_id`, which forced the
/// scan-only `assets` leg to run TWICE — measured 209 ms / 2.5M rows against
/// 44 ms / 268k for this one, per page view.
///
/// `assets.id` carries no skip index (`id` is not in its `ORDER BY`), so its
/// leg is a scan; `soroban_contracts.id` and `accounts.id` are bloom-indexed
/// granule seeks, and `soroban_contract_metadata` is 3 927 rows.
/// `FINAL` is replaced by `LIMIT 1 BY id` / `argMax(…, version)` throughout —
/// exact here for the same reason as task 0344, and `FINAL` on these
/// dimensions measured 4.7x the rows read.
///
/// `toBool(...)` on `known`, not the bare comparison: `a.id != 0` is `UInt8`
/// on the wire and the driver decodes a Rust `bool` from CH `Bool`. The same
/// class of mismatch (a `Nullable` aggregate into a non-nullable field) took
/// account-detail down in task 0324.
///
/// **`CAST(… AS Array(Int64))` around the id list, not a bare literal array.**
/// ClickHouse infers an array literal's element type from its VALUES, so a page
/// whose asset ids all happen to be positive yields `Array(UInt64)` and the
/// `id` column decodes as `UInt64` into `i64` — a 500 on that account's page
/// and on no other. Caught on production
/// (`GBO56XB4…`, whose only asset is `XTAR` at id 8106068169672383637); every
/// earlier test happened to include native, whose surrogate is negative.
pub(crate) async fn resolve_asset_identities(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<HashMap<i64, ResolvedAsset>, clickhouse::error::Error> {
    let in_list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT ids.id                        AS id, \
                toBool(a.id != 0)             AS known, \
                a.asset_type                  AS asset_type, \
                nullIf(a.asset_code, '')      AS asset_code, \
                a.issuer_id                   AS issuer_id, \
                nullIf(sc.contract_id, '')    AS contract_strkey, \
                nullIf(m.symbol, '')          AS symbol, \
                coalesce(m.decimals, 7)       AS decimals \
         FROM (SELECT arrayJoin(CAST([{in_list}] AS Array(Int64))) AS id) ids \
         LEFT JOIN (SELECT id, asset_type, asset_code, issuer_id FROM assets \
                    WHERE id IN ({in_list}) LIMIT 1 BY id) a ON a.id = ids.id \
         LEFT JOIN (SELECT id, contract_id FROM soroban_contracts \
                    WHERE id IN ({in_list}) LIMIT 1 BY id) sc ON sc.id = ids.id \
         LEFT JOIN (SELECT contract_id, \
                           argMax(symbol, version)   AS symbol, \
                           argMax(decimals, version) AS decimals \
                    FROM soroban_contract_metadata GROUP BY contract_id) m \
                ON m.contract_id = sc.contract_id"
    );

    let rows = client.query(&sql).fetch_all::<AssetIdentityChRow>().await?;

    // A classic asset's link identity is `CODE-ISSUER`, and the issuer is a
    // surrogate here — resolved by the shared bloom seek rather than by an
    // `accounts` join, which would have to be bounded through `assets` and so
    // would cost the scan above a second time.
    let issuers = super::ch::resolve_accounts(
        client,
        rows.iter()
            .filter(|r| r.known && r.asset_type == 1 && r.issuer_id != 0)
            .map(|r| r.issuer_id)
            .collect(),
    )
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let issuer = issuers.get(&r.issuer_id).filter(|s| !s.is_empty()).cloned();
            (
                r.id,
                ResolvedAsset {
                    known: r.known,
                    asset_type: r.asset_type,
                    asset_code: r.asset_code,
                    issuer,
                    contract_strkey: r.contract_strkey,
                    symbol: r.symbol,
                    decimals: r.decimals,
                },
            )
        })
        .collect())
}
