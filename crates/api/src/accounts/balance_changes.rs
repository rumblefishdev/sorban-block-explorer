//! `Balance change` on the account transaction list (task 0540) — how much the
//! account **in context** received and spent on each transaction.
//!
//! The figure is relative to one account: the same transaction seen from two
//! accounts shows two different numbers, and that is the point. It is why the
//! column lives on the account page alone and never on the global transaction
//! list, the ledger page or the asset page (T07) — a balance change with
//! nobody to ask the question is exactly what the retired `net_settled` column
//! got wrong.
//!
//! ## Where the numbers come from
//!
//! `asset_transfers` holds ONE ROW PER TOKEN MOVEMENT — an edge (`from`, `to`,
//! asset, amount), not a per-transaction aggregate. The account page already
//! holds `(ledger_sequence, application_order)` for its page after step 2 of
//! [`super::queries::fetch_transactions`], and that pair is the table's sort-key
//! prefix, so this is a seek, not a scan.
//!
//! Three things the read cannot drop:
//!
//! 1. **Dedup is mandatory.** `asset_transfers` is a `ReplacingMergeTree` with
//!    no version column, and production tables in this project carry unmerged
//!    duplicates permanently (task 0420). The inner `GROUP BY` over the FULL
//!    sort key collapses them; without it a duplicated row doubles the amount
//!    on screen. (`FINAL` also works and measured faster locally — on ONE part,
//!    which flatters it. Left as the explicit group-by until it is measured on
//!    real production parts.)
//! 2. **The partition prune.** `PARTITION BY intDiv(ledger_sequence, 500000)`
//!    — without the matching `intDiv(...) IN (…)` the key filter alone does not
//!    prune.
//! 3. **A transfer to self nets to zero.** `to − from` per row, not a
//!    first-match branch: with `from_id = to_id = account` both legs fire and
//!    cancel. That is the correct answer, not a defect.
//!
//! ## What `NULL` means, and what an empty list means
//!
//! `amount` is `NULL` for exactly one reason: a **non-fungible** movement,
//! where no amount exists by nature (the event payload carries a `{token_id}`,
//! not a value). It is a complete statement about what happened, not a gap in
//! our data — so it is carried as [`BalanceChange::nft_delta`] (signed count of
//! pieces, `+` received / `−` sent) with `amount = None`, and must never render
//! as `0`. The piece's own identity is NOT in this table; `nfts` /
//! `nft_ownership` are the source for "which piece".
//!
//! Separately, and far more consequential today: below
//! [`VALUE_FLOW_FLOOR_LEDGER`] the table holds NO rows at all, which looks
//! exactly like a transaction that moved nothing. The API therefore returns
//! `None` (not an empty list) for those transactions, so the frontend can say
//! "not indexed" instead of drawing a zero that is not a measurement.

use std::collections::{BTreeSet, HashMap};

use clickhouse::Row;
use serde::Deserialize;

/// First ledger `asset_transfers` covers — the ledger the live indexer started
/// writing it (deploy `production-2026.09.07-1`). Below it the table is empty
/// because nothing has been written yet, NOT because nothing moved, so the API
/// reports "not indexed" rather than a balance change of zero.
///
/// It drops to the ingest floor (50 457 424) once the historical backfill of
/// `50 457 424 .. 64 317 019` passes its coverage gate, and stays there for
/// good: 78.85% of chain history predates the ingest floor and no re-parse can
/// reach it.
///
/// ponytail: a `const`, not config — the only way to change it is a deploy
/// either way (env vars come from the CDK compute stack), so an env read would
/// buy nothing. Promote it if it ever has to move without one.
pub const VALUE_FLOW_FLOOR_LEDGER: i64 = 64_317_019;

/// One asset's net movement for the account in context, on one transaction.
/// Position in the vector is the order the movement happened on the chain —
/// never a ranking (see [`balance_change_delta_sql`]).
#[derive(Debug, Clone)]
pub struct BalanceChange {
    /// Asset identity accepted by `parse_asset_id` — `"native"`,
    /// `"CODE-ISSUER"`, or a bespoke token's `C…` contract StrKey. The
    /// frontend links to it. Empty only when the asset resolves to nothing at
    /// all (an unregistered contract), in which case the cell shows the code
    /// without a link rather than a dead one.
    pub asset: String,
    /// Display code — `asset_code` for classic, the on-chain `symbol` for a
    /// bespoke token, `None` for native (render as XLM) or when neither exists.
    pub asset_code: Option<String>,
    /// Display decimals — 7 for classic/native/SAC, on-chain `METADATA` for a
    /// bespoke token.
    pub decimals: u32,
    /// SIGNED raw amount for the account in context (`+` in, `−` out), as an
    /// `Int128` string. `None` = non-fungible movement, no amount exists —
    /// see `nft_delta`, and never render it as zero.
    pub amount: Option<String>,
    /// Signed count of non-fungible pieces moved (`+1` received, `−1` sent).
    /// `0` for an ordinary fungible asset.
    pub nft_delta: i64,
    /// WHICH piece moved — the contract-defined token id (`"44"`). A bulk
    /// move becomes one `BalanceChange` per piece, each naming its own, so
    /// every NFT is listed and linked separately. `None` when the pieces
    /// cannot be named: a fungible asset, a collection quarantined in
    /// `nft_ownership_pending` (the API never reads that table), or a set
    /// whose size contradicts the movement count.
    pub token_id: Option<String>,
}

/// One page transaction, by the three ids this read needs: the pair that seeks
/// `asset_transfers` and the transaction id that names a non-fungible piece in
/// `nft_ownership`. The caller holds all three after its own step 2.
#[derive(Debug, Clone, Copy)]
pub struct TxKey {
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub transaction_id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct DeltaChRow {
    ledger_sequence: i64,
    application_order: i16,
    asset_id: i64,
    /// `toString`-encoded to avoid an `Int128` wire decode. Nullable because
    /// `sum()` over a `Nullable` column is `Nullable` — decoding it into a
    /// non-nullable field 500s (the trap that took account-detail down, 0324).
    /// `None` here means every row for this asset was non-fungible.
    delta: Option<String>,
    nft_delta: i64,
    /// The new owner of the piece(s) this row covers — the edge's `to_id`, and
    /// the key that names them in `nft_ownership`. `None` for a burn (no new
    /// owner) and for every fungible row.
    nft_owner: Option<i64>,
}

#[derive(Debug, Row, Deserialize)]
struct AssetIdentityChRow {
    id: i64,
    /// `false` = no `assets` row. NOT an error: the three busiest NFT
    /// collections on production have zero rows there, so an `INNER JOIN`
    /// would silently drop every one of their transfers. The contract
    /// surrogate IS the `asset_id` in that case, which is what the fallback
    /// leg below resolves.
    known: bool,
    asset_type: i16,
    asset_code: Option<String>,
    issuer_id: i64,
    contract_strkey: Option<String>,
    symbol: Option<String>,
    decimals: u32,
}

/// Net per-asset balance change for the account in context, for a bounded page
/// of `(ledger_sequence, application_order)` keys, each transaction's assets in
/// the order their movements occurred.
///
/// Returns a map keyed by that pair. A transaction that moved nothing for this
/// account is simply absent — the caller renders the empty case, and
/// distinguishes it from "below the floor" itself.
pub async fn fetch_balance_changes(
    client: &clickhouse::Client,
    account_id: i64,
    keys: &[TxKey],
) -> Result<HashMap<(i64, i16), Vec<BalanceChange>>, clickhouse::error::Error> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    // Keys are integers, inlined directly — no injection surface, and it
    // sidesteps binding a tuple array (the clickhouse 0.15 defect that forced
    // the keyset statements to inline).
    let in_tuples = keys
        .iter()
        .map(|k| format!("({},{})", k.ledger_sequence, k.application_order))
        .collect::<Vec<_>>()
        .join(",");
    let partitions = keys
        .iter()
        .map(|k| k.ledger_sequence / 500_000)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let delta_sql = balance_change_delta_sql(account_id, &in_tuples, &partitions);

    let delta_rows = client.query(&delta_sql).fetch_all::<DeltaChRow>().await?;
    if delta_rows.is_empty() {
        return Ok(HashMap::new());
    }

    let asset_ids: BTreeSet<i64> = delta_rows.iter().map(|r| r.asset_id).collect();
    let identities = resolve_asset_identities(client, &asset_ids).await?;

    // Which PIECE moved. Only asked when the page actually carries a
    // non-fungible entry — one row in 2 256 264 on production today — so this
    // statement usually does not run at all.
    let tx_by_pair: HashMap<(i64, i16), i64> = if delta_rows.iter().any(|r| r.nft_delta != 0) {
        keys.iter()
            .map(|k| ((k.ledger_sequence, k.application_order), k.transaction_id))
            .collect()
    } else {
        HashMap::new()
    };
    let nft_lookups: BTreeSet<(i64, i64)> = delta_rows
        .iter()
        .filter(|r| r.nft_delta != 0)
        .filter_map(|r| {
            tx_by_pair
                .get(&(r.ledger_sequence, r.application_order))
                .map(|tx| (r.asset_id, *tx))
        })
        .collect();
    let pieces = resolve_moved_pieces(client, &nft_lookups).await?;

    let mut out: HashMap<(i64, i16), Vec<BalanceChange>> = HashMap::new();
    for row in delta_rows {
        let identity = identities.get(&row.asset_id);
        let (asset, asset_code, decimals) = identity.map_or_else(
            // Unresolvable asset: no link, no code, classic decimals. It
            // cannot happen through the query above (every id yields a row),
            // and is here so a future caller cannot silently get a wrong scale.
            || (String::new(), None, 7),
            |i| {
                // A NON-FUNGIBLE entry keeps its contract StrKey whatever
                // `assets` knows: the cell sends it to the NFT pages, which are
                // keyed on the contract and answer for collections `assets` has
                // never heard of — that is the whole reason this read joins
                // `assets` LEFT. A FUNGIBLE entry goes to `/assets/{id}`, so it
                // is linked only when that page can answer; otherwise the
                // identity is emitted EMPTY and the cell prints the code as
                // plain text. Measured on production: 4 such assets in one
                // historical partition, 0 in the live one — small, but a dead
                // link is exactly the plausible-looking wrongness this column
                // exists to avoid.
                let linkable = row.nft_delta != 0 || i.resolves_on_asset_page;
                let asset = if linkable {
                    i.asset.clone()
                } else {
                    String::new()
                };
                (asset, i.asset_code.clone(), i.decimals)
            },
        );
        // A non-fungible group is EXPANDED into one entry per piece when the
        // pieces can be named — that is what makes each NFT in a bulk move its
        // own clickable row rather than a `+3 NFT` lump.
        //
        // The count is the proof, and it is not ceremony: `nft_ownership` is
        // joined on the new OWNER, so a transaction where somebody else also
        // moved pieces to that same owner would hand back more ids than this
        // account moved. When the set size and the movement count disagree the
        // entry stays collapsed with no id — a coarser answer, never a wrong
        // one. `nft_delta` also guards the fungible case: one contract emitting
        // both shapes shares an `asset_id`, so without it a fungible entry
        // would pick up its non-fungible sibling's ids.
        let moved = usize::try_from(row.nft_delta.unsigned_abs()).unwrap_or(usize::MAX);
        let pieces_here = (row.nft_delta != 0)
            .then(|| {
                tx_by_pair
                    .get(&(row.ledger_sequence, row.application_order))
                    .and_then(|tx| pieces.get(&(row.asset_id, *tx, row.nft_owner)))
            })
            .flatten()
            .filter(|tokens| tokens.len() == moved)
            .cloned()
            .unwrap_or_default();

        let entry =
            |amount: Option<String>, nft_delta: i64, token_id: Option<String>| BalanceChange {
                asset: asset.clone(),
                asset_code: asset_code.clone(),
                decimals,
                amount,
                nft_delta,
                token_id,
            };
        let slot = out
            .entry((row.ledger_sequence, row.application_order))
            .or_default();
        if pieces_here.is_empty() {
            slot.push(entry(row.delta, row.nft_delta, None));
        } else {
            // One entry per piece, each carrying the direction of the group it
            // came from.
            let direction = row.nft_delta.signum();
            slot.extend(
                pieces_here
                    .into_iter()
                    .map(|token| entry(None, direction, Some(token))),
            );
        }
    }

    // NOT re-sorted here: the statement already returns each transaction's
    // assets in the order their movements occurred, and re-ordering in Rust
    // would silently override it.
    Ok(out)
}

struct AssetIdentity {
    asset: String,
    asset_code: Option<String>,
    decimals: u32,
    /// Whether `/assets/{asset}` can actually answer for this identity.
    ///
    /// A bespoke token's link is its contract StrKey, and `soroban_contracts`
    /// has a row for EVERY deployed contract — but the asset endpoint hydrates
    /// the key `(3, '', 0, surrogate)` out of `assets`, so a token nobody
    /// registered there answers 404. The two conditions are not the same
    /// question, and reading the StrKey's presence as if it were the second one
    /// is what produced a live-looking link to a page that does not exist.
    resolves_on_asset_page: bool,
}

#[derive(Debug, Row, Deserialize)]
struct MovedPieceChRow {
    contract_id: i64,
    transaction_id: i64,
    owner_id: Option<i64>,
    /// Every token id this transaction moved to this owner in this collection.
    token_ids: Vec<String>,
}

/// Which non-fungible PIECES each `(collection, transaction, new owner)` moved.
///
/// `asset_transfers` deliberately does not carry the token id — it is an edge
/// table, and the id belongs to the piece, not to the movement. `nft_ownership`
/// is where it lives, keyed `(contract_id, token_id, ledger_sequence,
/// event_order)`, so this filters on the leading `contract_id` and reads at
/// most that one collection's rows (17 816 for the largest on production;
/// measured 18 ms / 1 509 rows).
///
/// Grouped by OWNER because that is the only join the data supports: a bulk
/// transfer writes ten ownership rows that share an owner and carry
/// `event_order = 0` on every one of them (measured), so nothing pairs a single
/// piece with a single edge. The owner does pair a SET of pieces with a set of
/// edges, and the caller checks that set's size against the movement count
/// before showing any of it.
async fn resolve_moved_pieces(
    client: &clickhouse::Client,
    lookups: &BTreeSet<(i64, i64)>,
) -> Result<HashMap<(i64, i64, Option<i64>), Vec<String>>, clickhouse::error::Error> {
    if lookups.is_empty() {
        return Ok(HashMap::new());
    }
    let contracts = lookups
        .iter()
        .map(|(c, _)| c.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    let txs = lookups
        .iter()
        .map(|(_, t)| t.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT contract_id, transaction_id, owner_id, \
                groupUniqArray(token_id) AS token_ids \
         FROM nft_ownership \
         WHERE contract_id IN (CAST([{contracts}] AS Array(Int64))) \
           AND transaction_id IN (CAST([{txs}] AS Array(Int64))) \
         GROUP BY contract_id, transaction_id, owner_id"
    );
    Ok(client
        .query(&sql)
        .fetch_all::<MovedPieceChRow>()
        .await?
        .into_iter()
        .map(|r| {
            let mut tokens = r.token_ids;
            // Deterministic order: the cell lists these one per line, and a
            // page boundary must not reshuffle them.
            tokens.sort_unstable();
            ((r.contract_id, r.transaction_id, r.owner_id), tokens)
        })
        .collect())
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
async fn resolve_asset_identities(
    client: &clickhouse::Client,
    ids: &BTreeSet<i64>,
) -> Result<HashMap<i64, AssetIdentity>, clickhouse::error::Error> {
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
    let issuers = crate::common::ch::resolve_accounts(
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
            // `parse_asset_id`'s three accepted forms, by `TokenAssetType`:
            // 0 native, 1 classic credit, 3 bespoke Soroban. An asset with no
            // `assets` row is a bespoke token by construction, so it takes the
            // same branch as type 3.
            let (asset, asset_code) = match (r.known, r.asset_type) {
                (true, 0) => ("native".to_string(), None),
                (true, 1) => (
                    match (
                        r.asset_code.as_deref(),
                        issuers.get(&r.issuer_id).filter(|s| !s.is_empty()),
                    ) {
                        (Some(code), Some(issuer)) => format!("{code}-{issuer}"),
                        // No issuer StrKey means no link identity exists; the
                        // code still names the asset in the cell.
                        _ => String::new(),
                    },
                    r.asset_code.clone(),
                ),
                _ => (
                    r.contract_strkey.clone().unwrap_or_default(),
                    r.symbol.clone().or_else(|| r.asset_code.clone()),
                ),
            };
            (
                r.id,
                AssetIdentity {
                    asset,
                    asset_code,
                    decimals: r.decimals,
                    resolves_on_asset_page: r.known,
                },
            )
        })
        .collect())
}

/// The per-transaction, per-asset signed sum for one account.
///
/// `to − from` per deduplicated edge, summed per (transaction, asset).
/// `HAVING` drops assets that net to nothing: under a column called
/// `Balance change`, an asset whose balance did not change is not one. It is
/// also what makes the adversarial row readable — a six-hop arbitrage nets
/// exactly zero in eight assets and profits in one, and the profit is the
/// whole content of the row.
///
/// **Order is the order the movements happened in**, `min((op_index,
/// event_pos_in_op))` — the position of an asset's FIRST edge inside the
/// transaction. It is a fact the chain supplies, and the only ordering
/// available that is one: assets have different decimals and different prices,
/// so ranking them by amount would be comparing quantities that are not
/// comparable, and we have no prices to make them so (nothing in ClickHouse
/// carries one). An earlier version ranked by scaled magnitude and was dropped
/// for exactly that reason.
///
/// **Non-fungible movements group by their NEW OWNER** (`nft_owner`, the
/// edge's `to_id`; `NULL` for a burn, and `NULL` throughout for a fungible
/// asset so those still aggregate per asset). That is what lets a bulk move be
/// listed piece by piece: `nft_ownership` records the new owner, so the group's
/// `(collection, transaction, owner)` names the exact set of pieces that moved,
/// and its own count verifies the set before any of it is shown.
///
/// It has to be the owner rather than the event position, which was measured
/// and does not work: `nft_ownership.event_order` is `0` on every row of a
/// ten-piece transfer, so nothing there pairs one piece with one edge.
///
/// **Fungible and non-fungible movements of the SAME asset are separate rows**
/// — `is_non_fungible` is part of the grouping key. `asset_id` is the EMITTING
/// CONTRACT's surrogate, so one contract that emits both an `{amount}` and a
/// `{token_id}` transfer shares an id between them; without the split,
/// ClickHouse's `sum()` (which skips NULLs) would return a real `delta`
/// alongside a non-zero `nft_delta` on one row. The cell branches on "is there
/// an amount", so the piece movement would vanish — and if the fungible legs
/// happened to net out, the row would survive `HAVING` and print `0` for a
/// transaction that changed an owner. Splitting makes the DTO's invariant true
/// by construction: an entry is fungible or non-fungible, never both.
///
/// Extracted so its load-bearing clauses are assertable without a live
/// ClickHouse: the self-transfer subtraction, the RMT dedup, the partition
/// prune and the non-fungible split are each one word away from a regression
/// that review does not catch.
fn balance_change_delta_sql(account_id: i64, in_tuples: &str, partitions: &str) -> String {
    format!(
        "SELECT ledger_sequence, \
                application_order, \
                asset_id, \
                toString(sum(signed)) AS delta, \
                sum(nft) AS nft_delta, \
                nft_owner \
         FROM ( \
             SELECT ledger_sequence, application_order, op_index, event_pos_in_op, asset_id, \
                    any(amount IS NULL) AS is_non_fungible, \
                    any(if(amount IS NULL, to_id, NULL)) AS nft_owner, \
                    any(if(to_id = {account_id}, amount, 0) \
                        - if(from_id = {account_id}, amount, 0)) AS signed, \
                    any(if(amount IS NULL, \
                           if(to_id = {account_id}, 1, 0) - if(from_id = {account_id}, 1, 0), \
                           0)) AS nft \
             FROM asset_transfers \
             WHERE (ledger_sequence, application_order) IN ({in_tuples}) \
               AND intDiv(ledger_sequence, 500000) IN ({partitions}) \
               AND (from_id = {account_id} OR to_id = {account_id}) \
             GROUP BY ledger_sequence, application_order, op_index, event_pos_in_op, asset_id \
         ) \
         GROUP BY ledger_sequence, application_order, asset_id, is_non_fungible, nft_owner \
         HAVING (delta IS NOT NULL AND delta != '0') OR nft_delta != 0 \
         ORDER BY ledger_sequence, application_order, \
                  min((op_index, event_pos_in_op))"
    )
}

#[cfg(test)]
#[path = "balance_changes_tests.rs"]
mod tests;

/// Wire-type contract, asserted against a REAL ClickHouse — see the module
/// docs in the file itself.
#[cfg(test)]
#[path = "balance_changes_decode_smoke.rs"]
mod decode_smoke;
