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
    pub(crate) contract_id: i64,
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
    let rows = fetch_identity_rows(client, ids).await?;
    let issuers = resolve_issuers(client, &rows).await?;
    Ok(assemble(rows, &issuers))
}

/// The identity facts that need NO `accounts` seek — everything except a
/// classic asset's issuer StrKey.
///
/// For a caller that only NAMES assets (global search composes a pool's label
/// from [`leg_label`], which reads the code, the symbol and the contract), the
/// seek is a whole round trip whose result is discarded. Search fans its
/// buckets out concurrently and the slowest one sets the response time, so a
/// discarded round trip inside one bucket is paid in full.
pub(crate) async fn resolve_asset_identities_unlinked(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<HashMap<i64, ResolvedAsset>, clickhouse::error::Error> {
    let rows = fetch_identity_rows(client, ids).await?;
    Ok(assemble(rows, &HashMap::new()))
}

/// Identities AND their display facts, with the two independent follow-up
/// reads overlapped.
///
/// The `accounts` seek and the `asset_sac` / `asset_enrichment` reads both hang
/// off the statement above and neither depends on the other, so awaiting them
/// in sequence spends one round trip for nothing. Every surface that renders a
/// leg needs both halves, which is why they are one call rather than a rule
/// each call site has to remember.
pub(crate) async fn resolve_identities_and_display(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<(HashMap<i64, ResolvedAsset>, HashMap<i64, AssetDisplay>), clickhouse::error::Error> {
    let rows = fetch_identity_rows(client, ids).await?;
    let keys = display_keys(&rows);
    let (issuers, display) = futures::try_join!(
        resolve_issuers(client, &rows),
        resolve_display_for(client, &keys),
    )?;
    Ok((assemble(rows, &issuers), display))
}

/// A classic asset's link identity is `CODE-ISSUER`, and the issuer is a
/// surrogate on the row — resolved by the shared bloom seek rather than by an
/// `accounts` join, which would have to be bounded through `assets` and so
/// would cost the statement's scan a second time.
async fn resolve_issuers(
    client: &clickhouse::Client,
    rows: &[AssetIdentityChRow],
) -> Result<HashMap<i64, String>, clickhouse::error::Error> {
    super::ch::resolve_accounts(
        client,
        rows.iter()
            .filter(|r| r.known && r.asset_type == 1 && r.issuer_id != 0)
            .map(|r| r.issuer_id)
            .collect(),
    )
    .await
}

fn assemble(
    rows: Vec<AssetIdentityChRow>,
    issuers: &HashMap<i64, String>,
) -> HashMap<i64, ResolvedAsset> {
    rows.into_iter()
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
        .collect()
}

async fn fetch_identity_rows(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<Vec<AssetIdentityChRow>, clickhouse::error::Error> {
    let in_list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT ids.id                        AS id, \
                toBool(a.id != 0)             AS known, \
                a.asset_type                  AS asset_type, \
                nullIf(a.asset_code, '')      AS asset_code, \
                a.issuer_id                   AS issuer_id, \
                a.contract_id                 AS contract_id, \
                nullIf(sc.contract_id, '')    AS contract_strkey, \
                nullIf(m.symbol, '')          AS symbol, \
                coalesce(m.decimals, 7)       AS decimals \
         FROM (SELECT arrayJoin(CAST([{in_list}] AS Array(Int64))) AS id) ids \
         LEFT JOIN (SELECT id, asset_type, asset_code, issuer_id, contract_id \
                    FROM assets \
                    WHERE id IN ({in_list}) LIMIT 1 BY id) a ON a.id = ids.id \
         LEFT JOIN (SELECT id, contract_id FROM soroban_contracts \
                    WHERE id IN ({in_list}) LIMIT 1 BY id) sc ON sc.id = ids.id \
         LEFT JOIN (SELECT contract_id, \
                           argMax(symbol, version)   AS symbol, \
                           argMax(decimals, version) AS decimals \
                    FROM soroban_contract_metadata GROUP BY contract_id) m \
                ON m.contract_id = sc.contract_id"
    );

    client.query(&sql).fetch_all::<AssetIdentityChRow>().await
}

/// The identity tuple `asset_sac` and `asset_enrichment` key on.
type IdentityKey = (i16, String, i64, i64);

/// The rows that HAVE a display identity, paired with the tuple the side
/// tables key on.
///
/// An asset the dimension does not know carries no display extras and must not
/// widen the bounds — `known == false` is the NFT-collection case and the
/// unclassified-contract case, both real and both legitimately blank.
fn display_keys(rows: &[AssetIdentityChRow]) -> Vec<(i64, IdentityKey)> {
    rows.iter()
        .filter(|r| r.known)
        .map(|r| {
            (
                r.id,
                (
                    r.asset_type,
                    r.asset_code.clone().unwrap_or_default(),
                    r.issuer_id,
                    r.contract_id,
                ),
            )
        })
        .collect()
}

/// The two DISPLAY facts a pool leg renders and a balance-change cell does not:
/// where the asset page lives for a classic asset, and what avatar to draw.
#[derive(Debug, Clone, Default)]
pub(crate) struct AssetDisplay {
    /// Whether a SAC facet was OBSERVED for this asset (ADR 0051). The
    /// address itself is not stored and not looked up — derive it with
    /// [`sac_strkey`], which is what `/v1/assets` already does. Looking it up
    /// through `soroban_contracts` would be a second way to produce one fact.
    pub(crate) sac_observed: bool,
    /// From `asset_enrichment` (ADR 0050) — never from `assets`, whose
    /// `icon_url` column was dropped in task 0310 after measuring 0 of 411,654
    /// rows populated.
    pub(crate) icon_url: Option<String>,
}

#[derive(Debug, Row, Deserialize)]
struct SacFacetChRow {
    asset_type: i16,
    asset_code: String,
    issuer_id: i64,
    contract_id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct IconChRow {
    asset_type: i16,
    asset_code: String,
    issuer_id: i64,
    contract_id: i64,
    /// `Nullable(String)` on the table, so `argMax` over it is nullable too —
    /// decoding it as a bare `String` is refused by the driver, and only on a
    /// page that actually MATCHES an enrichment row (an empty result decodes
    /// nothing and passes). Every other reader of this column already declares
    /// it optional; this one had diverged.
    icon_url: Option<String>,
}

/// The two DISPLAY facts a pool leg renders and a balance-change cell does
/// not, for the identity tuples the side tables key on.
///
/// Taking the tuples rather than resolved assets is what lets the overlapped
/// path run this concurrently with the issuer seek — the tuple is complete on
/// the raw row, and the seek's result is not part of it. It is also why this is
/// a separate read rather than a flag on the identity statement: a flag would
/// make one function do two jobs, put the branch inside the statement, and hand
/// every call site a `true` nobody can read, while the account read, which
/// needs none of this, would carry the joins anyway.
///
/// It keys on the four-part identity tuple rather than the surrogate because
/// the side tables do. Re-deriving the tuple with a second `assets` read is the
/// double-scan the sibling statement's comment exists to prevent (209 ms /
/// 2.5M rows against 44 ms / 268k).
///
/// The two reads are independent and run concurrently. Both are bounded on all
/// three tuple components, so a Soroban asset (whose code is empty and whose
/// issuer is 0) cannot drag in every other Soroban asset's enrichment row.
///
/// No `sac_deployed` filter: the pools list does not apply one today, and this
/// is a move of that behaviour, not a change to it.
pub(crate) async fn resolve_display_for(
    client: &clickhouse::Client,
    known: &[(i64, IdentityKey)],
) -> Result<HashMap<i64, AssetDisplay>, clickhouse::error::Error> {
    if known.is_empty() {
        return Ok(HashMap::new());
    }

    let mut codes: Vec<String> = known.iter().map(|(_, k)| k.1.clone()).collect();
    codes.sort();
    codes.dedup();
    let mut issuers: Vec<i64> = known.iter().map(|(_, k)| k.2).collect();
    issuers.sort_unstable();
    issuers.dedup();
    let mut contracts: Vec<i64> = known.iter().map(|(_, k)| k.3).collect();
    contracts.sort_unstable();
    contracts.dedup();

    // Codes are bound; the two id lists are `i64` and inline by type.
    let placeholders = vec!["?"; codes.len()].join(",");
    let issuer_list = issuers
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let contract_list = contracts
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let bounds = format!(
        "asset_code IN ({placeholders}) AND issuer_id IN ({issuer_list}) \
         AND contract_id IN ({contract_list})"
    );

    let sac_sql = format!(
        "SELECT asset_type, asset_code, issuer_id, contract_id, \
         FROM asset_sac WHERE {bounds} \
         GROUP BY asset_type, asset_code, issuer_id, contract_id \
         HAVING max(sac_contract_id) != 0"
    );
    let icon_sql = format!(
        "SELECT asset_type, asset_code, issuer_id, contract_id, \
                argMax(icon_url, version) AS icon_url \
         FROM asset_enrichment WHERE {bounds} \
         GROUP BY asset_type, asset_code, issuer_id, contract_id"
    );

    let mut sac_q = client.query(&sac_sql);
    let mut icon_q = client.query(&icon_sql);
    for c in &codes {
        sac_q = sac_q.bind(c.as_str());
        icon_q = icon_q.bind(c.as_str());
    }
    let (sac_rows, icon_rows) = futures::try_join!(
        sac_q.fetch_all::<SacFacetChRow>(),
        icon_q.fetch_all::<IconChRow>(),
    )?;

    let sacs: std::collections::HashSet<IdentityKey> = sac_rows
        .into_iter()
        .map(|r| (r.asset_type, r.asset_code, r.issuer_id, r.contract_id))
        .collect();
    let icons: HashMap<IdentityKey, String> = icon_rows
        .into_iter()
        .filter_map(|r| {
            let url = r.icon_url.filter(|u| !u.is_empty())?;
            Some((
                (r.asset_type, r.asset_code, r.issuer_id, r.contract_id),
                url,
            ))
        })
        .collect();

    Ok(known
        .iter()
        .map(|(id, key)| {
            (
                *id,
                AssetDisplay {
                    sac_observed: sacs.contains(key),
                    icon_url: icons.get(key).cloned(),
                },
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "asset_identity_decode_smoke.rs"]
mod decode_smoke;

#[cfg(test)]
#[path = "asset_identity_tests.rs"]
mod tests;

/// The SAC mirror's `C…` StrKey for a classic or native asset.
///
/// **Derived, never stored and never looked up** (ADR 0051): the address is a
/// pure function of the asset and the network, so a table read would be a
/// second way to produce one fact — and the two ways would drift. `/v1/assets`
/// has always derived it; this is that call, in one place, so every surface
/// answers identically.
///
/// `observed` gates it: deriving unconditionally would hand out a link to a
/// contract that may not exist, which is the misleading-fallback class. The
/// caller passes what the dimension observed ([`AssetDisplay::sac_observed`],
/// or `sac_contract_surrogate != 0` on the assets read).
pub(crate) fn sac_strkey(
    observed: bool,
    code: &str,
    issuer: &str,
    network_id: &[u8; 32],
) -> Option<String> {
    observed
        .then(|| xdr_parser::derive_sac_strkey(code, issuer, network_id))
        .flatten()
}

/// The code an `assets` row DISPLAYS as, as a SQL expression over `alias`.
///
/// The `asset_type = 0` arm is load-bearing and is the whole reason this is one
/// function: native carries an EMPTY code on the ledger, so a plain column
/// match returns only the credit assets minted under the code `XLM` — a wrong
/// answer that looks like a right one (task 0440). Three modules matched and
/// ranked on this rule with three copies of the expression, each carrying a
/// "change one, change all three" note; the pools copy lost its note when the
/// pair columns died, at which point nothing tied them together.
///
/// `alias` is the table alias plus its dot (`"a."`), or empty when the query
/// does not alias `assets`.
pub(crate) fn shown_code_sql(alias: &str) -> String {
    format!("lower(if({alias}asset_type = 0, 'XLM', toString({alias}asset_code)))")
}

/// What one leg is CALLED, for a label the API composes itself.
///
/// The ladder, in the only order that never invents a name: a classic asset by
/// its code (native's code is `XLM`, which the ledger does not store), a
/// Soroban token by its self-declared symbol, and otherwise by the truncated
/// contract address — the identity that always exists.
///
/// The last rung is not a fallback, it is the honest answer: a Soroban token's
/// symbol is self-declared and NOT unique (measured on production: 2,276
/// contracts call themselves `SMOL`, and three distinct contracts among our own
/// pool legs claim `USDC`), so the address is the only thing that
/// discriminates. An asset the dimension does not know at all reaches the same
/// rung, for the same reason.
pub(crate) fn leg_label(identity: Option<&ResolvedAsset>) -> String {
    let Some(r) = identity else {
        return String::from("unknown");
    };
    if r.known && r.asset_type == domain::AssetFamily::Native as i16 {
        return String::from("XLM");
    }
    if let Some(code) = r.asset_code.as_deref().filter(|c| !c.is_empty()) {
        return code.to_string();
    }
    if let Some(sym) = r.symbol.as_deref().filter(|s| !s.is_empty()) {
        return sym.to_string();
    }
    match r.contract_strkey.as_deref() {
        Some(c) if c.len() > 8 => format!("{}…{}", &c[..4], &c[c.len() - 4..]),
        Some(c) => c.to_string(),
        None => String::from("unknown"),
    }
}
