//! ClickHouse writer module — populates the 17 production-schema
//! tables from parser output.
//!
//! Architecture (task 0206, hybrid-surrogate redesign):
//!
//! * [`stage::prepare`] — synchronous pre-write transform. Reads the
//!   same `Extracted*` slices PG does and produces CH-shaped row
//!   structs from [`rows`]. FK columns use a **hybrid key strategy**:
//!   three high-fan-out hubs (`accounts`, `soroban_contracts`,
//!   `transactions`) carry a deterministic surrogate `Int64 id`
//!   derived via [`ids`] (`cityhash_102_128` lower 64 bits over the
//!   StrKey / hash), keeping referencing tables narrow; the other 12
//!   tables use natural composite keys (StrKey / hash / tuple) on
//!   their PK and reference the hubs by surrogate. Surrogates are
//!   derived synchronously from the source key, so the writer is
//!   still single-pass and fully deterministic across replays.
//!   `LowCardinality(String)` dictionary-encodes per-block-repeated
//!   StrKeys for free on the natural-key columns.
//! * [`writer::PartitionWriter`] — streams rows into long-lived
//!   `clickhouse::Insert<RowT>` handles, one per table per backfill
//!   partition. Implements the commit-marker pattern so a partial
//!   run leaves the partition resumable. See module docs for the
//!   "why per-ledger inserts are wrong" rationale.
//! * [`persist_ledger_clickhouse`] — thin wrapper around a one-shot
//!   `PartitionWriter` (open → write_ledger → commit). Kept for
//!   legacy / single-ledger callers (tests, the runner's
//!   `Sink::persist_ledger` fallback). Production backfill drives
//!   `PartitionWriter` directly via the runner's partition-writer
//!   lifecycle.

use std::collections::{HashMap, HashSet};

use clickhouse::Client;
use domain::{ContractEventType, ContractType};
// Re-export the canonical (domain-owned) cache so `db_clickhouse::persist::
// ClassificationCache` stays a valid path for callers + integration tests that
// don't depend on `domain` directly (task 0283).
pub use domain::ClassificationCache;
use xdr_parser::event::extract_executable_update_new_wasm_hash;
use xdr_parser::types::{
    ContractFunction, EventSource, ExtractedAccountState, ExtractedAsset,
    ExtractedContractDeployment, ExtractedContractInterface, ExtractedEvent, ExtractedInvocation,
    ExtractedLedger, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedLpPosition,
    ExtractedNft, ExtractedNftEvent, ExtractedOperation, ExtractedSorobanBalance,
    ExtractedTransaction,
};
use xdr_parser::{SacOverride, classify_contract_from_wasm_spec};

use crate::SchemaError;

pub mod enrichment;
pub mod ids;
pub mod rows;
pub mod stage;
pub mod writer;

#[cfg(test)]
mod tests_cross;

pub use writer::PartitionWriter;

/// Per-ledger persist entrypoint kept for legacy callers and one-shot
/// use cases (tests, the single-ledger path on the `Sink` enum). Wraps
/// a one-shot [`PartitionWriter`] so the per-ledger semantics are
/// preserved end-to-end (open → write → commit) even though production
/// backfill goes through the partition-writer lifecycle directly.
///
/// Real production callers should drive [`PartitionWriter`] across the
/// whole backfill partition — per-ledger opens defeat the part-economy
/// design (see [`writer`] module docs). This wrapper exists so the
/// 0205-era signature stays callable.
#[allow(clippy::too_many_arguments)]
pub async fn persist_ledger_clickhouse(
    client: &Client,
    ledger: &ExtractedLedger,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
    contract_interfaces: &[ExtractedContractInterface],
    contract_deployments: &[ExtractedContractDeployment],
    account_states: &[ExtractedAccountState],
    liquidity_pools: &[ExtractedLiquidityPool],
    pool_snapshots: &[ExtractedLiquidityPoolSnapshot],
    assets: &[ExtractedAsset],
    nfts: &[ExtractedNft],
    nft_events: &[ExtractedNftEvent],
    lp_positions: &[ExtractedLpPosition],
    contract_metadata_writes: &[xdr_parser::ExtractedContractMetadata],
    soroban_token_balances: &[xdr_parser::ExtractedSorobanBalance],
    pool_family_writes: &[xdr_parser::pool_family::PoolFamilyWrite],
    sac_overrides: &[SacOverride],
    classification_cache: &ClassificationCache,
) -> Result<(), SchemaError> {
    // Task 0283 live cross-ledger verdict resolution. Two independent lookups
    // (different tables, different keys, neither consumes the other), so one
    // `tokio::join!` pays a single round-trip latency on a ledger carrying both
    // a deploy and a cross-ledger NFT event. Both fail open (empty map on error
    // = exact pre-0283 behaviour):
    // * G1 — verdict by `wasm_hash` from `wasm_interface_metadata` (the deploy
    //   override; upload + deploy are separate Soroban txs / ledgers).
    // * G9 — verdict by `contract_id` from `soroban_contracts` (event routing
    //   for contracts deployed earlier), memoised by `classification_cache`.
    // G1/G9 verdict lookups + task 0320 live WASM-upgrade prior-row prefetch.
    // All three are independent reads; one `join!` pays a single round-trip.
    let (prior_wasm_verdicts, prior_contract_verdicts, prior_contract_rows, sac_classic) = tokio::join!(
        fetch_prior_wasm_verdicts(client, contract_deployments, contract_interfaces),
        fetch_prior_contract_verdicts(
            client,
            nfts,
            nft_events,
            contract_deployments,
            classification_cache,
        ),
        fetch_prior_contract_rows(client, events),
        // task 0331 + ADR 0051: SAC contract-held balances key by the SAC
        // surrogate; re-map them onto the wrapped classic/native asset id below.
        fetch_sac_classic_map(client, soroban_token_balances),
    );
    // Fail closed on the SAC map (unlike the verdict prefetches above): an error
    // here would otherwise orphan contract-held balances under their surrogate key.
    let sac_classic = sac_classic?;
    // Task 0320 live path: `prior_contract_rows` feeds `build_wasm_upgrade_rows`
    // inside `prepare_with_sac_overrides` (same channel as the other two prior-*
    // reads) so it rewrites `soroban_contracts.wasm_hash` for contracts upgraded
    // this ledger — no post-prepare mutation.
    let staged = stage::prepare_with_sac_overrides(&stage::StageInputs {
        ledger,
        transactions,
        operations,
        events,
        invocations,
        contract_interfaces,
        contract_deployments,
        account_states,
        liquidity_pools,
        pool_snapshots,
        assets,
        nfts,
        nft_events,
        lp_positions,
        contract_metadata_writes,
        soroban_token_balances,
        pool_family_writes,
        // ADR 0051: `build_balance_rows` keys contract-held SAC balances onto the
        // wrapped classic/native asset (the SAC has no `assets` row) via this map.
        sac_classic: &sac_classic,
        sac_overrides,
        prior_wasm_verdicts: &prior_wasm_verdicts,
        prior_contract_verdicts: &prior_contract_verdicts,
        prior_contract_rows: &prior_contract_rows,
    })?;
    let mut pw = PartitionWriter::open(client.clone());
    if let Err(err) = pw.write_ledger(staged).await {
        pw.abort().await;
        return Err(err);
    }
    pw.commit().await
}

/// Subset of `wasm_interface_metadata.metadata` we need to re-classify a
/// WASM by its already-persisted function list. The column is the JSON
/// written by [`stage`] as `{"functions": [...], "wasm_byte_len": N}`; we
/// only read `functions` and feed it back through the canonical
/// [`classify_contract_from_wasm_spec`] for exact parity with the deploy-time
/// path (no rule duplication).
#[derive(serde::Deserialize)]
struct WasmMetadata {
    functions: Vec<ContractFunction>,
}

/// Re-classify an already-persisted WASM from its stored
/// `wasm_interface_metadata.metadata` JSON, returning a verdict **only** when
/// it is decisive enough to override the parser default `Other` — i.e.
/// `Nft`/`Fungible`. Anything else (parse failure, `Other`, `Token`) yields
/// `None`.
///
/// Single source of the "parse the stored metadata JSON → classify → keep only
/// the override-worthy verdicts" rule, shared by the live G1 prefetch here and
/// the one-shot `contract-type-rebuild` maintenance pass in `backfill-runner`
/// (which depends on this crate) so the policy lives in exactly one place.
pub fn classify_wasm_metadata_json(metadata_json: &str) -> Option<ContractType> {
    let meta = serde_json::from_str::<WasmMetadata>(metadata_json).ok()?;
    let verdict: ContractType = classify_contract_from_wasm_spec(&meta.functions).into();
    matches!(verdict, ContractType::Nft | ContractType::Fungible).then_some(verdict)
}

/// One `(hash, metadata)` row read from `wasm_interface_metadata` during the
/// live G1 prefetch. `h` is the lower-case hex of the `FixedString(32)`
/// `wasm_hash` (`lower(hex(...))` server-side) so it matches the hex the
/// parser emits without binding raw bytes.
#[derive(clickhouse::Row, serde::Deserialize)]
struct WasmVerdictRow {
    h: String,
    metadata: String,
}

/// Task 0283 live G1 — pre-fetch cross-ledger WASM verdicts for the deploys
/// in this ledger.
///
/// The pure stage (`stage::prepare`) only sees WASM uploaded in the SAME
/// ledger as a deploy. On Soroban `uploadContractWasm` and `createContract`
/// are separate transactions in (almost always) different ledgers, so a
/// deploy's WASM verdict is invisible to the stage and the contract would
/// persist the parser default `Other` — its NFT events then route to the
/// quarantine and the hot tables stay empty. This reads the verdict for those
/// deploy hashes from the already-persisted `wasm_interface_metadata` (the
/// upload landed in an earlier, already-committed ledger) and hands it to the
/// stage's existing deploy override.
///
/// Gated + fail-open by construction:
/// * No non-SAC deploy carrying a `wasm_hash` (≈99.8% of ledgers) → returns
///   empty with **no** round trip.
/// * Hashes already classified in THIS ledger are dropped from the lookup —
///   the same-ledger map already covers them.
/// * Any query / parse failure → empty map + `warn!`, so the writer behaves
///   exactly as pre-0283 (`Other` → quarantine) and the batch backstop
///   (`nft-reclassify`) drains later. The lookup can only improve a verdict,
///   never block a write.
///
/// Known boundary (degrades safely): if the WASM upload and the deploy land in
/// the SAME Lambda batch but DIFFERENT ledgers, and the upload's ledger has not
/// yet committed when the deploy ledger is staged, the prefetch finds nothing
/// and the deploy persists `Other` (and G2 emits no `assets` type-3 row). The
/// batch backstop (`contract-type-rebuild` + `nft-reclassify`) recovers it —
/// same fail-open contract, just deferred.
async fn fetch_prior_wasm_verdicts(
    client: &Client,
    contract_deployments: &[ExtractedContractDeployment],
    contract_interfaces: &[ExtractedContractInterface],
) -> HashMap<[u8; 32], ContractType> {
    let same_ledger: HashSet<String> = contract_interfaces
        .iter()
        .map(|i| i.wasm_hash.to_lowercase())
        .collect();
    let mut want: Vec<String> = contract_deployments
        .iter()
        .filter(|d| !d.is_sac)
        .filter_map(|d| d.wasm_hash.as_deref())
        .map(str::to_lowercase)
        .filter(|h| !same_ledger.contains(h))
        .collect();
    want.sort();
    want.dedup();
    if want.is_empty() {
        return HashMap::new();
    }

    match query_wasm_verdicts(client, &want).await {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "0283 live G1: wasm verdict prefetch failed — deploys persist as Other, batch backstop will drain"
            );
            HashMap::new()
        }
    }
}

/// One AGGREGATED row of the `fetch_sac_classic_map` query (a projection, not the
/// `asset_sac` table shape): a SAC's contract surrogate + the classic/native
/// identity it wraps.
#[derive(clickhouse::Row, serde::Deserialize)]
struct SacClassicRow {
    sac_id: i64,
    asset_type: i16,
    asset_code: String,
    issuer_id: i64,
}

/// Build the `SAC contract surrogate → classic/native asset_id` map from the
/// `asset_sac` facet table (task 0331 + ADR 0051). A SAC is not a distinct asset,
/// so a contract-held SAC balance must be re-keyed onto the classic/native asset it
/// wraps (see [`stage::build_balance_rows`]). `asset_sac` is one row per
/// SAC-having asset (~31k at mainnet scale), so the whole map loads at once — far
/// simpler than a reverse `IN`-list over the non-key `sac_contract_id` column.
/// Skips the query when there are no soroban balances to re-key. On error the map
/// is empty (SAC balances keep their surrogate key — orphaned but never wrong; a
/// re-run recovers).
pub async fn fetch_sac_classic_map(
    client: &Client,
    soroban_token_balances: &[ExtractedSorobanBalance],
) -> Result<HashMap<i64, i64>, clickhouse::error::Error> {
    if soroban_token_balances.is_empty() {
        return Ok(HashMap::new());
    }
    // `asset_sac` is an AggregatingMergeTree: `sac_contract_id` / `sac_deployed` are
    // `SimpleAggregateFunction(max)` columns, so a read MUST `GROUP BY` the full key
    // and apply `max(...)` to collapse the per-part partial states (this is the read
    // protocol, not a magnitude compare — the surrogate is constant per key, so
    // `max` just returns it). `contract_id` is in the key (always 0 for a SAC's
    // classic/native identity) hence in the GROUP BY, but unused in the output.
    // `HAVING max(sac_deployed) = 1` keeps only DEPLOYED SACs — an un-deployed SAC
    // has no contract and cannot hold a balance, so it never matches a balance key.
    let sql = "SELECT max(sac_contract_id) AS sac_id, asset_type, asset_code, issuer_id \
               FROM asset_sac \
               GROUP BY asset_type, asset_code, issuer_id, contract_id \
               HAVING max(sac_deployed) = 1";
    // Propagate the error (do NOT fail open to an empty map): with no map, a
    // contract-held SAC balance keys by its surrogate — which has no `assets` row,
    // so the row orphans (invisible to every read) and is COMMITTED silently. The
    // sibling verdict prefetches fail open because a miss just means "default
    // classification"; here a miss means lost balance data. Returning Err aborts
    // the ledger/partition so it retries once CH is readable again.
    let rows = client.query(sql).fetch_all::<SacClassicRow>().await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.sac_id,
                ids::asset_id(r.asset_type, &r.asset_code, r.issuer_id, 0),
            )
        })
        .collect())
}

/// Read + classify the verdicts for `hashes_hex_lower` from
/// `wasm_interface_metadata`. Returns only `Nft` / `Fungible` verdicts — the
/// deploy override ignores anything else, and `Other` is already the parser
/// default, so there is nothing to override.
async fn query_wasm_verdicts(
    client: &Client,
    hashes_hex_lower: &[String],
) -> Result<HashMap<[u8; 32], ContractType>, clickhouse::error::Error> {
    // The hashes are 64-char lower-case hex derived from parser output — no
    // injection surface. Compared against `lower(hex(wasm_hash))` because CH
    // `hex()` upper-cases while the parser / `hex::encode` produce lower-case.
    let in_list = hashes_hex_lower
        .iter()
        .map(|h| format!("'{h}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT lower(hex(wasm_hash)) AS h, metadata \
         FROM wasm_interface_metadata \
         WHERE lower(hex(wasm_hash)) IN ({in_list})"
    );

    let rows = client.query(&sql).fetch_all::<WasmVerdictRow>().await?;

    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Ok(bytes) = hex::decode(&row.h) else {
            continue;
        };
        let Ok(hash) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            continue;
        };
        if let Some(verdict) = classify_wasm_metadata_json(&row.metadata) {
            out.insert(hash, verdict);
        }
    }
    Ok(out)
}

/// One `(contract_id, contract_type)` row for the live G9 verdict lookup.
/// `contract_type` MUST stay `Option<i16>`: the column is `Nullable(Int16)`
/// and clickhouse 0.15 RBWNAT validation rejects a bare `i16` against
/// `Nullable(_)` — with the fail-open contract that made G9 a 100% no-op in
/// prod (lore-0392). The `WHERE contract_type IN (…)` filter already excludes
/// NULLs; the Option only satisfies the wire-type check.
#[derive(clickhouse::Row, serde::Deserialize)]
struct ContractVerdictRow {
    contract_id: String,
    contract_type: Option<i16>,
}

/// Task 0283 live G9 — resolve cross-ledger verdicts for contracts that emit
/// NFT rows/events in this ledger but were deployed earlier.
///
/// `route_for` (stage) builds its verdict map only from THIS ledger's
/// `contract_rows`, so a transfer from a contract deployed in a prior ledger
/// has no verdict and routes to the quarantine. This reads the already-known
/// verdict straight from `soroban_contracts` (populated at deploy time by G1)
/// and hands it to `route_for` as a fallback, so the event routes correctly
/// (Nft → hot, SAC/Fungible → drop) immediately.
///
/// Gated + fail-open:
/// * No NFT row/event from a not-deployed-here contract → empty, **no** round
///   trip. Contracts deployed THIS ledger are excluded — `contract_rows`
///   already covers them.
/// * Any query failure → empty map + `warn!`; routing falls back to
///   quarantine exactly as pre-G9 and the one-shot `nft-reclassify` drain (or
///   a re-run) recovers it. Can only improve routing, never block a write.
///
/// Option (c): a lazy per-(warm-container) [`ClassificationCache`] (the PG
/// `ClassificationCache` pattern) memoizes definitive verdicts, so the batched
/// DB lookup fires only on the FIRST sighting of a contract (~9/day steady
/// state) rather than once per event-bearing ledger (~80% of ledgers).
///
/// Known limitation (WASM upgrade): the cache keys on `contract_id` and never
/// re-resolves a definitive verdict for the warm-container lifetime, so a
/// contract upgraded from one decisive interface to another (e.g. Fungible →
/// Nft) keeps its first verdict until the container recycles. This is benign
/// today because the parser does not yet surface contract-instance `updated`
/// entries (so an upgrade can't even be detected at write time) — closing it is
/// gated on that parser work (task 0295). When `updated` lands, invalidate the
/// affected `contract_id`s here before the snapshot. Classification is
/// function-name based, so most upgrades preserve the interface and never flip.
async fn fetch_prior_contract_verdicts(
    client: &Client,
    nfts: &[ExtractedNft],
    nft_events: &[ExtractedNftEvent],
    contract_deployments: &[ExtractedContractDeployment],
    cache: &ClassificationCache,
) -> HashMap<String, ContractType> {
    let deployed_here: HashSet<&str> = contract_deployments
        .iter()
        .map(|d| d.contract_id.as_str())
        .collect();
    // Borrow the ids (no per-row clone on the hot path) — `want` only feeds
    // `missing`/`snapshot_for`, both of which take `&str`. Owned Strings are
    // allocated once, lazily, only for the verdicts actually returned below.
    let mut want: Vec<&str> = nfts
        .iter()
        .map(|n| n.contract_id.as_str())
        .chain(nft_events.iter().map(|e| e.contract_id.as_str()))
        .filter(|c| !deployed_here.contains(c))
        .collect();
    want.sort_unstable();
    want.dedup();
    if want.is_empty() {
        return HashMap::new();
    }

    // Only the contracts not yet memoized hit the DB; `Other`/unknown are never
    // cached, so they re-resolve until they earn a definitive verdict.
    let missing = cache.missing(want.iter().copied());
    if !missing.is_empty() {
        match query_contract_verdicts(client, &missing).await {
            Ok(fetched) => cache.extend_definitive(fetched),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "0283 live G9: contract verdict prefetch failed — events route to quarantine, drain recovers"
                );
                // Fail-open: don't populate; uncached contracts route to pending.
            }
        }
    }
    // `snapshot_for` borrows from `want`; copy to owned for the stage map.
    cache
        .snapshot_for(want.iter().copied())
        .into_iter()
        .map(|(id, ty)| (id.to_string(), ty))
        .collect()
}

/// Read decisive verdicts (`Token` / `Nft` / `Fungible`; `Other`/NULL skipped
/// — they route to quarantine anyway) for `contract_ids` from
/// `soroban_contracts`. Including `Token` lets SAC-emitted events DROP at
/// routing instead of leaking into pending (closes the 0221 write-time leak).
async fn query_contract_verdicts(
    client: &Client,
    contract_ids: &[&str],
) -> Result<HashMap<String, ContractType>, clickhouse::error::Error> {
    // `contract_ids` are `C…` StrKeys from parser output (base32, no quote
    // chars) — no injection surface.
    let in_list = contract_ids
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT contract_id, contract_type FROM soroban_contracts FINAL \
         WHERE contract_id IN ({in_list}) AND contract_type IN (0, 2, 3)"
    );

    let rows = client.query(&sql).fetch_all::<ContractVerdictRow>().await?;

    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        if let Some(ty) = row.contract_type
            && let Ok(verdict) = ContractType::try_from(ty)
        {
            out.insert(row.contract_id, verdict);
        }
    }
    Ok(out)
}

/// Task 0320 live path — prefetch the prior `soroban_contracts` rows (in full) for
/// the contracts that emit an `executable_update` SYSTEM event this ledger, so
/// [`stage::build_wasm_upgrade_rows`] can carry deployer / deployed_at / name /
/// contract_type / is_sac forward when it rewrites `wasm_hash` (the RMT whole-row
/// replace would otherwise NULL them — the clobber class owned by task 0316).
/// The row is read back as the same [`rows::SorobanContractRow`] it will be
/// written as, so no projection DTO is needed — the upgrade build just clones it
/// and overrides the three columns the upgrade changes.
///
/// Gated + fail-open: no `executable_update` event in the ledger (≈always) →
/// **no round trip**, empty map. Any query failure → empty map + `warn!`, and the
/// upgrade row is simply not emitted this ledger; the in-CH `wasm-upgrade-backfill`
/// maintenance pass recovers it.
async fn fetch_prior_contract_rows(
    client: &Client,
    events: &[(String, Vec<ExtractedEvent>)],
) -> HashMap<String, rows::SorobanContractRow> {
    let mut want: Vec<&str> = events
        .iter()
        .flat_map(|(_, evs)| evs.iter())
        // Consensus events only — drop the diagnostic container (byte-identical
        // copies + failed-tx events). Must match `build_wasm_upgrade_rows`'s
        // filters so the prefetch covers exactly the contracts it will rewrite:
        // non-diagnostic, host-emitted SYSTEM events with a parseable new hash.
        .filter(|ev| !matches!(ev.source, EventSource::Diagnostic))
        .filter(|ev| ev.event_type == ContractEventType::System)
        .filter(|ev| extract_executable_update_new_wasm_hash(&ev.topics).is_some())
        .filter_map(|ev| ev.contract_id.as_deref())
        .collect();
    want.sort_unstable();
    want.dedup();
    if want.is_empty() {
        return HashMap::new();
    }

    // `want` are `C…` StrKeys from parser output (base32, no quote chars).
    let in_list = want
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(", ");
    // Column order MUST match `rows::SorobanContractRow` field order (RowBinary is
    // positional). No `name` — dropped by task 0304 (selecting it was Code 47
    // UNKNOWN_IDENTIFIER on prod, killing every 0320 prefetch; lore-0392).
    let sql = format!(
        "SELECT id, contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, \
                deployed_at_ledger, contract_type, is_sac \
         FROM soroban_contracts FINAL WHERE contract_id IN ({in_list})"
    );

    match client
        .query(&sql)
        .fetch_all::<rows::SorobanContractRow>()
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|r| (r.contract_id.clone(), r))
            .collect(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "0320 live: prior contract-row prefetch failed — upgrade row skipped this ledger, wasm-upgrade-backfill recovers"
            );
            HashMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_ledger() -> ExtractedLedger {
        ExtractedLedger {
            sequence: 1,
            hash: "00".repeat(32),
            closed_at: 0,
            protocol_version: 22,
            transaction_count: 0,
            base_fee: 100,
        }
    }

    /// `persist_ledger_clickhouse` against an unroutable URL must
    /// surface a transport error — we treat it as the "no side effect"
    /// proof that the wrapper actually issues the insert path
    /// (the stub it replaced returned Ok without any I/O).
    #[tokio::test]
    async fn wrapper_returns_err_when_client_unreachable() {
        // Deliberately unroutable URL — any real network use must fail.
        let client = Client::default().with_url("http://127.0.0.1:1");
        let ledger = synthetic_ledger();
        // Single ledger, no transactions or downstream data — the
        // writer still opens the `ledgers` insert at commit time, which
        // touches the network and surfaces the unroutable-URL error.
        let res = persist_ledger_clickhouse(
            &client,
            &ledger,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &ClassificationCache::new(),
        )
        .await;
        assert!(res.is_err(), "expected transport error, got: {res:?}");
    }
}
