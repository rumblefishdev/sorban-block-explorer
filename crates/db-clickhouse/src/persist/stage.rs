//! Synchronous pre-write transform from parser `Extracted*` slices to
//! CH-shaped row structs in [`super::rows`].
//!
//! ## Design — hybrid surrogate / natural keys
//!
//! Three high-cardinality FK hubs (`accounts`, `soroban_contracts`,
//! `transactions`) carry surrogate `id: Int64` derived via
//! [`super::ids`] (`cityhash64(natural_key)`, deterministic). All FK
//! columns pointing at these tables are `Int64` referencing those
//! ids. Cheaper joins, smaller storage, faster scans vs natural-key
//! everywhere — see `ids.rs` module docs for the why.
//!
//! Other tables (`assets`, `nfts`, `liquidity_pools`,
//! `liquidity_pool_snapshots`, `operations_appearances`,
//! `transaction_participants`, `nft_ownership`, `lp_positions`,
//! `account_balances_current`) keep natural / composite primary
//! keys where they're already cheap.
//!
//! ## Three structural differences from PG path
//!
//! 1. **`soroban_events` is unfolded** per ADR 0044 §Decision §4a —
//!    one CH row per `ExtractedEvent` (vs PG's per-`(contract, tx,
//!    ledger)` fold).
//! 2. **`Decimal128(7)` scaling happens here.** Parser emits decimal
//!    strings or i64 stroops; CH RowBinary expects underlying `i128`
//!    scaled by 10⁷ (1 stroop = 1 i128 unit).
//! 3. **Stub-rowing for `soroban_contracts`.** Pass 2 emits NULL-rich
//!    rows for contracts referenced by events/ops/invocations but
//!    not deployed in this batch. Mirrors PG `write::upsert_contracts_
//!    returning_id` Pass 2 — keeps mid-stream backfill ranges
//!    JOIN-able even without preceding history.
//!
//! ## Trustline removal handling
//!
//! Parser-emitted `removed_trustlines` are translated to a unified
//! `BalanceRow` with `amount = 0` and current ledger as
//! `last_updated_ledger`. `ReplacingMergeTree(last_updated_ledger)`
//! keeps the zero-balance row (newest version wins). Read-time
//! convention: `WHERE amount > 0` to recover "active trustlines"
//! semantics.

use std::collections::{HashMap, HashSet};

use domain::{AssetType, ContractEventType, ContractType, OperationType};
use serde_json::Value;
use xdr_parser::ExtractedContractMetadata;
use xdr_parser::ExtractedSorobanBalance;
use xdr_parser::SacOverride;
use xdr_parser::asset_appearances::AssetRef;
use xdr_parser::scval;
use xdr_parser::types::{
    EventSource, ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment,
    ExtractedContractInterface, ExtractedEvent, ExtractedInvocation, ExtractedLedger,
    ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft,
    ExtractedNftEvent, ExtractedOperation, ExtractedTransaction, SacAssetIdentity,
};
use xdr_parser::{AccountDelta, LedgerDelta, NetSettled};
use xdr_parser::{EventAsset, LedgerAsset};

use xdr_parser::event::extract_executable_update_new_wasm_hash;
use xdr_parser::pool_config_factory::PoolConfig;
use xdr_parser::pool_family::PoolFamilyWrite;

use super::ids;
use super::rows::*;
use crate::SchemaError;

/// Sum `gross_volume_a` (asset-A-side trade volume, stroops) per pool from the
/// `claimedAtoms` the parser attaches to path-payment / offer ops (the 0261
/// claim-atom extractor emits `amountA` per atom). Keyed by raw 32-byte pool id
/// to match [`LiquidityPoolSnapshotRow`]. Trades only — LP deposits/withdrawals
/// carry no claimed atoms. Shared by live ingest (via [`prepare`]) and the 0266
/// backfill worker, so the value is identical on either path.
pub fn gross_volume_a_by_pool(
    operations: &[(String, Vec<ExtractedOperation>)],
) -> HashMap<[u8; 32], i128> {
    let mut gross: HashMap<[u8; 32], i128> = HashMap::new();
    for (_tx, ops) in operations {
        for op in ops {
            let Some(atoms) = op.details.get("claimedAtoms").and_then(Value::as_array) else {
                continue;
            };
            for atom in atoms {
                let (Some(pool_hex), Some(amount_a)) = (
                    atom.get("poolId").and_then(Value::as_str),
                    atom.get("amountA").and_then(Value::as_i64),
                ) else {
                    continue;
                };
                let Ok(bytes) = hex::decode(pool_hex) else {
                    continue;
                };
                let Ok(pool_id) = <[u8; 32]>::try_from(bytes.as_slice()) else {
                    continue;
                };
                *gross.entry(pool_id).or_insert(0) += i128::from(amount_a);
            }
        }
    }
    gross
}

/// Per-(pool, asset) SIGNED amounts for ONE operation — everything it moved
/// through a pool (task 0279 → `lp_operation_amounts`), from the two sources
/// the XDR offers, which are disjoint by op type:
///
/// - **trades** (path payments / offers): `claimedAtoms`, the same atoms and
///   same JSON shape [`gross_volume_a_by_pool`] walks — that one sums the
///   attribution away, this one keeps it;
/// - **deposits / withdrawals**: `poolDelta`, the pool's own before/after
///   reserve images from the op's meta (`operation.rs::pool_delta_details`).
///   They carry no claim atoms, and their op body holds only the caller's
///   max/min bounds — never what executed.
///
/// **Sign is from the POOL's side**, which is the perspective the XDR atom
/// already uses: `assetSold`/`amountSold` is what was taken FROM the offer
/// owner (here, the pool) and `assetBought`/`amountBought` what was sent TO it.
/// So sold → negative, bought → positive. Verified against prod rather than
/// read off the spec: on ledger 63,904,097 of pool `41270552…` (XLM/TF),
/// Horizon reports the pool selling XLM, and that ledger's snapshot moves
/// `reserve_a` (XLM) down by exactly the summed sold amount while `reserve_b`
/// (TF) rises.
///
/// Summing per (op, pool, asset) is what keeps the row key
/// `(pool, ledger, tx, application_order, asset)` collision-free: one op can
/// take the same pool several times under CAP-38 interleaved matching, and
/// those atoms would otherwise be distinct rows sharing a key — which the RMT
/// collapses to one, silently dropping the rest of the fill. A circular path
/// crossing the SAME pool in BOTH directions nets out here by construction;
/// that is the honest per-op figure, and it is why the backfill's
/// `gross_volume_a` gate compares `abs(amount)` (see the schema comment).
///
/// Returned sorted, so a replay emits rows in a stable order.
fn pool_fill_amounts(details: &Value) -> Vec<([u8; 32], i64, i64)> {
    let mut sums: HashMap<([u8; 32], i64), i64> = HashMap::new();

    let atoms = details
        .get("claimedAtoms")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for atom in atoms {
        let Some(pool_id) = atom_pool_id(atom) else {
            continue;
        };
        // The atom is written from the offer owner's side: sold is taken FROM
        // the pool, bought is sent TO it.
        for (asset_key, amount_key, sign) in [
            ("assetBought", "amountBought", 1i64),
            ("assetSold", "amountSold", -1i64),
        ] {
            let (Some(asset_id), Some(amount)) = (
                atom.get(asset_key).and_then(claim_atom_asset_id),
                atom.get(amount_key).and_then(Value::as_i64),
            ) else {
                continue;
            };
            let entry = sums.entry((pool_id, asset_id)).or_insert(0);
            *entry = entry.saturating_add(sign * amount);
        }
    }

    // `poolDelta` is already after-minus-before per leg, so its sign needs no
    // interpretation here — only the same (pool, asset) folding.
    if let Some(delta) = details.get("poolDelta")
        && let Some(pool_id) = atom_pool_id(delta)
    {
        for (asset_key, amount_key) in [("assetA", "amountA"), ("assetB", "amountB")] {
            let (Some(asset_id), Some(amount)) = (
                delta.get(asset_key).and_then(claim_atom_asset_id),
                delta.get(amount_key).and_then(Value::as_i64),
            ) else {
                continue;
            };
            let entry = sums.entry((pool_id, asset_id)).or_insert(0);
            *entry = entry.saturating_add(amount);
        }
    }

    let mut out: Vec<_> = sums
        .into_iter()
        // A leg that nets to zero moved nothing this op — no row for it.
        .filter(|(_, amount)| *amount != 0)
        .map(|((pool_id, asset_id), amount)| (pool_id, asset_id, amount))
        .collect();
    out.sort_unstable();
    out
}

/// The raw 32-byte pool id of a `claimedAtoms` / `poolDelta` entry.
fn atom_pool_id(entry: &Value) -> Option<[u8; 32]> {
    let hex = entry.get("poolId").and_then(Value::as_str)?;
    <[u8; 32]>::try_from(hex::decode(hex).ok()?.as_slice()).ok()
}

/// The `asset_id` surrogate for a claim atom's asset ref — the `format_asset`
/// wire shape (`"native"` or `"CODE:GISSUER"`) mapped onto the same surrogate
/// space `operation_asset_appearances` / `balances` use, so a pool leg keys
/// identically wherever it appears.
fn claim_atom_asset_id(asset: &Value) -> Option<i64> {
    let s = asset.as_str()?;
    if s == "native" {
        return Some(ids::NATIVE_ASSET_ID);
    }
    let (code, issuer) = s.split_once(':')?;
    (!code.is_empty() && !issuer.is_empty()).then(|| ids::credit_asset_id(code, issuer))
}

#[derive(Debug, Default)]
pub struct StagedLedger {
    pub ledger_sequence: i64,

    pub ledger_rows: Vec<LedgerRow>,
    pub account_rows: Vec<AccountRow>,
    /// `account_entry_state` rows — one per account whose `AccountEntry` was
    /// OBSERVED this change set (full-set replace; trustline-only appearances
    /// never emit one). lore-0463.
    pub account_entry_state_rows: Vec<AccountEntryStateRow>,
    pub wasm_rows: Vec<WasmInterfaceMetadataRow>,
    pub contract_rows: Vec<SorobanContractRow>,
    /// On-chain Soroban token metadata side table (task 0297). Populated inside
    /// [`prepare_with_sac_overrides`] via [`build_metadata_rows`] from the
    /// `StageInputs.contract_metadata_writes` slice.
    pub metadata_rows: Vec<SorobanContractMetadataRow>,
    pub transaction_rows: Vec<TransactionRow>,
    pub hash_index_rows: Vec<TransactionHashIndexRow>,
    pub participant_rows: Vec<TransactionParticipantRow>,
    pub pool_rows: Vec<LiquidityPoolRow>,
    pub pool_instance_state_rows: Vec<PoolInstanceStateRow>,
    pub pool_state_change_rows: Vec<PoolStateChangeRow>,
    pub snapshot_rows: Vec<LiquidityPoolSnapshotRow>,
    pub lp_position_rows: Vec<LpPositionRow>,
    pub op_rows: Vec<OperationAppearanceRow>,
    /// Per-(asset, tx) presence rows (task 0359) → `operation_asset_appearances`,
    /// the asset-dimension twin of `participant_rows`.
    pub op_asset_rows: Vec<OperationAssetAppearanceRow>,
    /// Per-(pool, tx) presence rows (task 0365) → `operation_pools`, the
    /// pool-dimension twin of `participant_rows` / `op_asset_rows`.
    pub op_pool_rows: Vec<OperationPoolRow>,
    /// Per-(op, pool, asset) amounts (task 0279) → `lp_operation_amounts`, the
    /// value twin of `op_pool_rows`. Trades from `claimedAtoms`, deposits and
    /// withdrawals from the op's own reserve delta (`poolDelta`).
    pub lp_amount_rows: Vec<LpOperationAmountRow>,
    pub event_rows: Vec<SorobanEventRow>,
    pub invocation_rows: Vec<SorobanInvocationAppearanceRow>,
    pub asset_rows: Vec<AssetRow>,
    /// SAC facet rows (ADR 0051) → `asset_sac` AggregatingMergeTree side table.
    pub asset_sac_rows: Vec<AssetSacRow>,
    pub nft_rows: Vec<NftRow>,
    pub nft_ownership_rows: Vec<NftOwnershipRow>,
    /// Task 0217 / 0220 — quarantine bucket for NFT rows whose
    /// contract is still `Other` / NULL-classified at staging time.
    /// Routed alongside `nft_rows` via the per-contract verdict
    /// computed from observed WASM interfaces in this ledger plus the
    /// parser-emitted `contract_type` on each deployment. CH has no
    /// per-row UPDATE, so promotion happens only via the post-backfill
    /// drain runbook.
    pub nft_pending_rows: Vec<NftPendingRow>,
    pub nft_ownership_pending_rows: Vec<NftOwnershipPendingRow>,
    /// Unified `balances` rows for ALL asset types (task 0331 Option A). Type-3
    /// tokens are built in [`prepare_with_sac_overrides`] via [`build_balance_rows`]
    /// from `StageInputs.soroban_token_balances`; classic + native per-account
    /// balances are appended straight from `account_states` (single-write — the
    /// legacy `account_balances_current` staging was removed).
    pub unified_balance_rows: Vec<BalanceRow>,
}

/// Named, borrowed inputs to [`prepare_with_sac_overrides`].
///
/// Replaces the former ~18 positional arguments: many are `&[T]` slices and a
/// few share types, so a positional call could silently transpose two. Named
/// fields make the call site readable and a wrong order a compile error. `Copy`
/// (every field is a shared reference) so the stage body can destructure it
/// back into locals with zero ceremony.
#[derive(Clone, Copy)]
pub struct StageInputs<'a> {
    pub ledger: &'a ExtractedLedger,
    pub transactions: &'a [ExtractedTransaction],
    pub operations: &'a [(String, Vec<ExtractedOperation>)],
    pub events: &'a [(String, Vec<ExtractedEvent>)],
    pub invocations: &'a [(String, Vec<ExtractedInvocation>)],
    pub contract_interfaces: &'a [ExtractedContractInterface],
    pub contract_deployments: &'a [ExtractedContractDeployment],
    pub account_states: &'a [ExtractedAccountState],
    pub liquidity_pools: &'a [ExtractedLiquidityPool],
    pub pool_snapshots: &'a [ExtractedLiquidityPoolSnapshot],
    pub assets: &'a [ExtractedAsset],
    pub nfts: &'a [ExtractedNft],
    pub nft_events: &'a [ExtractedNftEvent],
    pub lp_positions: &'a [ExtractedLpPosition],
    /// On-chain Soroban token metadata writes (task 0297). Threaded through to
    /// `metadata_rows` via [`build_metadata_rows`] inside
    /// [`prepare_with_sac_overrides`]. Empty `&[]` for legacy callers.
    pub contract_metadata_writes: &'a [ExtractedContractMetadata],
    /// Per-holder Soroban token (type-3) balances from `ContractData`
    /// `Balance(Address)` entries (task 0331). Threaded to the unified
    /// `unified_balance_rows` via [`build_balance_rows`]. Empty `&[]` for
    /// legacy callers.
    pub soroban_token_balances: &'a [ExtractedSorobanBalance],
    /// Every family's pool state writes behind ONE seam (task 0518,
    /// decision 4a): router-family plane `PoolData` (the fungible reserve
    /// source, task 0374 step 7) and pool instances (the STATE source for
    /// share tokens and planes; supersedes the deposit⇄mint detector as
    /// primary — it stays a cross-check), and pair-factory instances (reserve
    /// source AND declaration: leg tokens, deploying factory, LP supply).
    /// Staging partitions by variant; adding a family adds a variant + an
    /// arm, never a field.
    pub pool_family_writes: &'a [xdr_parser::pool_family::PoolFamilyWrite],
    /// Task 0331 + ADR 0051 — SAC contract surrogate → wrapped classic/native
    /// `asset_id` (from `asset_sac`, via [`crate::persist::fetch_sac_classic_map`]).
    /// [`build_balance_rows`] keys a contract-held SAC balance onto the classic
    /// asset it wraps instead of the SAC surrogate (which has no `assets` row of its
    /// own). Empty map for legacy callers (SAC balances keep their surrogate key).
    pub sac_classic: &'a HashMap<i64, i64>,
    /// Crypto-proven un-deployed-SAC overrides for this ledger's events
    /// (task 0323, `xdr_parser::detect_undeployed_sac_overrides`). Each
    /// suppresses the Pass-2 FK stub (no contract row) + seeds a SAC `assets`
    /// row. Empty for legacy callers.
    pub sac_overrides: &'a [SacOverride],
    /// Task 0283 live G1 — cross-ledger WASM verdicts by `wasm_hash`. Empty map
    /// for legacy callers (behaves exactly as pre-0283).
    pub prior_wasm_verdicts: &'a HashMap<[u8; 32], ContractType>,
    /// Task 0283 live G9 — cross-ledger contract verdicts by `contract_id`.
    pub prior_contract_verdicts: &'a HashMap<String, ContractType>,
    /// Task 0320 live WASM-upgrade — prior `soroban_contracts` rows (read back
    /// in full) for the contracts that emit an `executable_update` this ledger,
    /// so [`build_wasm_upgrade_rows`] can carry identity forward when it rewrites
    /// `wasm_hash`. Empty map for legacy callers (no upgrade rows emitted).
    pub prior_contract_rows: &'a HashMap<String, SorobanContractRow>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare(
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
) -> Result<StagedLedger, SchemaError> {
    // Convenience wrapper: no SAC overrides, no cross-ledger verdicts (the
    // legacy / test path). Behaves exactly as the pre-StageInputs `prepare`.
    prepare_with_sac_overrides(&StageInputs {
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
        contract_metadata_writes: &[],
        pool_family_writes: &[],
        soroban_token_balances: &[],
        sac_classic: &HashMap::new(),
        sac_overrides: &[],
        prior_wasm_verdicts: &HashMap::new(),
        prior_contract_verdicts: &HashMap::new(),
        prior_contract_rows: &HashMap::new(),
    })
}

/// Build corrected `soroban_contracts` rows for contracts that emitted an
/// `executable_update` SYSTEM event this ledger (task 0320, live path).
///
/// Per `executable_update` event with a parseable new WASM hash AND a known
/// prior row, emit a full row that **overrides** `wasm_hash` +
/// `wasm_uploaded_at_ledger` (= the upgrade ledger, the RMT version that wins
/// the merge) and **carries forward** the identity columns. `contract_type` is
/// carried forward unchanged — the class never net-changes on upgrade for
/// current data; the rare flip is task 0325.
///
/// Events without a parseable hash, without a contract address, or **without a
/// prior row are skipped** — emitting a partial row would clobber the identity
/// columns to NULL under RMT. Multiple upgrades of one contract in the same
/// ledger collapse to the last (on-chain application order).
pub fn build_wasm_upgrade_rows(
    events: &[(String, Vec<ExtractedEvent>)],
    prior: &HashMap<String, SorobanContractRow>,
    ledger_sequence: i64,
) -> Vec<SorobanContractRow> {
    // Keyed by contract_id so multiple upgrades of one contract in the same
    // ledger collapse to the last seen (= on-chain application order).
    let mut by_contract: HashMap<String, SorobanContractRow> = HashMap::new();
    for (_tx_hash, evs) in events {
        for ev in evs {
            // Only consensus events drive state. The diagnostic container holds
            // byte-identical copies of consensus events AND events from FAILED
            // transactions (an upgrade that never applied) — acting on those would
            // write a `wasm_hash` the chain never adopted. Mirror the `soroban_events`
            // staging guard (this is the same population the backfill reads, post-drop).
            if is_diagnostic(ev.source) {
                continue;
            }
            // Only the host emits `executable_update`, and always as a SYSTEM
            // event. A contract can emit a Contract-typed event with the same
            // topic shape; requiring System blocks that spoof of its own
            // `wasm_hash` (and never drops a real upgrade — all are System).
            if ev.event_type != ContractEventType::System {
                continue;
            }
            let Some(addr) = ev.contract_id.as_deref() else {
                continue;
            };
            // `extract_…` returns `Some` only for a well-formed executable_update.
            let Some(new_hash) = extract_executable_update_new_wasm_hash(&ev.topics) else {
                continue;
            };
            // Skip-on-miss: without the prior row we cannot carry identity
            // columns forward, and a partial row would NULL them under RMT.
            let Some(prior_row) = prior.get(addr) else {
                continue;
            };
            // Carry the prior row forward verbatim, overriding only what the
            // upgrade actually changes: the new WASM hash and the RMT version
            // (= upgrade ledger, so it wins the merge). `id` / `contract_id` /
            // deployer / name / contract_type / is_sac all ride along from the
            // read-back row — matching the backfill SQL, which also passes
            // `is_sac` through (no upgrader is a mislabeled SAC on current data).
            let mut row = prior_row.clone();
            row.wasm_hash = Some(new_hash);
            row.wasm_uploaded_at_ledger = ledger_sequence;
            by_contract.insert(addr.to_string(), row);
        }
    }
    by_contract.into_values().collect()
}

/// Map parser-extracted token-metadata writes to `soroban_contract_metadata`
/// rows (task 0297). Called inside [`prepare_with_sac_overrides`] from the
/// `StageInputs.contract_metadata_writes` slice. SAC filtering already happened
/// in the producer (`xdr_parser::extract_contract_metadata_writes`); `version` =
/// observed ledger.
pub fn build_metadata_rows(
    writes: &[ExtractedContractMetadata],
) -> Vec<SorobanContractMetadataRow> {
    writes
        .iter()
        .map(|w| SorobanContractMetadataRow {
            contract_id: w.contract_id.clone(),
            name: w.metadata.name.clone(),
            symbol: w.metadata.symbol.clone(),
            decimals: w.metadata.decimals,
            version: i64::from(w.ledger),
        })
        .collect()
}

/// Map parser-extracted Soroban token balances to unified `balances` rows
/// (task 0331, Option C). `holder_id` = `ids::address_id(holder)`; `amount` raw.
///
/// `asset_id` is resolved from the STORING contract's surrogate: a SAC is NOT a
/// distinct asset (ADR 0051 retired `asset_type=2`) and has no `assets` row of its
/// own, so a contract-held SAC balance must key by the classic/native asset it
/// wraps or it would orphan. `sac_classic` maps a SAC contract surrogate → its
/// classic/native `asset_id` (from the `asset_sac` facet, via
/// [`crate::persist::fetch_sac_classic_map`]); a storing contract ABSENT from the
/// map is a type-3 token and keeps its own surrogate (`ids::asset_id(3, …)`). The
/// resolution happens HERE, at build time — the live indexer (`prepare_with_sac_overrides`)
/// and the RPC seed both call this one shared fn with the same map, so neither
/// post-mutates the staged rows.
pub fn build_balance_rows(
    balances: &[ExtractedSorobanBalance],
    sac_classic: &HashMap<i64, i64>,
) -> Vec<BalanceRow> {
    // Dedup by (holder_id, asset_id) keeping the LAST occurrence, position-stable:
    // two txs in one ledger can touch the same holder+asset, producing rows that
    // share the RMT version (`last_updated_ledger`) — a tie the merge would resolve
    // nondeterministically. Ledger/tx order puts the final state last, so last-wins
    // is correct; the first-seen position is preserved for deterministic output.
    let mut rows: Vec<BalanceRow> = Vec::with_capacity(balances.len());
    let mut idx: HashMap<(i64, i64), usize> = HashMap::with_capacity(balances.len());
    for b in balances {
        let contract = ids::contract_id(&b.contract_id);
        let holder_id = ids::address_id(&b.holder);
        let asset_id = sac_classic
            .get(&contract)
            .copied()
            .unwrap_or_else(|| ids::asset_id(3, "", 0, contract));
        let row = BalanceRow {
            holder_id,
            asset_id,
            amount: b.balance,
            last_updated_ledger: i64::from(b.ledger),
            // The `ContractData` entry was removed, as opposed to a holder who
            // merely spent down to zero — ADR 0055.
            closed_at_ledger: if b.closed { i64::from(b.ledger) } else { 0 },
        };
        match idx.get(&(holder_id, asset_id)) {
            Some(&i) => rows[i] = row,
            None => {
                idx.insert((holder_id, asset_id), rows.len());
                rows.push(row);
            }
        }
    }
    rows
}

/// Same as [`prepare`] but also consumes `sac_overrides` — the crypto-proven
/// un-deployed-SAC emitters for this ledger (task 0323). An un-deployed SAC is
/// modelled as an ASSET, not a contract: each override (a) suppresses the
/// Pass-2 FK stub so NO `soroban_contracts` row is written for it, and (b)
/// seeds a SAC `assets` row from its `identity`. (Replaces the task-0220
/// `is_sac=true` skeleton re-insert, which wrote a contract row instead.)
///
/// Production callers that have a `ParseOutput.sac_overrides` slice
/// (PG-side bridge for task 0218 + the CH backfill path) call this
/// directly; legacy callers via [`prepare`] get a no-op override list
/// and behave exactly as before — the override mechanism stays opt-in
/// at the call site, so the addition is fully backward-compatible.
///
/// `prior_wasm_verdicts` (task 0283 live G1 fix) carries cross-ledger
/// WASM verdicts the pure stage cannot see: on Soroban `uploadContractWasm`
/// and `createContract` are separate transactions in (almost always)
/// different ledgers, so a deploy's WASM is invisible to the same-ledger
/// `wasm_classification` map below and the contract would persist the parser
/// default `Other`. The writer pre-fetches the verdict for such hashes from
/// the already-persisted `wasm_interface_metadata` (see
/// `persist::fetch_prior_wasm_verdicts`) and passes it here; the deploy
/// override consults it as a fallback after the same-ledger map. Legacy
/// callers via [`prepare`] pass an empty map and behave exactly as before.
pub fn prepare_with_sac_overrides(input: &StageInputs<'_>) -> Result<StagedLedger, SchemaError> {
    // Destructure back into locals (StageInputs is `Copy`) so the body below is
    // unchanged from the positional-argument era — every name matches.
    let StageInputs {
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
        sac_classic,
        sac_overrides,
        prior_wasm_verdicts,
        prior_contract_verdicts,
        prior_contract_rows,
    } = *input;

    let ledger_sequence_i64 = i64::from(ledger.sequence);
    let ledger_hash = decode_hash(&ledger.hash, "ledger.hash")?;
    let ledger_closed_at_ms = ledger.closed_at.saturating_mul(1_000);

    let mut out = StagedLedger {
        ledger_sequence: ledger_sequence_i64,
        ..Default::default()
    };

    out.ledger_rows.push(LedgerRow {
        sequence: ledger_sequence_i64,
        hash: ledger_hash,
        closed_at: ledger_closed_at_ms,
        protocol_version: i32::try_from(ledger.protocol_version)
            .map_err(|_| staging_err("protocol_version overflow"))?,
        transaction_count: i32::try_from(ledger.transaction_count)
            .map_err(|_| staging_err("transaction_count overflow"))?,
        base_fee: i64::from(ledger.base_fee),
    });

    // ---- Accounts universe + per-tx participant union ----
    let mut account_keys: HashSet<String> = HashSet::new();
    let mut participants_per_tx: HashMap<String, HashSet<String>> = HashMap::new();
    // Task 0383 (K3-4 asset side): (asset_id) touched by a decoded Soroban token
    // event, per tx. Emitted into `operation_asset_appearances` once tx_ids are
    // resolved. `HashSet` dedups the common case of many token events per tx.
    let mut event_assets_per_tx: HashMap<String, HashSet<i64>> = HashMap::new();
    let has_soroban: HashMap<String, bool> = tx_has_soroban_map(operations);

    // Net-settled "value moved" per (tx, asset), looked up when the
    // `operation_asset_appearances` presence rows are emitted below.
    //
    // Value comes from the AUTHORITATIVE LEDGER — the account / trustline /
    // ContractData balance changes consensus actually applied — for EVERY tx,
    // classic or Soroban. Token EVENTS are contract-emitted LOGS (any contract can
    // emit any `"transfer"` it likes), so they are NEVER trusted for value; a
    // ledger balance cannot be forged. `ledger_deltas` carries the per-(holder,
    // asset) balance deltas: native (`AccountEntry`), classic credit
    // (`TrustLineEntry`), SAC contract-held (`ContractData` `Balance` struct), and
    // bespoke token balances (`ContractData` `Balance` bare i128). `sac_classic`
    // re-maps a contract-held SAC balance onto the wrapped classic asset (a SAC
    // address is a one-way hash of its asset, so the reverse needs the registry).
    // `None` = the reduction was not representable in i128 → NULL ("not computed"),
    // never a wrapped figure.
    //
    // A SAC first seen THIS ledger (its carrier flagged with `sac_contract_id` in
    // `assets`) isn't in the pre-fetched DB `asset_sac` map yet. Seed those
    // current-ledger carriers so a same-ledger contract-held SAC balance re-keys
    // onto the wrapped classic/native id instead of orphaning on its surrogate.
    // This seeded `sac_map` feeds BOTH the value reduction here AND
    // `build_balance_rows` below, so a C→C transfer of a just-registered SAC nets
    // correctly on both paths (its legs are both `SacWrapped`). Guarded: the
    // common ledger (no new SAC) skips the clone; the DB map wins (`or_insert`).
    let effective_sac_classic;
    let sac_map: &HashMap<i64, i64> = if assets.iter().any(|t| t.sac_contract_id.is_some()) {
        let mut m = sac_classic.clone();
        for t in assets {
            if let Some(sac) = t.sac_contract_id.as_deref() {
                let issuer_id = t
                    .issuer_address
                    .as_deref()
                    .map(ids::account_id)
                    .unwrap_or(0);
                let classic = ids::asset_id(
                    t.asset_type as i16,
                    t.asset_code.as_deref().unwrap_or(""),
                    issuer_id,
                    0,
                );
                m.entry(ids::contract_id(sac)).or_insert(classic);
            }
        }
        effective_sac_classic = m;
        &effective_sac_classic
    } else {
        sac_classic
    };

    // O(1) per-tx op count lookup. Built once over `operations` so the
    // transactions loop stays linear in `transactions.len()` rather than
    // the prior O(tx_count × op_groups) `iter().find()` scan.
    let op_count_by_tx: HashMap<&str, i16> = operations
        .iter()
        .map(|(h, ops)| (h.as_str(), i16::try_from(ops.len()).unwrap_or(i16::MAX)))
        .collect();

    for tx in transactions {
        let entry = participants_per_tx.entry(tx.hash.clone()).or_default();
        account_keys.insert(tx.source_account.clone());
        entry.insert(tx.source_account.clone());
        // Task 0359 K2-4: the fee-bump payer funds the fee but runs no ops and is
        // not the inner source — register it so the fee-bump tx shows on the
        // payer's account page. `Some` only for fee-bump envelopes.
        if let Some(fee_source) = &tx.fee_source
            && is_strkey_account(fee_source)
        {
            account_keys.insert(fee_source.clone());
            entry.insert(fee_source.clone());
        }
    }

    for (tx_hash, ops) in operations {
        let entry = participants_per_tx.entry(tx_hash.clone()).or_default();
        for op in ops {
            if let Some(src) = &op.source_account {
                account_keys.insert(src.clone());
                entry.insert(src.clone());
            }
            // Task 0359 F-C (K1-5): typed parser-emitted counterparties — every
            // account the op involves besides its source (destinations, crossed-
            // offer sellers, CB claimants, inflationDest, revoke targets). The
            // single op-side participant source, replacing the old string-`details`
            // participant extraction. `is_strkey_account` guards, the
            // `starts_with('G')` retain below is the final backstop.
            for key in &op.counterparties {
                if is_strkey_account(key) {
                    account_keys.insert(key.clone());
                    entry.insert(key.clone());
                }
            }
            // Asset issuers are deliberately NOT registered as participants (task
            // 0359, decision 1c): an asset's activity lives on its ASSET page
            // (`operation_asset_appearances`), and the issuer is derivable from the
            // asset_id — registering the issuer on every tx touching its asset
            // would be redundant with that index and would flood a popular
            // issuer's account page. Issuers still enter the `accounts` universe
            // via detected-asset extraction below.
        }
    }

    // Task 0383 (K2-7 + K3-4): decode SEP-41 / CAP-67 token events
    // (transfer / mint / burn / clawback) into participant + asset presence.
    //
    // Scope: only Soroban-context txs (`has_soroban`). Protocol 23 makes every
    // classic payment emit a SAC transfer event too, but those txs already
    // register their accounts + assets via the op path (0359) — measured 99.4%
    // of transfer events, with 670/670 participant coverage (see lore 0383
    // S-devils-advocate-findings). The net-new value is the contract-internal
    // flows a classic op never sees, i.e. exactly the `has_soroban` txs.
    // Presence only — no amount (the tx-detail page decodes it from archive XDR).
    for (tx_hash, evs) in events {
        if !has_soroban.get(tx_hash).copied().unwrap_or(false) {
            continue;
        }
        let entry = participants_per_tx.entry(tx_hash.clone()).or_default();
        let asset_entry = event_assets_per_tx.entry(tx_hash.clone()).or_default();
        for ev in evs {
            // Skip diagnostic-source events — they are host trace/simulation
            // output, not a real state change, and are dropped from the
            // persisted `soroban_events` (below). Filtering here keeps live
            // ingest byte-identical to the backfill (which reads `soroban_events`)
            // and never registers a participant/asset from a failed-call trace.
            if is_diagnostic(ev.source) {
                continue;
            }
            let Some(derived) =
                derive_token_event(&ev.topics, ev.contract_id.as_deref().map(ids::contract_id))
            else {
                continue;
            };
            for key in derived.participant_strkeys {
                account_keys.insert(key.clone());
                entry.insert(key);
            }
            if let Some(asset_id) = derived.asset_id {
                asset_entry.insert(asset_id);
            }
        }
    }

    for (tx_hash, invs) in invocations {
        let entry = participants_per_tx.entry(tx_hash.clone()).or_default();
        for inv in invs {
            if let Some(caller) = &inv.caller_account
                && is_strkey_account(caller)
            {
                account_keys.insert(caller.clone());
                entry.insert(caller.clone());
            }
        }
    }

    for dep in contract_deployments {
        if let Some(deployer) = &dep.deployer_account {
            account_keys.insert(deployer.clone());
        }
    }

    for st in account_states {
        account_keys.insert(st.account_id.clone());
        for b in st.balances.as_array().into_iter().flatten() {
            if let Some(issuer) = b.get("issuer").and_then(Value::as_str)
                && !issuer.is_empty()
            {
                account_keys.insert(issuer.to_string());
            }
        }
        for rm in &st.removed_trustlines {
            if let Some(issuer) = rm.get("issuer").and_then(Value::as_str)
                && !issuer.is_empty()
            {
                account_keys.insert(issuer.to_string());
            }
        }
    }
    for pool in liquidity_pools {
        if let Some(issuer) = asset_issuer(&pool.asset_a) {
            account_keys.insert(issuer);
        }
        if let Some(issuer) = asset_issuer(&pool.asset_b) {
            account_keys.insert(issuer);
        }
    }
    for asset in assets {
        if let Some(issuer) = &asset.issuer_address {
            account_keys.insert(issuer.clone());
        }
    }
    for nft in nfts {
        if let Some(owner) = &nft.owner_account {
            account_keys.insert(owner.clone());
        }
    }
    for ev in nft_events {
        if let Some(owner) = &ev.owner_account
            && is_strkey_account(owner)
        {
            account_keys.insert(owner.clone());
            participants_per_tx
                .entry(ev.transaction_hash.clone())
                .or_default()
                .insert(owner.clone());
        }
    }
    for lpp in lp_positions {
        account_keys.insert(lpp.account_id.clone());
    }

    let total = account_keys.len();
    account_keys.retain(|k| k.len() <= 56 && k.starts_with('G'));
    let dropped = total - account_keys.len();
    if dropped > 0 {
        tracing::debug!(
            ledger_sequence = ledger.sequence,
            dropped,
            kept = account_keys.len(),
            "dropped non-G-prefix or oversize StrKeys from CH accounts staging"
        );
    }

    let overrides = merge_account_state_overrides(account_states);

    for key in &account_keys {
        let ov = overrides.get(key);
        let last_seen_ledger = ov
            .and_then(|o| o.last_seen_ledger)
            .unwrap_or(ledger_sequence_i64);
        let first_seen_ledger = ov
            .and_then(|o| o.first_seen_ledger)
            .unwrap_or(last_seen_ledger);
        let sequence_number = match ov {
            Some(o) if o.sequence_number >= 0 => o.sequence_number,
            _ => 0,
        };
        out.account_rows.push(AccountRow {
            id: ids::account_id(key),
            account_id: key.clone(),
            first_seen_ledger,
            last_seen_ledger,
            sequence_number,
            home_domain: ov.and_then(|o| o.home_domain.clone()),
        });
    }

    // ---- wasm_interface_metadata (deduped by wasm_hash) ----
    //
    // Task 0118 Phase 2 (PG-side mirror) — run the wasm-spec classifier
    // alongside the metadata dedup. The resulting per-hash verdict
    // feeds two downstream consumers in this same `prepare` call:
    //   * `contract_rows.contract_type` override for non-SAC deploys
    //     whose WASM is uploaded in the same ledger (matches PG
    //     `Staged::prepare` behaviour at staging.rs:578-585).
    //   * NFT-candidate routing (`nft_rows` / `nft_pending_rows`,
    //     task 0217 / 0220) — `Other`/NULL verdict routes to
    //     quarantine; `Fungible` / `Token` drops the row entirely.
    let mut wasm_seen: HashSet<[u8; 32]> = HashSet::new();
    let mut wasm_classification: HashMap<[u8; 32], ContractType> =
        HashMap::with_capacity(contract_interfaces.len());
    for iface in contract_interfaces {
        let hash = decode_hash(&iface.wasm_hash, "wasm_hash")?;
        if !wasm_seen.insert(hash) {
            continue;
        }
        let classification = xdr_parser::classify_contract_from_wasm_spec(&iface.functions);
        wasm_classification.insert(hash, classification.into());

        // Task 0327: persist the mutability bit so the API can surface the
        // Upgradeable/Immutable badge. Read back via
        // `JSONExtractBool(metadata,'upgradeable')`; rows written before this
        // (no key) read as Unknown → chip renders nothing.
        let metadata = serde_json::json!({
            "functions": iface.functions,
            "wasm_byte_len": iface.wasm_byte_len,
            "upgradeable": iface.upgradeable,
        });
        out.wasm_rows.push(WasmInterfaceMetadataRow {
            wasm_hash: hash,
            metadata: serde_json::to_string(&metadata)
                .map_err(|e| staging_err(&format!("wasm metadata serialize: {e}")))?,
        });
    }

    // ---- soroban_contracts (deduped by contract_id) ----
    let mut contract_seen: HashSet<String> = HashSet::new();
    for dep in contract_deployments {
        if !contract_seen.insert(dep.contract_id.clone()) {
            continue;
        }
        let wasm_hash = match &dep.wasm_hash {
            Some(h) => Some(decode_hash(h, "deployment.wasm_hash")?),
            None => None,
        };
        let deployed = i64::from(dep.deployed_at_ledger);
        // Task 0118 Phase 2 (PG-side mirror) — if this deployment's
        // wasm carries a definitive `Nft` / `Fungible` verdict, override
        // the parser default (`Other` for non-SAC) before the row reaches
        // CH. SAC deploys stay `Token` (is_sac short-circuits WASM
        // classification — SACs have no WASM).
        //
        // Verdict source, in precedence order:
        //   1. `wasm_classification` — WASM uploaded in THIS ledger.
        //   2. `prior_wasm_verdicts` — WASM uploaded in an EARLIER ledger,
        //      pre-fetched by the writer from `wasm_interface_metadata`
        //      (task 0283 live G1). This is the common Soroban case
        //      (upload + deploy are separate txs / ledgers); without it
        //      the contract would persist `Other` and its NFT events would
        //      route to quarantine until the batch backstop drains them.
        let mut contract_type = dep.contract_type;
        if !dep.is_sac
            && let Some(hash) = wasm_hash
            && let Some(classified) = wasm_classification
                .get(&hash)
                .or_else(|| prior_wasm_verdicts.get(&hash))
                .copied()
            && matches!(classified, ContractType::Nft | ContractType::Fungible)
        {
            contract_type = classified;
        }
        out.contract_rows.push(SorobanContractRow {
            id: ids::contract_id(&dep.contract_id),
            contract_id: dep.contract_id.clone(),
            wasm_hash,
            wasm_uploaded_at_ledger: deployed,
            deployer_id: dep.deployer_account.as_deref().map(ids::account_id),
            deployed_at_ledger: Some(deployed),
            contract_type: Some(contract_type as i16),
            is_sac: dep.is_sac,
        });
    }

    // (task 0297) On-chain token name/symbol/decimals → the dedicated
    // `soroban_contract_metadata` side table. A pure 1:1 map of the producer's
    // output (extraction + SAC-skip already done), built here like the other
    // `out.*` rows rather than post-`prepare`. (The legacy `Symbol("name")` →
    // `soroban_contracts.name` write path was removed with Postgres in task
    // 0244; the dead-column DROP is task 0304 / 0310.)
    out.metadata_rows = build_metadata_rows(contract_metadata_writes);
    // `sac_map` (seeded above with this-ledger SAC carriers, before the value
    // reduction) re-keys a contract-held SAC balance onto its wrapped
    // classic/native asset.
    out.unified_balance_rows = build_balance_rows(soroban_token_balances, sac_map);

    // Task 0323 — un-deployed SACs are modelled as ASSETS, not contracts.
    // The `is_sac=true` skeleton `soroban_contracts` rows that task 0220 wrote
    // here are removed. `sac_overrides` (now the crypto-proven event emitters
    // from `detect_undeployed_sac_overrides`) instead (a) suppress the Pass-2 FK
    // stub below so no contract row is written, and (b) seed a SAC `assets` row
    // in the asset-emission pass. A real deploy still writes its contract row
    // from `contract_deployments` (site above).

    // ---- transactions + transaction_hash_index ----
    // `(surrogate id, application_order)` per hash: the surrogate keys joins,
    // the application order is the ledger's own temporal position — the ONLY
    // valid intra-ledger ordering (a hash surrogate sorts randomly; the task
    // 0374 e2e caught pool state picking an intermediate write as "last" on
    // 127 of 1,410 real pairs when ordered by tx_id).
    let mut tx_id_by_hash: HashMap<String, i64> = HashMap::with_capacity(transactions.len());
    for (idx, tx) in transactions.iter().enumerate() {
        let hash = decode_hash(&tx.hash, "tx.hash")?;
        let tx_id = ids::transaction_id(&hash);
        tx_id_by_hash.insert(tx.hash.clone(), tx_id);

        let inner_tx_hash = match tx.inner_tx_hash.as_deref() {
            Some(h) => Some(decode_hash(h, "inner_tx_hash")?),
            None => None,
        };
        let app_order =
            i16::try_from(idx + 1).map_err(|_| staging_err("application_order overflow (>i16)"))?;
        let op_count = op_count_by_tx.get(tx.hash.as_str()).copied().unwrap_or(0);

        out.transaction_rows.push(TransactionRow {
            id: tx_id,
            hash,
            ledger_sequence: ledger_sequence_i64,
            application_order: app_order,
            source_id: ids::account_id(&tx.source_account),
            fee_charged: tx.fee_charged,
            inner_tx_hash,
            successful: tx.successful,
            operation_count: op_count,
            has_soroban: *has_soroban.get(&tx.hash).unwrap_or(&false),
            parse_error: tx.parse_error,
        });

        out.hash_index_rows.push(TransactionHashIndexRow {
            hash,
            ledger_sequence: ledger_sequence_i64,
        });

        // Fee-bump: also index the inner-tx hash so a lookup by the inner
        // hash resolves to the wrapping fee-bump (Horizon `inner_transaction`
        // semantics). `inner_tx_hash → ledger_sequence` is immutable, same as
        // the outer key (task 0375).
        if let Some(inner) = inner_tx_hash {
            out.hash_index_rows.push(TransactionHashIndexRow {
                hash: inner,
                ledger_sequence: ledger_sequence_i64,
            });
        }
    }

    // ---- transaction_participants ----
    for tx in transactions {
        let Some(set) = participants_per_tx.get(&tx.hash) else {
            continue;
        };
        let Some(&tx_id) = tx_id_by_hash.get(&tx.hash) else {
            continue;
        };
        for key in set {
            if !is_strkey_account(key) {
                continue;
            }
            out.participant_rows.push(TransactionParticipantRow {
                account_id: ids::account_id(key),
                ledger_sequence: ledger_sequence_i64,
                transaction_id: tx_id,
            });
        }
    }

    // ---- liquidity_pools (dedup by pool_id, latest watermark) ----
    let mut pool_indices: HashMap<[u8; 32], usize> = HashMap::new();
    for pool in liquidity_pools {
        let pool_id = decode_hash(&pool.pool_id, "pool_id")?;
        let (Some((a_type, a_code, a_issuer)), Some((b_type, b_code, b_issuer))) = (
            split_pool_asset(&pool.asset_a),
            split_pool_asset(&pool.asset_b),
        ) else {
            continue;
        };
        let last_updated_ledger = i64::from(pool.last_updated_ledger);
        let asset_a_code = a_code.unwrap_or_default();
        let asset_a_issuer_id = a_issuer.as_deref().map(ids::account_id).unwrap_or(0);
        let asset_b_code = b_code.unwrap_or_default();
        let asset_b_issuer_id = b_issuer.as_deref().map(ids::account_id).unwrap_or(0);
        let new_row = LiquidityPoolRow {
            pool_id,
            // Legs migration step 2 (task 0374 committed follow-through):
            // classic rows fill `legs` too, so the pair columns can retire.
            // Classic legs are ASSET surrogates (`pool_leg_asset_id` — the
            // same key `lp_operation_amounts` joins on), NOT contract
            // surrogates like a soroban row's; `pool_kind` says which space.
            legs: vec![
                ids::pool_leg_asset_id(a_type as i16, &asset_a_code, asset_a_issuer_id),
                ids::pool_leg_asset_id(b_type as i16, &asset_b_code, asset_b_issuer_id),
            ],
            asset_a_type: a_type as i16,
            asset_a_code,
            asset_a_issuer_id,
            asset_b_type: b_type as i16,
            asset_b_code,
            asset_b_issuer_id,
            fee_bps: pool.fee_bps,
            last_updated_ledger,
            pool_kind: 0,
            deployment_id: 0,
            pool_type_raw: String::new(),
        };
        match pool_indices.get(&pool_id).copied() {
            Some(idx) => {
                let existing = &mut out.pool_rows[idx];
                if last_updated_ledger >= existing.last_updated_ledger {
                    existing.last_updated_ledger = last_updated_ledger;
                }
            }
            None => {
                pool_indices.insert(pool_id, out.pool_rows.len());
                out.pool_rows.push(new_row);
            }
        }
    }

    // One family seam (task 0518, decision 4a): partition the unified write
    // collection back into per-family views for the arms below. The views are
    // borrows — no clones, no reordering, the fold stays the single dedup
    // home downstream.
    let mut plane_pool_data: Vec<&xdr_parser::pool_state::ExtractedPlanePoolData> = Vec::new();
    let mut pool_instances: Vec<&xdr_parser::pool_state::ExtractedPoolInstance> = Vec::new();
    let mut factory_pairs: Vec<&xdr_parser::pool_pair_factory::ExtractedFactoryPair> = Vec::new();
    let mut config_pools: Vec<&xdr_parser::pool_config_factory::ExtractedConfigPool> = Vec::new();
    let mut address_lists: Vec<&xdr_parser::pool_config_factory::ExtractedAddressListWrite> =
        Vec::new();
    for write in pool_family_writes {
        match write {
            PoolFamilyWrite::RouterPlane(w) => plane_pool_data.push(w),
            PoolFamilyWrite::RouterPool(w) => pool_instances.push(w),
            PoolFamilyWrite::FactoryPair(w) => factory_pairs.push(w),
            PoolFamilyWrite::ConfigPool(w) => config_pools.push(w),
            PoolFamilyWrite::AddressList(w) => address_lists.push(w),
        }
    }

    // Soroban pool registrations (task 0374): the semantic decode lives in
    // `xdr_parser::pool_router` (same idiom as `detect_nft_events`); this
    // section only maps registrations to registry rows. Idempotent under the
    // RMT key (pool_id, version last_updated_ledger).
    //
    // A registration names its pool in the event DATA payload, which the
    // emitter chooses freely — so the claim is CORROBORATED against the named
    // pool's own instance storage before it becomes a row (review #438).
    // The pool contract is the authenticated owner of that entry and records
    // its `Router` there; the instance is written in the SAME transaction as
    // `add_pool` (probed on raw meta), so the corroborating fact is always in
    // this ledger's parse output. Without it, any contract could emit an
    // `add_pool` naming a REAL pool and replace its registry row wholesale
    // (RMT keyed on pool_id, versioned by ledger).
    //
    // An older contract version writes no `Router` key at all (read from chain
    // 2026-09-01: five of the ten deployments, 23 pools). Those instances are
    // real pools and cannot be authenticated — the chain simply never recorded
    // who registered them — so they are ACCEPTED and counted, never refused: a
    // missing key is an older contract, not a forgery, and dropping a real pool
    // is the failure this module's reject taxonomy exists to prevent.
    //
    // The acceptance is gated on the instance being CREATED in this ledger
    // (decision karolkow 2026-09-02): a genuine registration deploys and
    // initialises the pool in one transaction (497/497 measured, dead
    // deployments included), so its instance is always a creation. Without
    // the gate the arm was inducibly forgeable — one transaction could touch
    // an existing router-less pool (forcing an `updated` instance write) and
    // emit a forged `add_pool` naming it. A router-less registration whose
    // instance was merely touched is exactly that signature and is refused.
    // The map ORs `created` across the ledger's writes, so a creation
    // followed by a same-ledger update still qualifies.
    let declared_router: HashMap<&str, (Option<&str>, bool)> = {
        let mut m: HashMap<&str, (Option<&str>, bool)> = HashMap::new();
        for i in &pool_instances {
            let e = m.entry(i.state.pool.as_str()).or_insert((None, false));
            e.0 = i.state.router.as_deref();
            e.1 |= i.created;
        }
        m
    };
    for reg in xdr_parser::pool_router::detect_pool_registrations(events) {
        match declared_router.get(reg.event.pool.as_str()) {
            Some(&(Some(router), _)) if router == reg.router => {}
            Some(&(None, true)) => tracing::warn!(
                ledger_sequence = ledger.sequence,
                pool = %reg.event.pool,
                emitter = %reg.router,
                "add_pool registration accepted UNVERIFIED — the named pool's \
                 instance declares no router (older contract version) and was \
                 created in this ledger"
            ),
            Some(&(None, false)) => {
                tracing::warn!(
                    ledger_sequence = ledger.sequence,
                    pool = %reg.event.pool,
                    emitter = %reg.router,
                    "add_pool registration refused — router-less pool's \
                     instance was only TOUCHED this ledger, not created: the \
                     induced-forgery signature (no genuine registration ever \
                     looks like this)"
                );
                continue;
            }
            found => {
                tracing::warn!(
                    ledger_sequence = ledger.sequence,
                    pool = %reg.event.pool,
                    emitter = %reg.router,
                    declared_router = ?found.and_then(|f| f.0),
                    "add_pool registration refused — the named pool does not declare \
                     this emitter as its router"
                );
                continue;
            }
        }
        match pool_registry_row(&reg.event, &reg.router, ledger_sequence_i64) {
            Ok(row) => out.pool_rows.push(row),
            Err(reason) => tracing::error!(
                ledger_sequence = ledger.sequence,
                pool = %reg.event.pool,
                reason,
                "add_pool registration refused — a pool is missing from the registry"
            ),
        }
    }

    // Soroswap-family registrations (task 0518): the factory's `new_pair`,
    // corroborated by the PAIR's own instance — the pair records its
    // deploying factory at DataKey 4 in storage IT owns, and the factory
    // deploys + initialises the pair in the registering transaction, so a
    // genuine registration always has a same-ledger CREATED instance
    // pointing back at the emitter. Anything else — no instance, a foreign
    // factory, or a merely TOUCHED instance (the induced-forgery signature,
    // same class the router family's created gate closes) — is refused
    // loudly. No UNVERIFIED arm exists here: the factory pointer is part of
    // the recognition shape, so a pair without one is not a pair.
    // The map also carries the instance's OWN leg tokens: the event's legs
    // are a claim, the instance's are ledger-authenticated, and staging
    // compares them (review #447) — the same both-or-refuse discipline as
    // everywhere else, instead of trusting the claim because history showed
    // them equal.
    let declared_factory: HashMap<&str, (&str, bool, &str, &str)> = {
        let mut m: HashMap<&str, (&str, bool, &str, &str)> = HashMap::new();
        for sp in &factory_pairs {
            let e = m.entry(sp.state.pair.as_str()).or_insert((
                sp.state.factory.as_str(),
                false,
                sp.state.token_0.as_str(),
                sp.state.token_1.as_str(),
            ));
            e.0 = sp.state.factory.as_str();
            e.1 |= sp.created;
            e.2 = sp.state.token_0.as_str();
            e.3 = sp.state.token_1.as_str();
        }
        m
    };
    for reg in xdr_parser::pool_pair_factory::detect_pair_registrations(events) {
        match declared_factory.get(reg.event.pair.as_str()) {
            Some(&(factory, true, token_0, token_1))
                if factory == reg.factory
                    && token_0 == reg.event.token_0
                    && token_1 == reg.event.token_1 => {}
            found => {
                tracing::warn!(
                    ledger_sequence = ledger.sequence,
                    pair = %reg.event.pair,
                    emitter = %reg.factory,
                    declared = ?found,
                    "new_pair registration refused — the named pair's instance does \
                     not declare this emitter as its factory at creation, or the \
                     event's legs disagree with the instance's own"
                );
                continue;
            }
        }
        match factory_pair_registry_row(&reg, ledger_sequence_i64) {
            Ok(row) => out.pool_rows.push(row),
            Err(reason) => tracing::error!(
                ledger_sequence = ledger.sequence,
                pair = %reg.event.pair,
                reason,
                "new_pair registration refused — a pair is missing from the registry"
            ),
        }
    }

    // Config-factory registrations (task 0518, third adapter): the factory's
    // `create`/`liquidity_pool` event carries ONLY the pool address — legs,
    // fee, share token all come from the pool's own `CONFIG`. The pool
    // records NO factory back-pointer (read from chain 2026-09-03), so the
    // corroboration is the created gate plus a decodable full CONFIG written
    // by the pool itself in the registering LEDGER (a genuine registration
    // constructs the pool — config, reserves, shares — in the deploy
    // transaction; verified on the creation ledger raw meta; the maps here
    // aggregate per ledger, the granularity the sibling gates use too).
    //
    // Three forgery shapes, three answers: an event naming an EXISTING pool
    // fails the created gate; one naming a contract without the family
    // shape has no CONFIG to decode; and a second emitter co-claiming a
    // GENUINE pool inside its creation ledger — the one shape the pool-side
    // checks cannot arbitrate, because this family has no back-pointer and
    // both rows would carry the same RMT version — meets a TWO-STAGE gate
    // (review #447):
    //
    // 1. Membership list, POINTWISE: the emitter must have written the
    //    named pool into an address list in storage the EMITTER owns, this
    //    ledger (the genuine factory appends each pool to its own pools
    //    vector in the creation tx — read from raw meta). An event-only
    //    forgery dies here and the genuine registration SURVIVES.
    // 2. Conflict, BOTH refused: address-list content is still just data —
    //    a determined attacker can copy the pool's address into his own
    //    list — so if more than one emitter survives stage 1, every
    //    registration for that pool refuses loudly. With 20 registrations
    //    in all of history, a refused genuine one is loud and
    //    backfillable, while a nondeterministic `deployment_id` would be
    //    silent and permanent. (The cryptographic anchor — the pool
    //    address's deployer preimage — is NOT in the ledger meta; probed
    //    2026-09-05.)
    let declared_config: HashMap<&str, (Option<&PoolConfig>, bool)> = {
        let mut m: HashMap<&str, (Option<&PoolConfig>, bool)> = HashMap::new();
        for cp in &config_pools {
            let e = m.entry(cp.state.pool.as_str()).or_insert((None, false));
            if let Some(config) = cp.state.config.as_ref() {
                e.0 = Some(config);
            }
            e.1 |= cp.created;
        }
        m
    };
    // Stage-1 lookup: which pools did each emitter record in its OWN
    // storage this ledger?
    let listed_by_owner: HashMap<&str, HashSet<&str>> = {
        let mut m: HashMap<&str, HashSet<&str>> = HashMap::new();
        for al in &address_lists {
            m.entry(al.owner.as_str())
                .or_default()
                .extend(al.members.iter().map(String::as_str));
        }
        m
    };
    let config_regs: Vec<_> =
        xdr_parser::pool_config_factory::detect_config_pool_registrations(events)
            .into_iter()
            .filter(|reg| {
                let listed = listed_by_owner
                    .get(reg.factory.as_str())
                    .is_some_and(|pools| pools.contains(reg.pool.as_str()));
                if !listed {
                    tracing::warn!(
                        ledger_sequence = ledger.sequence,
                        pool = %reg.pool,
                        emitter = %reg.factory,
                        "create/liquidity_pool registration refused — the emitter \
                         did not record the named pool in its own storage this \
                         ledger (an event-only claim)"
                    );
                }
                listed
            })
            .collect();
    let mut config_emitters: HashMap<&str, Vec<&str>> = HashMap::new();
    for reg in &config_regs {
        config_emitters
            .entry(reg.pool.as_str())
            .or_default()
            .push(reg.factory.as_str());
    }
    let mut config_pools_staged: HashSet<&str> = HashSet::new();
    for reg in &config_regs {
        let emitters = &config_emitters[reg.pool.as_str()];
        if emitters.iter().any(|e| *e != reg.factory) {
            tracing::error!(
                ledger_sequence = ledger.sequence,
                pool = %reg.pool,
                emitters = ?emitters,
                "create/liquidity_pool registration refused for EVERY emitter — \
                 conflicting emitters claim one pool in one ledger, and the RMT \
                 version tie would pick the attribution nondeterministically; \
                 a pool is missing from the registry and needs the runbook's \
                 registration pass"
            );
            continue;
        }
        // Identical duplicates (one emitter announcing twice) collapse to
        // one row — a second identical push would only feed the same RMT
        // version tie with the same content.
        if !config_pools_staged.insert(reg.pool.as_str()) {
            continue;
        }
        match declared_config.get(reg.pool.as_str()) {
            Some(&(Some(config), true)) => {
                match config_pool_registry_row(reg, config, ledger_sequence_i64) {
                    Ok(row) => out.pool_rows.push(row),
                    Err(reason) => tracing::error!(
                        ledger_sequence = ledger.sequence,
                        pool = %reg.pool,
                        reason,
                        "create/liquidity_pool registration refused — a pool is \
                         missing from the registry"
                    ),
                }
            }
            found => {
                tracing::warn!(
                    ledger_sequence = ledger.sequence,
                    pool = %reg.pool,
                    emitter = %reg.factory,
                    declared = found.is_some(),
                    "create/liquidity_pool registration refused — the named pool \
                     did not write its own full CONFIG at creation in this ledger"
                );
                continue;
            }
        }
    }

    // Pool state from ledger entries (task 0374, step 7): plane writes are
    // THE reserve source; instance writes are the STATE source for share
    // tokens (the deposit⇄mint detector remains only as a cross-check, per
    // the same reasoning that demoted update_reserves in T4). Unparseable
    // coordinates are refused loudly — a silent skip is a pool going dark.
    //
    // Rows accumulate in locals and land in `out` FOLDED (one per key) — the
    // single dedup home for both soroban state tables (decision karolkow
    // 2026-09-01: fold in stage, symmetric for both, no parser-side pre-fold).
    let mut pool_state_rows: Vec<PoolStateChangeRow> = Vec::new();
    let mut instance_state_rows: Vec<PoolInstanceStateRow> = Vec::new();
    for pw in plane_pool_data {
        let Some(pool_id) = ids::contract_payload(&pw.data.pool) else {
            tracing::error!(
                ledger_sequence = pw.ledger_sequence,
                pool = %pw.data.pool,
                "plane write refused: pool address is not a valid C… strkey"
            );
            continue;
        };
        let Some(reserves) = parse_reserves(&pw.data.reserves) else {
            tracing::error!(
                ledger_sequence = pw.ledger_sequence,
                pool = %pw.data.pool,
                "plane write refused: non-numeric reserve — a snapshot is missing"
            );
            continue;
        };
        pool_state_rows.push(PoolStateChangeRow {
            pool_id,
            ledger_sequence: i64::from(pw.ledger_sequence),
            reserves,
            plane_id: ids::contract_id(&pw.data.plane),
        });
    }
    for inst in pool_instances {
        // A concentrated pool's per-operation reserves live in its INSTANCE
        // (Reserve0/Reserve1) — the plane is not updated per op for them
        // (measured; T4 refined). Fungible instances carry no `reserves`
        // here by construction, so no double-write against the plane rows.
        if !inst.state.reserves.is_empty() {
            match (
                ids::contract_payload(&inst.state.pool),
                parse_reserves(&inst.state.reserves),
                inst.state.plane.as_deref(),
            ) {
                (Some(pool_id), Some(reserves), Some(plane)) => {
                    pool_state_rows.push(PoolStateChangeRow {
                        pool_id,
                        ledger_sequence: i64::from(inst.ledger_sequence),
                        reserves,
                        // Never a placeholder 0: the plane is half of the
                        // provenance check AND part of the fold/sort key —
                        // a plane-less row would neither match the declared
                        // plane nor fold with rows that do. Refused below,
                        // same as the instance-state arm.
                        plane_id: ids::contract_id(plane),
                    });
                }
                _ => tracing::error!(
                    ledger_sequence = inst.ledger_sequence,
                    pool = %inst.state.pool,
                    "instance reserve write refused — bad pool address, \
                     non-numeric reserve, or no declared plane; a \
                     concentrated snapshot is missing"
                ),
            }
        }
        // What the pool declares about ITSELF. `plane_id` is written for
        // EVERY instance — it is the provenance authority reads check reserve
        // rows against (review #438) — while `share_token_id = 0` stays
        // structural for concentrated pools, which never mint one.
        match (
            ids::contract_payload(&inst.state.pool),
            inst.state.plane.as_deref(),
        ) {
            (Some(pool_id), Some(plane)) => {
                // A PRESENT-but-unparseable TotalShares is refused loudly —
                // writing 0 for it would be the misleading-fallback class
                // (0 must mean "key absent", never "we failed to read it").
                let total_shares = match inst.state.total_shares.as_deref() {
                    None => 0,
                    Some(raw) => match raw.parse::<i128>() {
                        Ok(v) => v,
                        Err(_) => {
                            tracing::error!(
                                ledger_sequence = inst.ledger_sequence,
                                pool = %inst.state.pool,
                                "instance write refused: non-numeric TotalShares"
                            );
                            continue;
                        }
                    },
                };
                instance_state_rows.push(PoolInstanceStateRow {
                    pool_id,
                    plane_id: ids::contract_id(plane),
                    share_token_id: inst
                        .state
                        .token_share
                        .as_deref()
                        .map(ids::contract_id)
                        .unwrap_or(0),
                    total_shares,
                    derived_at_ledger: i64::from(inst.ledger_sequence),
                });
            }
            _ => tracing::error!(
                ledger_sequence = inst.ledger_sequence,
                pool = %inst.state.pool,
                "instance write refused: pool address is not a valid C… strkey, \
                 or the instance declares no plane"
            ),
        }
    }
    // Soroswap pairs (task 0518): the pair's OWN instance is both the
    // reserve source and the declaration — owner, stamp and declaration
    // COINCIDE, so `plane_id` is the pair's own id: the read-side provenance
    // filter passes by construction, a forged foreign write still lands in
    // its own key space, and `share_token_id` is the pair too (it IS its own
    // SEP-41 LP token). `total_shares = 0` before the first mint is a TRUE
    // zero (nothing outstanding), not a fallback; a present-but-unparseable
    // value refuses the row loudly, same as every sibling arm.
    for sp in factory_pairs {
        let Some(pool_id) = ids::contract_payload(&sp.state.pair) else {
            tracing::error!(
                ledger_sequence = sp.ledger_sequence,
                pair = %sp.state.pair,
                "pair write refused: address is not a valid C… strkey"
            );
            continue;
        };
        let self_id = ids::contract_id(&sp.state.pair);
        if let Some((r0, r1)) = &sp.state.reserves {
            match parse_reserve_pair(r0, r1) {
                Some(reserves) => pool_state_rows.push(PoolStateChangeRow {
                    pool_id,
                    ledger_sequence: i64::from(sp.ledger_sequence),
                    reserves,
                    plane_id: self_id,
                }),
                None => tracing::error!(
                    ledger_sequence = sp.ledger_sequence,
                    pair = %sp.state.pair,
                    "pair reserve write refused: non-numeric reserve — a snapshot is missing"
                ),
            }
        }
        let Ok(total_shares) = parse_supply(sp.state.total_supply.as_deref()) else {
            tracing::error!(
                ledger_sequence = sp.ledger_sequence,
                pair = %sp.state.pair,
                "pair instance row refused: non-numeric TotalSupply (any reserve \
                 row from this write is already staged)"
            );
            continue;
        };
        instance_state_rows.push(PoolInstanceStateRow {
            pool_id,
            plane_id: self_id,
            share_token_id: self_id,
            total_shares,
            derived_at_ledger: i64::from(sp.ledger_sequence),
        });
    }
    // Config-factory pools (task 0518): the pool's own keyed persistent
    // entries are the reserve source — owner, stamp and declaration
    // COINCIDE (the entries are ledger-authenticated to the pool), so
    // `plane_id` is the pool's own id, same construction as the
    // pair-factory arm.
    //
    // The INSTANCE row is emitted ONLY when the transaction wrote `CONFIG`
    // (creation + admin config changes): `pool_instance_state` is RMT
    // whole-row keyed on `pool_id` alone, and a per-operation TotalShares
    // write arrives WITHOUT the config, so staging it would clobber
    // `share_token_id` to 0 — the misleading-fallback class. Consequence:
    // for THIS family the row's `total_shares` is STRUCTURALLY 0 forever
    // (TotalShares never co-occurs with a post-creation CONFIG write) — it
    // is not a snapshot of anything; the LIVE supply is the share token's
    // own tracked supply — the share token is a separate SEP-41 contract
    // whose mints/burns ride the generic token pipeline. The read half must
    // branch on family (`share_token_id != pool_id` + deployment) and never
    // render this 0 as a measured value.
    for cp in config_pools {
        let Some(pool_id) = ids::contract_payload(&cp.state.pool) else {
            tracing::error!(
                ledger_sequence = cp.ledger_sequence,
                pool = %cp.state.pool,
                "config-pool write refused: address is not a valid C… strkey"
            );
            continue;
        };
        let self_id = ids::contract_id(&cp.state.pool);
        if let Some((ra, rb)) = &cp.state.reserves {
            match parse_reserve_pair(ra, rb) {
                Some(reserves) => pool_state_rows.push(PoolStateChangeRow {
                    pool_id,
                    ledger_sequence: i64::from(cp.ledger_sequence),
                    reserves,
                    plane_id: self_id,
                }),
                None => tracing::error!(
                    ledger_sequence = cp.ledger_sequence,
                    pool = %cp.state.pool,
                    "config-pool reserve write refused: non-numeric reserve — a \
                     snapshot is missing"
                ),
            }
        }
        if let Some(config) = &cp.state.config {
            // A PRESENT-but-unparseable TotalShares refuses the row loudly,
            // same as every sibling arm; absent means the config write did
            // not touch it (0 stays the structural creation value).
            let Ok(total_shares) = parse_supply(cp.state.total_shares.as_deref()) else {
                tracing::error!(
                    ledger_sequence = cp.ledger_sequence,
                    pool = %cp.state.pool,
                    "config-pool instance row refused: non-numeric TotalShares \
                     (any reserve row from this write is already staged)"
                );
                continue;
            };
            instance_state_rows.push(PoolInstanceStateRow {
                pool_id,
                plane_id: self_id,
                share_token_id: ids::contract_id(&config.share_token),
                total_shares,
                derived_at_ledger: i64::from(cp.ledger_sequence),
            });
        }
    }

    // ONE row per key for both state tables — the folds' doc comments carry
    // the key rationale (plane in the state key; version ties within a
    // ledger for the instance table).
    out.pool_state_change_rows = fold_pool_state_changes(pool_state_rows);
    out.pool_instance_state_rows = fold_pool_instance_state(instance_state_rows);

    // ---- liquidity_pool_snapshots ----
    // Per-(pool, ledger) asset-A trade volume from claim atoms (0261 extractor).
    // Live ingest now derives it directly (previously backfill-only); the 0266
    // worker reuses this same value via `prepare`, so live + backfill agree.
    let gross_volume_by_pool = gross_volume_a_by_pool(operations);
    for snap in pool_snapshots {
        let pool_id = decode_hash(&snap.pool_id, "snapshot.pool_id")?;
        let reserve_a = snap
            .reserves
            .get("a")
            .and_then(Value::as_i64)
            .map(i128::from)
            .unwrap_or(0);
        let reserve_b = snap
            .reserves
            .get("b")
            .and_then(Value::as_i64)
            .map(i128::from)
            .unwrap_or(0);
        out.snapshot_rows.push(LiquidityPoolSnapshotRow {
            pool_id,
            ledger_sequence: i64::from(snap.ledger_sequence),
            reserve_a,
            reserve_b,
            total_shares: decimal7_string_to_i128(&snap.total_shares)?,
            // Asset-A-side trade volume for this (pool, ledger) from claim
            // atoms (0261). `None` when the pool had no trade this ledger.
            // USD tvl/volume/fee_revenue have NO columns here any more: they
            // were written as NULL since 0199 (compute-at-read, ADR 0053) and
            // read by nothing — dropped from the write path in 0374's
            // distillation; prod drops them with ALTER … DROP COLUMN.
            gross_volume_a: gross_volume_by_pool.get(&pool_id).copied(),
        });
    }

    // ---- lp_positions (dedup by (pool_id, account_id)) ----
    use std::collections::hash_map::Entry;
    let mut lp_dedup: HashMap<([u8; 32], i64), LpPositionRow> = HashMap::new();
    for pos in lp_positions {
        let pool_id = decode_hash(&pos.pool_id, "lp_position.pool_id")?;
        let acct_id = ids::account_id(&pos.account_id);
        let last = i64::from(pos.last_updated_ledger);
        let first = pos.first_deposit_ledger.map(i64::from).unwrap_or(last);
        let new_row = LpPositionRow {
            pool_id,
            account_id: acct_id,
            shares: decimal7_string_to_i128(&pos.shares)?,
            first_deposit_ledger: first,
            last_updated_ledger: last,
            // The pool-share trustline was removed — the participant left the
            // pool, as opposed to withdrawing to zero and staying. ADR 0055.
            closed_at_ledger: if pos.closed { last } else { 0 },
        };
        match lp_dedup.entry((pool_id, acct_id)) {
            Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                if new_row.last_updated_ledger >= existing.last_updated_ledger {
                    let preserved_first = existing
                        .first_deposit_ledger
                        .min(new_row.first_deposit_ledger);
                    *existing = new_row;
                    existing.first_deposit_ledger = preserved_first;
                } else {
                    existing.first_deposit_ledger = existing
                        .first_deposit_ledger
                        .min(new_row.first_deposit_ledger);
                }
            }
            Entry::Vacant(vac) => {
                vac.insert(new_row);
            }
        }
    }
    out.lp_position_rows.extend(lp_dedup.into_values());

    // ---- operations_appearances (identity fold per task 0163) ----
    #[derive(Eq, PartialEq, Hash)]
    struct OpKey {
        tx_hash_hex: String,
        op_type: i16,
        source_account: Option<String>,
        destination_account: Option<String>,
        contract_strkey: Option<String>,
        asset_code: String,
        asset_issuer_account: Option<String>,
        /// Sorted + deduped — canonical order makes the fold identity (and
        /// the emitted row) deterministic across re-parses (task 0261/0266).
        pool_ids: Vec<[u8; 32]>,
    }
    struct OpAgg {
        count: i64,
        min_apply_order: u32,
    }
    let mut op_agg: HashMap<OpKey, OpAgg> = HashMap::new();
    for (tx_hash, ops) in operations {
        if !tx_id_by_hash.contains_key(tx_hash) {
            continue;
        }
        // Per-tx dedup for the asset fan-out (PR #6): N ops touching the same
        // asset in one tx would otherwise write N identical (asset, tx) rows. The
        // RMT sort key collapses them eventually, but deduping at write cuts the
        // backfilled volume up front. Scoped per tx — one entry per tx_hash here.
        let mut seen_tx_asset_ids: HashSet<i64> = HashSet::new();
        // Same per-tx dedup for the pool fan-out (task 0365): N ops crossing the
        // same pool in one tx → one (pool, tx) row.
        let mut seen_tx_pool_ids: HashSet<[u8; 32]> = HashSet::new();
        for op in ops {
            // ---- operation_asset_appearances (task 0359, pure presence) ----
            // Asset-dimension twin of transaction_participants: one row per
            // (asset the op touches, tx). Native is a FIRST-CLASS surrogate (never
            // the empty-string sentinel); classic credit hashes
            // code:issuer_surrogate — both via `ids::asset_id`.
            if !op.asset_appearances.is_empty() {
                let tx_id = tx_id_by_hash[tx_hash];
                for asset in &op.asset_appearances {
                    let asset_id = match asset {
                        AssetRef::Native => ids::NATIVE_ASSET_ID,
                        AssetRef::Credit { code, issuer } => ids::credit_asset_id(code, issuer),
                    };
                    if seen_tx_asset_ids.insert(asset_id) {
                        out.op_asset_rows.push(OperationAssetAppearanceRow {
                            asset_id,
                            ledger_sequence: ledger_sequence_i64,
                            transaction_id: tx_id,
                            // `Some(v)` = reduced; `None` (-> NULL) = touched but
                        });
                    }
                }
            }

            let typed = OpTyped::from_details(op.op_type, &op.details);
            let mut pool_ids = Vec::with_capacity(typed.pool_ids_hex.len());
            for h in &typed.pool_ids_hex {
                pool_ids.push(decode_hash(h, "op.pool_ids")?);
            }
            pool_ids.sort_unstable();
            pool_ids.dedup();

            // ---- operation_pools (task 0365, pure presence) ----
            // Pool-dimension twin of the asset fan-out above: one row per (pool
            // the op crossed, tx). `pool_ids` is already the sorted+deduped
            // crossing list; dedup per-tx so N ops crossing the same pool in one
            // tx write one (pool, tx) row (the RMT collapses any residual). Sourced
            // from `oa.pool_ids` — no XDR-only data, so a plain CH re-key can
            // backfill it (task 0365 Path B).
            if !pool_ids.is_empty() {
                let tx_id = tx_id_by_hash[tx_hash];
                for pool_id in &pool_ids {
                    if seen_tx_pool_ids.insert(*pool_id) {
                        out.op_pool_rows.push(OperationPoolRow {
                            pool_id: *pool_id,
                            ledger_sequence: ledger_sequence_i64,
                            transaction_id: tx_id,
                        });
                    }
                }
            }

            // ---- lp_operation_amounts (task 0279) ----
            // The value twin of the block above: `gross_volume_a_by_pool` walks
            // the same trade atoms and sums them into one number per pool; here
            // the per-(op, pool, asset) attribution is KEPT instead of
            // discarded, and deposits/withdrawals — which have no atoms — come
            // from the op's own reserve delta.
            {
                let tx_id = tx_id_by_hash[tx_hash];
                // Fail the ledger rather than clamp, matching the
                // `transactions.application_order` conversion above: this
                // column is part of the ORDER BY, so two operations squeezed
                // onto one saturated value would share a key and the RMT would
                // drop a fill silently — the loss the per-op summing exists to
                // prevent. Unreachable while Stellar caps ops per tx at 100.
                let order = i16::try_from(op.operation_index)
                    .map_err(|_| staging_err("lp_operation_amounts application_order (>i16)"))?;
                for (pool_id, asset_id, amount) in pool_fill_amounts(&op.details) {
                    out.lp_amount_rows.push(LpOperationAmountRow {
                        pool_id,
                        ledger_sequence: ledger_sequence_i64,
                        transaction_id: tx_id,
                        application_order: order,
                        asset_id,
                        amount,
                    });
                }
            }

            let key = OpKey {
                tx_hash_hex: tx_hash.clone(),
                op_type: op.op_type as i16,
                source_account: op.source_account.clone(),
                destination_account: typed.destination,
                contract_strkey: typed.contract_id,
                asset_code: typed.asset_code.unwrap_or_default(),
                asset_issuer_account: typed.asset_issuer,
                pool_ids,
            };
            op_agg
                .entry(key)
                .and_modify(|agg| {
                    agg.count += 1;
                    agg.min_apply_order = agg.min_apply_order.min(op.operation_index);
                })
                .or_insert(OpAgg {
                    count: 1,
                    min_apply_order: op.operation_index,
                });
        }
    }
    for (k, agg) in op_agg {
        let Some(&tx_id) = tx_id_by_hash.get(&k.tx_hash_hex) else {
            continue;
        };
        let app_order = i16::try_from(agg.min_apply_order)
            .map_err(|_| staging_err("operation_index >i16 — protocol violation"))?;
        out.op_rows.push(OperationAppearanceRow {
            transaction_id: tx_id,
            application_order: app_order,
            op_type: k.op_type,
            source_id: k.source_account.as_deref().map(ids::account_id),
            destination_id: k.destination_account.as_deref().map(ids::account_id),
            contract_id: k.contract_strkey.as_deref().map(ids::contract_id),
            asset_code: k.asset_code,
            asset_issuer_id: k.asset_issuer_account.as_deref().map(ids::account_id),
            pool_ids: k.pool_ids,
            amount: agg.count,
            ledger_sequence: ledger_sequence_i64,
        });
    }

    // ---- operation_asset_appearances: event-derived (task 0383, K3-4) ----
    // SAC / bespoke token moves (transfer / mint / burn / clawback) make the
    // moved asset appear in the tx. Same (asset, tx) grain as the op-derived
    // rows above; the RMT collapses any overlap. Presence only (model A).
    for (tx_hash, asset_ids) in &event_assets_per_tx {
        let Some(&tx_id) = tx_id_by_hash.get(tx_hash) else {
            continue;
        };
        for &asset_id in asset_ids {
            out.op_asset_rows.push(OperationAssetAppearanceRow {
                asset_id,
                ledger_sequence: ledger_sequence_i64,
                transaction_id: tx_id,
            });
        }
    }

    // ---- soroban_events (UNFOLDED per ADR 0044 §4a) ----
    let mut diagnostic_dropped: usize = 0;
    let mut contract_orphan_dropped: usize = 0;
    for (tx_hash, evs) in events {
        let Some(&tx_id) = tx_id_by_hash.get(tx_hash) else {
            continue;
        };
        for ev in evs {
            if is_diagnostic(ev.source) {
                diagnostic_dropped += 1;
                continue;
            }
            let Some(contract_strkey) = &ev.contract_id else {
                contract_orphan_dropped += 1;
                continue;
            };
            let event_index = i16::try_from(ev.event_index)
                .map_err(|_| staging_err("event_index overflow (>i16)"))?;
            let topics_xdr = serde_json::to_string(&ev.topics)
                .map_err(|e| staging_err(&format!("event topics serialize: {e}")))?;
            let data_xdr = serde_json::to_string(&ev.data)
                .map_err(|e| staging_err(&format!("event data serialize: {e}")))?;
            let signature = extract_event_signature(&ev.topics);
            out.event_rows.push(SorobanEventRow {
                contract_id: ids::contract_id(contract_strkey),
                transaction_id: tx_id,
                ledger_sequence: ledger_sequence_i64,
                event_index,
                event_type: ev.event_type as i16,
                signature,
                topics_xdr,
                data_xdr,
            });
        }
    }
    if diagnostic_dropped > 0 || contract_orphan_dropped > 0 {
        tracing::debug!(
            ledger_sequence = ledger.sequence,
            diagnostic_dropped,
            contract_orphan_dropped,
            staged = out.event_rows.len(),
            "CH soroban_events filtered"
        );
    }

    // ---- soroban_invocations_appearances (ADR 0034 fold) ----
    #[derive(Eq, PartialEq, Hash)]
    struct InvKey {
        contract_strkey: String,
        tx_hash_hex: String,
    }
    struct InvAgg {
        amount: i32,
        caller_account: Option<String>,
        caller_contract_strkey: Option<String>,
    }
    let mut inv_agg: HashMap<InvKey, InvAgg> = HashMap::new();
    for (tx_hash, invs) in invocations {
        if !tx_id_by_hash.contains_key(tx_hash) {
            continue;
        }
        for inv in invs {
            let Some(contract) = &inv.contract_id else {
                continue;
            };
            let (caller_account, caller_contract) = match inv.caller_account.as_deref() {
                Some(k) if is_strkey_account(k) => (Some(k.to_string()), None),
                Some(k) if k.starts_with('C') => (None, Some(k.to_string())),
                _ => (None, None),
            };
            let key = InvKey {
                contract_strkey: contract.clone(),
                tx_hash_hex: tx_hash.clone(),
            };
            inv_agg
                .entry(key)
                .and_modify(|agg| {
                    agg.amount = agg.amount.saturating_add(1);
                    if agg.caller_account.is_none() && agg.caller_contract_strkey.is_none() {
                        agg.caller_account = caller_account.clone();
                        agg.caller_contract_strkey = caller_contract.clone();
                    }
                })
                .or_insert(InvAgg {
                    amount: 1,
                    caller_account,
                    caller_contract_strkey: caller_contract,
                });
        }
    }
    for (k, agg) in inv_agg {
        let Some(&tx_id) = tx_id_by_hash.get(&k.tx_hash_hex) else {
            continue;
        };
        out.invocation_rows.push(SorobanInvocationAppearanceRow {
            contract_id: ids::contract_id(&k.contract_strkey),
            transaction_id: tx_id,
            ledger_sequence: ledger_sequence_i64,
            caller_id: agg.caller_account.as_deref().map(ids::account_id),
            caller_contract_id: agg.caller_contract_strkey.as_deref().map(ids::contract_id),
            amount: agg.amount,
        });
    }

    // ---- assets identity rows (dedup by 4-tuple) + asset_sac facet rows ----
    //
    // ADR 0051: the classic_credit / native `assets` row is the asset's identity;
    // the SAC handle is a FACET written to the `asset_sac` side table — NOT a
    // column on `assets`, which is a no-version RMT re-written whole every ledger
    // a trustline for the asset changes (`detect_classic_credit_assets`) and would
    // clobber a mutable non-key column to its default on the next re-emit (the
    // ~25% NULL prod bug). `push_asset` dedups identity by the 4-tuple; `push_sac`
    // collects one facet per key, `max`-merging `sac_deployed` (a deploy sticks
    // over a later un-deployed override) — mirrored by the `asset_sac`
    // AggregatingMergeTree so the fold is correct cross-ledger too, not just
    // within one staged batch.
    let mut asset_seen: HashSet<(i16, String, i64, i64)> = HashSet::new();
    let push_asset =
        |out: &mut StagedLedger, seen: &mut HashSet<(i16, String, i64, i64)>, row: AssetRow| {
            // Dedup by the identity 4-tuple. `row.id` is already the `ids::asset_id`
            // surrogate (computed in `AssetRow::staged`), so build sites can't diverge.
            let key = (
                row.asset_type,
                row.asset_code.clone(),
                row.issuer_id,
                row.contract_id,
            );
            if seen.insert(key) {
                out.asset_rows.push(row);
            }
        };
    // Facet accumulator: (asset_type, code, issuer_id, contract_id=0) → (surrogate, deployed).
    let mut sac_facets: HashMap<(i16, String, i64), (i64, u8)> = HashMap::new();
    let mut push_sac = |asset_type: i16,
                        asset_code: String,
                        issuer_id: i64,
                        sac_contract_id: i64,
                        deployed: u8| {
        let e = sac_facets
            .entry((asset_type, asset_code, issuer_id))
            .or_insert((0, 0));
        if sac_contract_id != 0 {
            e.0 = sac_contract_id;
        }
        e.1 = e.1.max(deployed);
    };
    for t in assets {
        let issuer_id = t
            .issuer_address
            .as_deref()
            .map(ids::account_id)
            .unwrap_or(0);
        let contract_id_int = t.contract_id.as_deref().map(ids::contract_id).unwrap_or(0);
        let row = AssetRow::staged(
            t.asset_type as i16,
            t.asset_code.clone().unwrap_or_default(),
            issuer_id,
            contract_id_int,
        );
        // SAC facet carried by `detect_assets`' SAC branch (the classic/native
        // carrier for a deploy). Emitted to `asset_sac`, never onto `assets`.
        if let Some(sac) = t.sac_contract_id.as_deref() {
            push_sac(
                t.asset_type as i16,
                row.asset_code.clone(),
                issuer_id,
                ids::contract_id(sac),
                t.sac_deployed as u8,
            );
        }
        push_asset(&mut out, &mut asset_seen, row);
    }

    // SAC deploys (ADR 0051): ensure the underlying classic_credit / native
    // identity row exists and record the SAC facet (`sac_deployed = 1`). Never a
    // separate `type=2` row. `detect_assets` already emits the same carrier from
    // these deployments; the dedup collapses the two, and this also covers callers
    // that pass deployments without running `detect_assets`.
    for dep in contract_deployments {
        let Some(sac) = &dep.sac_asset else {
            continue;
        };
        let (asset_type, asset_code, issuer_id) = match sac {
            SacAssetIdentity::Native => (domain::AssetFamily::Native, String::new(), 0),
            SacAssetIdentity::Credit { code, issuer } => (
                domain::AssetFamily::ClassicCredit,
                code.clone(),
                ids::account_id(issuer),
            ),
        };
        push_sac(
            asset_type as i16,
            asset_code.clone(),
            issuer_id,
            ids::contract_id(&dep.contract_id),
            1,
        );
        push_asset(
            &mut out,
            &mut asset_seen,
            AssetRow::staged(
                asset_type as i16,
                asset_code,
                issuer_id,
                0, // key reserved for soroban identity
            ),
        );
    }

    // Un-deployed-SAC facet (task 0323 AC#3 → ADR 0051). The crypto-proven event
    // emitters in `sac_overrides` (see `detect_undeployed_sac_overrides`) get NO
    // contract row (suppressed in Pass-2 below); fold the SAC handle onto the
    // underlying classic_credit / native row with `sac_deployed = false` so its
    // activity has a home while it stays a reserved-but-un-deployed address.
    // `ov.identity` carries the classic asset. A deployed SAC (e.g. USDC) that
    // also emits is harmless — `push_asset`'s merge keeps `sac_deployed = true`
    // from the real deploy row (OR-merge never downgrades it).
    for ov in sac_overrides {
        let (asset_type, asset_code, issuer_id) = match &ov.identity {
            SacAssetIdentity::Native => (domain::AssetFamily::Native, String::new(), 0),
            SacAssetIdentity::Credit { code, issuer } => (
                domain::AssetFamily::ClassicCredit,
                code.clone(),
                ids::account_id(issuer),
            ),
        };
        push_sac(
            asset_type as i16,
            asset_code.clone(),
            issuer_id,
            ids::contract_id(&ov.contract_id),
            0,
        );
        push_asset(
            &mut out,
            &mut asset_seen,
            AssetRow::staged(
                asset_type as i16,
                asset_code,
                issuer_id,
                0, // key reserved for soroban identity
            ),
        );
    }

    // Native XLM singleton (PG sqlx migration 20260428000000 analog).
    push_asset(
        &mut out,
        &mut asset_seen,
        AssetRow::staged(domain::AssetFamily::Native as i16, String::new(), 0, 0),
    );

    // ---- assets type-3 for WASM-classified Soroban fungibles (task 0283 G2) --
    //
    // Mirror of PG `insert_assets_from_reclassified_contracts`: a contract
    // whose verdict resolved to `Fungible` (same-ledger classification or the
    // writer's prior-ledger prefetch via the deploy override above) gets a
    // bespoke-Soroban (`asset_type = 3`) asset row carrying only the
    // `contract_id` — code/issuer are empty, aggregates are filled later by the
    // `asset-aggregates` batch. `push_asset` dedups against any row the parser
    // already emitted same-batch; SAC short-circuits `Fungible`, so these never
    // collide with a SAC-carrying classic/native row. Read from the staged
    // `contract_rows` so the corrected verdict (incl. the prior-ledger override)
    // is honoured.
    let fungible_contract_ids: Vec<i64> = out
        .contract_rows
        .iter()
        .filter(|r| !r.is_sac && r.contract_type == Some(ContractType::Fungible as i16))
        .map(|r| r.id)
        .collect();
    for contract_id in fungible_contract_ids {
        push_asset(
            &mut out,
            &mut asset_seen,
            AssetRow::staged(
                domain::AssetFamily::Soroban as i16,
                String::new(),
                0,
                contract_id,
            ),
        );
    }

    // Materialise the accumulated SAC facets into `asset_sac` rows (contract_id
    // is 0 — the classic/native carrier's key). AggregatingMergeTree `max`-merges
    // these with any prior-ledger rows, so `sac_deployed` is monotonic.
    for ((asset_type, asset_code, issuer_id), (sac_contract_id, sac_deployed)) in sac_facets {
        out.asset_sac_rows.push(AssetSacRow {
            asset_type,
            asset_code,
            issuer_id,
            contract_id: 0,
            sac_contract_id,
            sac_deployed,
        });
    }

    // ---- NFT routing verdict map (task 0217 / 0220) -------------------
    //
    // Build a per-contract verdict map keyed by strkey. Sources, in
    // precedence order:
    //   1. Same-ledger `contract_rows` carrying a definitive
    //      `contract_type` (Token / Nft / Fungible). Either:
    //        - SAC deploy (`is_sac=true` → Token).
    //        - WASM-classified deploy (the override applied above).
    //   2. SAC overrides (also Token) — these were skipped from Pass-2
    //      stubs, so they're in `out.contract_rows` already.
    // Contracts with NO entry in EITHER source → treat as `Other`/uncached →
    // route to pending. The stage itself has no DB access; cross-ledger
    // verdicts arrive via `prior_contract_verdicts` (task 0283 live G9), the
    // writer's lookup of `soroban_contracts` for contracts emitting NFT
    // rows/events here but deployed earlier. This restores the PG
    // `ClassificationCache` semantic the CH cutover dropped — without it a
    // later transfer from an already-classified NFT would quarantine.
    let mut verdict_by_contract: HashMap<&str, ContractType> = HashMap::new();
    for row in &out.contract_rows {
        if let Some(ty_i16) = row.contract_type
            && let Ok(ty) = ContractType::try_from(ty_i16)
        {
            verdict_by_contract.insert(row.contract_id.as_str(), ty);
        }
    }

    // 3-way routing helper. Mirrors PG `resolve_nft_filter` bucketing.
    // This-ledger `contract_rows` take precedence; `prior_contract_verdicts`
    // (G9, cross-ledger) is the fallback for contracts not deployed here.
    enum NftRoute {
        Hot,
        Pending,
        Drop,
    }
    let route_for = |strkey: &str| -> NftRoute {
        let verdict = verdict_by_contract
            .get(strkey)
            .copied()
            .or_else(|| prior_contract_verdicts.get(strkey).copied());
        match verdict {
            Some(ContractType::Token) | Some(ContractType::Fungible) => NftRoute::Drop,
            Some(ContractType::Nft) => NftRoute::Hot,
            // `Other` and uncached (no entry in either source) both go to
            // quarantine — same semantic as PG-side `resolve_nft_filter`.
            _ => NftRoute::Pending,
        }
    };

    // ---- nfts / nfts_pending (dedup by (contract_id, token_id),
    //                            latest watermark) ----
    //
    // Each `(contract_id, token_id)` row lives in exactly one bucket
    // (hot OR pending) per partition — picked by the per-contract
    // verdict above. Dedup keys are per-bucket so a contract that
    // somehow appeared with mixed verdicts within the same ledger
    // (impossible today, defensive) would have separate slots.
    let mut nft_hot_indices: HashMap<(i64, String), usize> = HashMap::new();
    let mut nft_pending_indices: HashMap<(i64, String), usize> = HashMap::new();
    for nft in nfts {
        let route = route_for(nft.contract_id.as_str());
        if matches!(route, NftRoute::Drop) {
            continue;
        }
        let contract_id_int = ids::contract_id(&nft.contract_id);
        let watermark = i64::from(nft.last_seen_ledger);
        let key = (contract_id_int, nft.token_id.clone());
        let owner_id = nft.owner_account.as_deref().map(ids::account_id);
        let minted = nft.minted_at_ledger.map(i64::from);

        match route {
            NftRoute::Hot => match nft_hot_indices.get(&key).copied() {
                Some(idx) => {
                    let existing = &mut out.nft_rows[idx];
                    if watermark >= existing.current_owner_ledger {
                        existing.current_owner_id = owner_id;
                        existing.current_owner_ledger = watermark;
                    }
                    existing.minted_at_ledger = match (existing.minted_at_ledger, minted) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (Some(a), None) => Some(a),
                        (None, b) => b,
                    };
                    existing.collection_name = existing
                        .collection_name
                        .clone()
                        .or_else(|| nft.collection_name.clone());
                    existing.name = existing.name.clone().or_else(|| nft.name.clone());
                    existing.media_url =
                        existing.media_url.clone().or_else(|| nft.media_url.clone());
                }
                None => {
                    nft_hot_indices.insert(key, out.nft_rows.len());
                    out.nft_rows.push(NftRow {
                        contract_id: contract_id_int,
                        token_id: nft.token_id.clone(),
                        collection_name: nft.collection_name.clone(),
                        name: nft.name.clone(),
                        media_url: nft.media_url.clone(),
                        minted_at_ledger: minted,
                        current_owner_id: owner_id,
                        current_owner_ledger: watermark,
                    });
                }
            },
            NftRoute::Pending => match nft_pending_indices.get(&key).copied() {
                Some(idx) => {
                    let existing = &mut out.nft_pending_rows[idx];
                    if watermark >= existing.current_owner_ledger {
                        existing.current_owner_id = owner_id;
                        existing.current_owner_ledger = watermark;
                    }
                    existing.minted_at_ledger = match (existing.minted_at_ledger, minted) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (Some(a), None) => Some(a),
                        (None, b) => b,
                    };
                    existing.collection_name = existing
                        .collection_name
                        .clone()
                        .or_else(|| nft.collection_name.clone());
                    existing.name = existing.name.clone().or_else(|| nft.name.clone());
                    existing.media_url =
                        existing.media_url.clone().or_else(|| nft.media_url.clone());
                }
                None => {
                    nft_pending_indices.insert(key, out.nft_pending_rows.len());
                    out.nft_pending_rows.push(NftPendingRow {
                        contract_id: contract_id_int,
                        token_id: nft.token_id.clone(),
                        collection_name: nft.collection_name.clone(),
                        name: nft.name.clone(),
                        media_url: nft.media_url.clone(),
                        minted_at_ledger: minted,
                        current_owner_id: owner_id,
                        current_owner_ledger: watermark,
                    });
                }
            },
            NftRoute::Drop => unreachable!("filtered above"),
        }
    }

    // ---- nft_ownership / nft_ownership_pending ----
    for ev in nft_events {
        let route = route_for(ev.contract_id.as_str());
        if matches!(route, NftRoute::Drop) {
            continue;
        }
        let Some(&tx_id) = tx_id_by_hash.get(&ev.transaction_hash) else {
            continue;
        };
        let event_order =
            i16::try_from(ev.event_order).map_err(|_| staging_err("nft event_order overflow"))?;
        let contract_id = ids::contract_id(&ev.contract_id);
        let ledger_sequence = i64::from(ev.ledger_sequence);
        let owner_id = ev.owner_account.as_deref().map(ids::account_id);
        let event_type = ev.event_type as i16;

        match route {
            NftRoute::Hot => out.nft_ownership_rows.push(NftOwnershipRow {
                contract_id,
                token_id: ev.token_id.clone(),
                ledger_sequence,
                event_order,
                transaction_id: tx_id,
                owner_id,
                event_type,
            }),
            NftRoute::Pending => out.nft_ownership_pending_rows.push(NftOwnershipPendingRow {
                contract_id,
                token_id: ev.token_id.clone(),
                ledger_sequence,
                event_order,
                transaction_id: tx_id,
                owner_id,
                event_type,
            }),
            NftRoute::Drop => unreachable!("filtered above"),
        }
    }

    // ---- unified `balances` — classic + native per-account balances (lore-0331
    // Option A single-write). `account_balances_current` is no longer written (only
    // its table remains, for the pending classic→`balances` migration + rollback).
    // Dedup straight on (holder_id, asset_id): the project `asset_id` already folds
    // the Horizon alphanum4/12 split into one `classic-credit` key. amount is the
    // same scaled-i128 (decimals=7 at read); accounts resolve via `accounts`.
    let mut balance_dedup: HashMap<(i64, i64), BalanceRow> = HashMap::new();
    // Entry-state dedup by account, LAST occurrence wins — the same hazard
    // `build_balance_rows` guards against, for the same reason. States arrive one
    // per TRANSACTION (`process.rs` calls `extract_account_states` per tx and
    // concatenates), and every state in a ledger carries that ledger as its
    // watermark. Two `SetOptions` on one account in one ledger would therefore
    // emit two `account_entry_state` rows at the SAME RMT version, and the merge
    // resolves a tie arbitrarily — a removed signer could survive as the winner,
    // which is exactly what the whole-set-replacement design promises cannot
    // happen. Ledger/tx order puts the final state last, so last-wins is right.
    let mut entry_state_dedup: HashMap<i64, AccountEntryStateRow> = HashMap::new();
    for st in account_states {
        let watermark = i64::from(st.last_seen_ledger);
        let account_id_int = ids::account_id(&st.account_id);
        for b in st.balances.as_array().into_iter().flatten() {
            let Some(asset_type) = b
                .get("asset_type")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<AssetType>().ok())
            else {
                continue;
            };
            let balance_str = b.get("balance").and_then(Value::as_str).unwrap_or("0");
            let amount = decimal7_string_to_i128(balance_str)?;
            let asset_id = if asset_type == AssetType::Native {
                ids::NATIVE_ASSET_ID
            } else {
                let code = b.get("asset_code").and_then(Value::as_str).unwrap_or("");
                let issuer = b.get("issuer").and_then(Value::as_str).unwrap_or("");
                if code.is_empty() || issuer.is_empty() {
                    continue;
                }
                ids::credit_asset_id(code, issuer)
            };
            // The account entry itself was removed (`account_merge`) — the
            // native 0 below is a tombstone, not a balance. Every other row
            // here is live: a zero trustline balance is a real, open trustline.
            // ADR 0055.
            let closed_at = if st.account_removed && asset_type == AssetType::Native {
                watermark
            } else {
                0
            };
            upsert_balance(
                &mut balance_dedup,
                account_id_int,
                asset_id,
                amount,
                watermark,
                closed_at,
            );
        }

        // Signers/thresholds side row — ONLY when the AccountEntry itself was
        // observed (`thresholds.is_some()`); a trustline-only accum must not
        // touch the set. Deleted accounts (`account_removed`) emit nothing:
        // the page gates on `deleted`, and a merge cannot change signers.
        if let (Some(th_hex), false) = (&st.thresholds, st.account_removed) {
            match parse_thresholds(th_hex) {
                Some([master_weight, threshold_low, threshold_med, threshold_high]) => {
                    let signers = st.signers.as_deref().unwrap_or_default();
                    let mut signer_keys = Vec::with_capacity(signers.len());
                    let mut signer_weights = Vec::with_capacity(signers.len());
                    let mut signer_types = Vec::with_capacity(signers.len());
                    for sg in signers {
                        let key = sg.get("key").and_then(Value::as_str).unwrap_or("");
                        let weight = sg.get("weight").and_then(Value::as_u64).unwrap_or(0) as u32;
                        let typ = sg.get("type").and_then(Value::as_str).unwrap_or("unknown");
                        // A keyless signer would silently shrink the set — a
                        // 3-of-5 stored as 3-of-4, which reads as a real
                        // threshold rather than as missing data. Cannot happen
                        // from `account_data` (the key is always emitted), so
                        // reaching this means the producer changed shape.
                        if key.is_empty() {
                            tracing::warn!(
                                account = %st.account_id,
                                "signer entry carries no key — DROPPED from the set; \
                                 the stored signer count is now lower than the chain's"
                            );
                            continue;
                        }
                        // The protocol constrains non-master weights to 1-255
                        // (SetOptions deletes at 0). Store what the chain
                        // carried; out-of-range is an anomaly worth a trace,
                        // never a silent clamp.
                        if weight == 0 || weight > 255 {
                            tracing::warn!(
                                account = %st.account_id,
                                weight,
                                "signer weight outside protocol range 1-255 — stored as carried"
                            );
                        }
                        signer_keys.push(key.to_string());
                        signer_weights.push(weight);
                        signer_types.push(typ.to_string());
                    }
                    entry_state_dedup.insert(
                        account_id_int,
                        AccountEntryStateRow {
                            account_id: account_id_int,
                            signer_keys,
                            signer_weights,
                            signer_types,
                            master_weight,
                            threshold_low,
                            threshold_med,
                            threshold_high,
                            flags: st.flags.unwrap_or(0),
                            last_updated_ledger: watermark,
                        },
                    );
                }
                None => tracing::warn!(
                    account = %st.account_id,
                    thresholds = %th_hex,
                    "unparseable thresholds hex — signers row skipped, not fabricated"
                ),
            }
        }

        for rm in &st.removed_trustlines {
            let code = rm.get("asset_code").and_then(Value::as_str).unwrap_or("");
            let issuer = rm.get("issuer").and_then(Value::as_str).unwrap_or("");
            if code.is_empty() || issuer.is_empty() {
                continue;
            }
            let asset_id = ids::credit_asset_id(code, issuer);
            // The trustline was REMOVED — this is the closure the read path
            // could never see, because it looked identical to a live zero.
            upsert_balance(
                &mut balance_dedup,
                account_id_int,
                asset_id,
                0,
                watermark,
                watermark,
            );
        }
    }
    out.unified_balance_rows.extend(balance_dedup.into_values());
    out.account_entry_state_rows
        .extend(entry_state_dedup.into_values());

    // ---- soroban_contracts Pass 2 stub-rowing ----
    {
        let mut emitted: HashSet<&str> = HashSet::new();
        for cid in &contract_seen {
            emitted.insert(cid.as_str());
        }
        // Task 0323 — suppress the Pass-2 FK stub for SAC-override contracts.
        // `sac_overrides` are crypto-proven un-deployed SACs (modelled as
        // ASSETS, not contracts) plus deployed SACs that emit this ledger
        // (already carrying a real deploy row). Neither should get an
        // `is_sac=false` stub: un-deployed SACs get an `assets` row instead,
        // and for a deployed SAC a stub at `wasm_uploaded_at_ledger=0` could
        // clobber its real deploy row on the equal-version RMT merge.
        for ov in sac_overrides {
            emitted.insert(ov.contract_id.as_str());
        }

        let mut referenced: HashSet<String> = HashSet::new();
        for (_, ops) in operations {
            for op in ops {
                if let Some(c) = OpTyped::from_details(op.op_type, &op.details).contract_id {
                    referenced.insert(c);
                }
            }
        }
        for (_, evs) in events {
            for ev in evs {
                if is_diagnostic(ev.source) {
                    continue;
                }
                if let Some(c) = &ev.contract_id {
                    referenced.insert(c.clone());
                }
            }
        }
        for (_, invs) in invocations {
            for inv in invs {
                if let Some(c) = &inv.contract_id {
                    referenced.insert(c.clone());
                }
                if let Some(caller) = &inv.caller_account
                    && caller.starts_with('C')
                {
                    referenced.insert(caller.clone());
                }
            }
        }
        for a in assets {
            if let Some(c) = &a.contract_id {
                referenced.insert(c.clone());
            }
        }
        for n in nfts {
            referenced.insert(n.contract_id.clone());
        }
        for ev in nft_events {
            referenced.insert(ev.contract_id.clone());
        }

        for cid in &referenced {
            if emitted.contains(cid.as_str()) {
                continue;
            }
            out.contract_rows.push(SorobanContractRow {
                id: ids::contract_id(cid),
                contract_id: cid.clone(),
                wasm_hash: None,
                wasm_uploaded_at_ledger: 0,
                deployer_id: None,
                deployed_at_ledger: None,
                contract_type: None,
                is_sac: false,
            });
        }
    }

    // ---- Task 0320 live path: WASM-upgrade row rewrites ----
    // Appended last (after deploy / SAC-override / skeleton rows) so a contract
    // upgraded this ledger gets a full row overriding `wasm_hash` +
    // `wasm_uploaded_at_ledger` (RMT version = upgrade ledger, wins the merge)
    // while carrying its identity forward. `prior_contract_rows` is empty for
    // every caller except the live indexer, so this is a no-op elsewhere. A
    // contract that both deploys and upgrades in one ledger is impossible (the
    // WASM upload must precede the upgrade in an earlier tx).
    out.contract_rows.extend(build_wasm_upgrade_rows(
        events,
        prior_contract_rows,
        ledger_sequence_i64,
    ));

    Ok(out)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn decode_hash(hex_str: &str, field: &'static str) -> Result<[u8; 32], SchemaError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| staging_err(&format!("hex decode {field}: {e} (value={hex_str})")))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        staging_err(&format!(
            "hex length {field}: expected 32 bytes, got {} (value={hex_str})",
            bytes.len()
        ))
    })
}

fn staging_err(msg: &str) -> SchemaError {
    SchemaError::Staging(msg.to_string())
}

/// 4-byte `Thresholds` hex → [master_weight, low, med, high]. `None` on any
/// malformation — the caller skips the row rather than fabricating zeros.
fn parse_thresholds(hex_str: &str) -> Option<[u8; 4]> {
    let bytes = hex::decode(hex_str).ok()?;
    <[u8; 4]>::try_from(bytes.as_slice()).ok()
}

/// Upsert a `BalanceRow` into the per-`(holder_id, asset_id)` dedup map, keeping
/// the newest `last_updated_ledger` (RMT version semantics resolved at stage time).
fn upsert_balance(
    map: &mut HashMap<(i64, i64), BalanceRow>,
    holder_id: i64,
    asset_id: i64,
    amount: i128,
    last_updated_ledger: i64,
    closed_at_ledger: i64,
) {
    use std::collections::hash_map::Entry;
    let row = BalanceRow {
        holder_id,
        asset_id,
        amount,
        last_updated_ledger,
        closed_at_ledger,
    };
    match map.entry((holder_id, asset_id)) {
        Entry::Occupied(mut occ) => {
            if row.last_updated_ledger >= occ.get().last_updated_ledger {
                *occ.get_mut() = row;
            }
        }
        Entry::Vacant(vac) => {
            vac.insert(row);
        }
    }
}

fn decimal7_string_to_i128(s: &str) -> Result<i128, SchemaError> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1i128, r),
        None => (1i128, s.strip_prefix('+').unwrap_or(s)),
    };
    let unsigned: i128 = if let Some((whole, frac)) = rest.split_once('.') {
        let whole_n: i128 = if whole.is_empty() {
            0
        } else {
            whole
                .parse()
                .map_err(|e| staging_err(&format!("decimal parse '{s}': {e}")))?
        };
        let frac_trim = &frac[..frac.len().min(7)];
        let frac_n: i128 = if frac_trim.is_empty() {
            0
        } else {
            frac_trim
                .parse()
                .map_err(|e| staging_err(&format!("decimal frac parse '{s}': {e}")))?
        };
        let scale = 10i128.pow((7 - frac_trim.len()) as u32);
        whole_n
            .checked_mul(10_000_000)
            .and_then(|w| w.checked_add(frac_n * scale))
            .ok_or_else(|| staging_err(&format!("decimal overflow '{s}'")))?
    } else {
        rest.parse()
            .map_err(|e| staging_err(&format!("integer parse '{s}': {e}")))?
    };
    Ok(sign * unsigned)
}

fn is_strkey_account(s: &str) -> bool {
    s.len() <= 56 && s.starts_with('G')
}

fn is_diagnostic(src: EventSource) -> bool {
    matches!(src, EventSource::Diagnostic)
}

/// Collapse `pool_state_changes` to ONE row per (pool, plane, ledger) — the
/// cross-writer twin of `dedup_final_pool_snapshots` (lore-0356), via the
/// shared `keep_last_by_key` fold. This is the ONLY fold on this vector: the
/// plane arm and the concentrated-instance arm can collide on a pool's
/// registration ledger, and emitting both would leave the surviving row to a
/// version-less `ReplacingMergeTree` — the hazard backfills.md rule 4 names.
///
/// `plane_id` is IN the key (three-lens review, 2026-09-01): a forged plane
/// entry naming a real pool would otherwise EVICT the pool's genuine row at
/// this fold (and at the table's RMT key) — the read-side declared-plane
/// filter would then hide the forgery but serve a stale ledger's reserves as
/// current. With the plane in the key a forged row lands in its own key
/// space, dies at the read filter, and stays visible to the divergence
/// monitor (see the `pool_state_changes` DDL comment). Genuine collisions
/// still fold: both arms stamp the pool's own declared plane.
///
/// Last-wins in staging order: the instance arm is the more specific source
/// for a concentrated pool and runs second. Folding — not a version column —
/// is what makes the stored row a deterministic function of the ledger, so a
/// re-parse still wins simply by landing last (rule 4). The `ledger_sequence`
/// component is belt-and-braces for a future batching caller
/// (`xdr_parser::fold`).
fn fold_pool_state_changes(rows: Vec<PoolStateChangeRow>) -> Vec<PoolStateChangeRow> {
    xdr_parser::fold::keep_last_by_key(rows, |r| (r.pool_id, r.plane_id, r.ledger_sequence))
}

/// Collapse instance-state rows to ONE per (pool, ledger) — the last image
/// in apply order. Load-bearing, not theoretical: 29 of 259 real
/// (pool, ledger) keys in the raw corpus carry more than one instance image
/// (up to 5 in one ledger — every pool action rewrites the instance), and
/// the table's RMT version (`derived_at_ledger`) TIES within a ledger, so an
/// unfolded insert would leave the surviving image to an arbitrary merge —
/// the 0463 defect class. Every image carries the FULL instance storage, so
/// keeping only the last loses nothing.
fn fold_pool_instance_state(rows: Vec<PoolInstanceStateRow>) -> Vec<PoolInstanceStateRow> {
    xdr_parser::fold::keep_last_by_key(rows, |r| (r.pool_id, r.derived_at_ledger))
}

/// Raw decimal reserve strings → `i128`, all-or-nothing: one unparseable
/// element refuses the whole vector, because a partial reserve set is a
/// snapshot lying about its own arity. Shared by the plane and the
/// concentrated-instance arm.
fn parse_reserves(raw: &[String]) -> Option<Vec<i128>> {
    raw.iter().map(|r| r.parse::<i128>().ok()).collect()
}

/// The two-leg tuple flavour of [`parse_reserves`], shared by the
/// pair-factory and config-factory arms — same all-or-nothing rule.
fn parse_reserve_pair(a: &str, b: &str) -> Option<Vec<i128>> {
    Some(vec![a.parse::<i128>().ok()?, b.parse::<i128>().ok()?])
}

/// Raw LP-supply string → `i128`. Absent means the write did not touch the
/// key (a structural 0, never a fallback); PRESENT-but-unparseable is
/// `Err` so the caller refuses the row loudly. Shared by the pair-factory
/// (TotalSupply) and config-factory (TotalShares) arms.
fn parse_supply(raw: Option<&str>) -> Result<i128, ()> {
    match raw {
        None => Ok(0),
        Some(raw) => raw.parse::<i128>().map_err(|_| ()),
    }
}

/// Registry row for one corroborated `new_pair` registration (task 0518).
///
/// `pool_type_raw` stays EMPTY: the vendor emits no type — Soroswap is one
/// fixed constant-product mode — and an invented label would be our
/// interpretation, not a verbatim value (decision 64). The fee is the
/// vendor's compiled-in constant: 3/1000 on every swap ("Constant product
/// AMM with a .3% swap fee", `soroswap/core` pair source, fetched
/// 2026-09-02) = 30 bps. Legs are the pair's leg TOKENS in vendor order
/// (token_0, token_1); the share token is NOT a registry column — the pair
/// is its own LP token and the relation lives in `pool_instance_state`.
fn factory_pair_registry_row(
    reg: &xdr_parser::pool_pair_factory::PairRegistration,
    ledger_sequence: i64,
) -> Result<LiquidityPoolRow, &'static str> {
    let pool_id =
        ids::contract_payload(&reg.event.pair).ok_or("pair address is not a valid C… strkey")?;
    Ok(LiquidityPoolRow {
        pool_id,
        asset_a_type: 0,
        asset_a_code: String::new(),
        asset_a_issuer_id: 0,
        asset_b_type: 0,
        asset_b_code: String::new(),
        asset_b_issuer_id: 0,
        fee_bps: 30,
        last_updated_ledger: ledger_sequence,
        pool_kind: 1,
        legs: vec![
            ids::contract_id(&reg.event.token_0),
            ids::contract_id(&reg.event.token_1),
        ],
        deployment_id: ids::contract_id(&reg.factory),
        pool_type_raw: String::new(),
    })
}

/// Registry row for one corroborated config-factory registration. The event
/// names only the pool; every registry fact comes from the pool's own
/// `CONFIG` (the same map that corroborated the registration). The share
/// token is NOT a registry column — the relation lives in
/// `pool_instance_state`, same as both sibling families.
///
/// `pool_type_raw` stores the vendor's `PairType` discriminant verbatim
/// ("0" = XYK today; a stable pool would carry its own value) — the same
/// un-normalised-on-purpose rule as the router family's sym.
fn config_pool_registry_row(
    reg: &xdr_parser::pool_config_factory::ConfigPoolRegistration,
    config: &xdr_parser::pool_config_factory::PoolConfig,
    ledger_sequence: i64,
) -> Result<LiquidityPoolRow, &'static str> {
    let pool_id =
        ids::contract_payload(&reg.pool).ok_or("pool address is not a valid C… strkey")?;
    // The chain carries the fee as i64; an out-of-i32-range value is a new
    // vocabulary nobody has seen — refuse it loudly rather than record a
    // plausible truncation.
    let fee_bps =
        i32::try_from(config.total_fee_bps).map_err(|_| "total_fee_bps out of i32 range")?;
    Ok(LiquidityPoolRow {
        pool_id,
        asset_a_type: 0,
        asset_a_code: String::new(),
        asset_a_issuer_id: 0,
        asset_b_type: 0,
        asset_b_code: String::new(),
        asset_b_issuer_id: 0,
        fee_bps,
        last_updated_ledger: ledger_sequence,
        pool_kind: 1,
        legs: vec![
            ids::contract_id(&config.token_a),
            ids::contract_id(&config.token_b),
        ],
        deployment_id: ids::contract_id(&reg.factory),
        pool_type_raw: config.pool_type.to_string(),
    })
}

/// Registry row for one decoded `add_pool` registration.
///
/// `Err` names what was wrong. A bad pool address must never fabricate a
/// 32-byte `pool_id`, and an unparseable fee must never become a plausible
/// zero (Karol, 2026-08-28: error, not warn-and-default) — either way the
/// registration is refused loudly and lands in the missing-pool alarm.
///
/// The share-token relation lives ONLY in `pool_instance_state` (side table;
/// a registry column for it was dead-on-arrival and removed). No venue label
/// is stored anywhere: labels resolve from `deployment_id` at read time. The salt and raw
/// init_args are NOT materialised — the add_pool event itself sits complete
/// in soroban_events; extract on demand, never copy.
fn pool_registry_row(
    reg: &xdr_parser::pool_router::AddPoolEvent,
    router_strkey: &str,
    ledger_sequence: i64,
) -> Result<LiquidityPoolRow, &'static str> {
    let pool_id =
        ids::contract_payload(&reg.pool).ok_or("pool address is not a valid C… strkey")?;
    // Position 0 is a u32 fee in EVERY shape measured on mainnet (497/497,
    // pinned by the corpus test). A shape where it is missing or unparseable
    // is a new vocabulary nobody has seen — refuse it loudly rather than
    // record a plausible fee of 0.
    let fee_bps = reg
        .init_args
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .ok_or("init_args[0] is not a parseable fee")?;
    Ok(LiquidityPoolRow {
        pool_id,
        asset_a_type: 0,
        asset_a_code: String::new(),
        asset_a_issuer_id: 0,
        asset_b_type: 0,
        asset_b_code: String::new(),
        asset_b_issuer_id: 0,
        fee_bps,
        last_updated_ledger: ledger_sequence,
        pool_kind: 1,
        legs: reg.tokens.iter().map(|t| ids::contract_id(t)).collect(),
        deployment_id: ids::contract_id(router_strkey),
        pool_type_raw: reg.pool_type.clone(),
    })
}

/// Event NAME, lifted from the topics into the `signature` column (the cheap
/// `WHERE signature = 'transfer'` filter).
///
/// Three publishing conventions exist on mainnet (task 0517; measured
/// 2026-09-02 on two 1M-ledger windows of the then-NULL population, shapes
/// identical in both):
///
/// 1. `[Symbol(name), …]` — the dominant convention (SEP-41, the router
///    family, …). Unchanged.
/// 2. `[String(label), Symbol(name), …]` — a protocol label first, the name
///    second (SoroswapPair/Router/Aggregator, DeFindexVault, BlendStrategy).
///    The label is NOT copied anywhere: it sits verbatim in `topics_xdr`
///    forever — extract on demand, never copy.
/// 3. `[String(name), …]` where `topics[1]` is NOT a Symbol — the
///    Phoenix-family plain-&str convention (`("swap","sender")` publishes
///    two Strings): the FIRST topic is the name, the second discriminates
///    the field and stays in the topics for the protocol's decoder.
///
/// Known compromise in arm 3: a future protocol publishing
/// `[String(label), String(name)]` would get its label lifted as the name —
/// wrong but visible and verifiable per protocol, unlike the silent NULL it
/// replaces.
///
/// Anything else with a non-empty topic vector resolves nowhere: it keeps
/// NULL **and warns**, so the next convention surfaces as a count, never as
/// absence (the 0517 monitor; 100% of the measured NULL population had a
/// String first topic, so this arm is quiet today). An EMPTY topic vector
/// stays a silent NULL — there is no name to resolve.
fn extract_event_signature(topics: &Value) -> Option<String> {
    let arr = topics.as_array()?;
    let first = arr.first()?;
    let nonempty = |s: &str| (!s.is_empty()).then(|| s.to_string());
    if let Some(name) = scval::typed_str(first, "sym").and_then(nonempty) {
        return Some(name);
    }
    if let Some(label_or_name) = scval::typed_str(first, "string").and_then(nonempty) {
        if let Some(name) = arr
            .get(1)
            .and_then(|t| scval::typed_str(t, "sym"))
            .and_then(nonempty)
        {
            return Some(name);
        }
        return Some(label_or_name);
    }
    tracing::warn!(
        topics = %topics,
        "event name unresolved — unknown topic convention (task 0517 monitor)"
    );
    None
}

fn tx_has_soroban_map(operations: &[(String, Vec<ExtractedOperation>)]) -> HashMap<String, bool> {
    operations
        .iter()
        .map(|(tx_hash, ops)| {
            let has = ops.iter().any(|op| {
                matches!(
                    op.op_type,
                    OperationType::InvokeHostFunction
                        | OperationType::ExtendFootprintTtl
                        | OperationType::RestoreFootprint
                )
            });
            (tx_hash.clone(), has)
        })
        .collect()
}

/// Canonical per-type projection of an operation's identity fields from its
/// `details` JSON. Public because `audit-harness` (operations-order-diff)
/// projects the SAME identity when diffing DB order against archive XDR —
/// it previously kept a hand-maintained copy, which drifted (task 0455,
/// finding 9); sharing the one implementation makes drift inexpressible.
pub struct OpTyped {
    pub destination: Option<String>,
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    /// Liquidity pools touched by the op. Single-element for LP
    /// deposit/withdraw; the full crossed-pool list (from result claim
    /// atoms) for path payments; empty otherwise. Task 0261 / 0268.
    pub pool_ids_hex: Vec<String>,
}

impl OpTyped {
    pub fn from_details(op_type: OperationType, details: &Value) -> Self {
        let mut out = Self {
            destination: None,
            contract_id: None,
            asset_code: None,
            asset_issuer: None,
            pool_ids_hex: Vec::new(),
        };
        match op_type {
            OperationType::CreateAccount => {
                out.destination = str_field(details, "destination");
            }
            OperationType::Payment => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("asset") {
                    let (c, i) = split_asset_ref(asset);
                    out.asset_code = c;
                    out.asset_issuer = i;
                }
            }
            OperationType::PathPaymentStrictReceive | OperationType::PathPaymentStrictSend => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("destAsset") {
                    let (c, i) = split_asset_ref(asset);
                    out.asset_code = c;
                    out.asset_issuer = i;
                }
            }
            OperationType::AccountMerge => {
                out.destination = str_field(details, "destination");
            }
            OperationType::Clawback => {
                out.destination = str_field(details, "from");
                if let Some(asset) = details.get("asset") {
                    let (c, i) = split_asset_ref(asset);
                    out.asset_code = c;
                    out.asset_issuer = i;
                }
            }
            OperationType::LiquidityPoolDeposit | OperationType::LiquidityPoolWithdraw => {
                out.pool_ids_hex = str_field(details, "liquidityPoolId").into_iter().collect();
            }
            OperationType::InvokeHostFunction => {
                out.contract_id = str_field(details, "contractId");
            }
            OperationType::ChangeTrust => {
                if let Some(asset) = details.get("asset") {
                    let (c, i) = split_asset_ref(asset);
                    out.asset_code = c;
                    out.asset_issuer = i;
                }
            }
            OperationType::SetTrustLineFlags => {
                out.destination = str_field(details, "trustor");
                if let Some(asset) = details.get("asset") {
                    let (c, i) = split_asset_ref(asset);
                    out.asset_code = c;
                    out.asset_issuer = i;
                }
            }
            OperationType::AllowTrust => {
                out.destination = str_field(details, "trustor");
                if let Some(asset) = details.get("asset")
                    && let Some(code) = asset.as_str()
                {
                    out.asset_code = Some(code.to_string());
                }
            }
            OperationType::BeginSponsoringFutureReserves => {
                out.destination = str_field(details, "sponsoredId");
            }
            // Deliberately no identity fields beyond `source`. Exhaustive on
            // purpose (no `_` arm): a protocol bump that adds an operation
            // type must fail compilation here and force a decision, instead
            // of silently projecting an empty identity (task 0455; same
            // total-function posture as 0434). Offers can still touch pools —
            // that identity arrives via the `poolIds` fallback below.
            //
            // Audited per-arm (2026-08-06): none of these drop data, because
            // this struct was never the only channel. Offer selling/buying
            // pairs and claimable-balance assets go through
            // `xdr_parser::asset_appearances` → `operation_asset_appearances`
            // (a single asset_code column cannot hold a two-asset op);
            // CB claimants, inflationDest and revoke-sponsorship targets go
            // through `xdr_parser::op_participants` (the single op-side
            // participant source). `balanceId`/`offerId` are entity ids,
            // consistently not identity dimensions.
            OperationType::ManageSellOffer
            | OperationType::CreatePassiveSellOffer
            | OperationType::SetOptions
            | OperationType::Inflation
            | OperationType::ManageData
            | OperationType::BumpSequence
            | OperationType::ManageBuyOffer
            | OperationType::CreateClaimableBalance
            | OperationType::ClaimClaimableBalance
            | OperationType::EndSponsoringFutureReserves
            | OperationType::RevokeSponsorship
            | OperationType::ClawbackClaimableBalance
            | OperationType::ExtendFootprintTtl
            | OperationType::RestoreFootprint => {}
        }
        // `poolIds` (path payments + offers crossing an LP — task 0261) is
        // present on any op whose result carried claim atoms, regardless of op
        // type. LP deposit/withdraw already set `pool_ids_hex` from
        // `liquidityPoolId` above, so the guard skips them.
        if out.pool_ids_hex.is_empty()
            && let Some(ids) = details.get("poolIds").and_then(Value::as_array)
        {
            out.pool_ids_hex = ids
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        out
    }
}

fn str_field(obj: &Value, field: &str) -> Option<String> {
    obj.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn split_asset_ref(asset: &Value) -> (Option<String>, Option<String>) {
    let Some(s) = asset.as_str() else {
        return (None, None);
    };
    if s == "native" {
        return (None, None);
    }
    match s.split_once(':') {
        Some((code, issuer)) if !code.is_empty() && !issuer.is_empty() => {
            (Some(code.to_string()), Some(issuer.to_string()))
        }
        _ => (None, None),
    }
}

fn asset_issuer(asset: &Value) -> Option<String> {
    asset
        .as_object()?
        .get("issuer")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn split_pool_asset(asset: &Value) -> Option<(AssetType, Option<String>, Option<String>)> {
    if let Some(s) = asset.as_str()
        && s == "native"
    {
        return Some((AssetType::Native, None, None));
    }
    let obj = asset.as_object()?;
    let ty = obj.get("type").and_then(Value::as_str)?.parse().ok()?;
    let code = obj
        .get("code")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let issuer = obj
        .get("issuer")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some((ty, code, issuer))
}

#[derive(Debug, Default)]
struct AccountOverride {
    sequence_number: i64,
    sequence_set: bool,
    first_seen_ledger: Option<i64>,
    home_domain: Option<String>,
    last_seen_ledger: Option<i64>,
}

impl AccountOverride {
    fn from_first(state: &ExtractedAccountState) -> Self {
        Self {
            sequence_number: state.sequence_number,
            sequence_set: state.sequence_number >= 0,
            first_seen_ledger: state.first_seen_ledger.map(i64::from),
            home_domain: state.home_domain.clone(),
            last_seen_ledger: Some(i64::from(state.last_seen_ledger)),
        }
    }
}

#[derive(Debug)]
struct AccountOverridePublic {
    last_seen_ledger: Option<i64>,
    first_seen_ledger: Option<i64>,
    sequence_number: i64,
    home_domain: Option<String>,
}

fn merge_account_state_overrides(
    states: &[ExtractedAccountState],
) -> HashMap<String, AccountOverridePublic> {
    use std::collections::hash_map::Entry;
    let mut overrides: HashMap<String, AccountOverride> = HashMap::new();
    for st in states {
        match overrides.entry(st.account_id.clone()) {
            Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                if st.sequence_number >= 0 {
                    existing.sequence_number = st.sequence_number;
                    existing.sequence_set = true;
                }
                let incoming = st.first_seen_ledger.map(i64::from);
                existing.first_seen_ledger = match (existing.first_seen_ledger, incoming) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, b) => b,
                };
                if let Some(hd) = st.home_domain.clone() {
                    existing.home_domain = Some(hd);
                }
                let last = i64::from(st.last_seen_ledger);
                existing.last_seen_ledger = Some(match existing.last_seen_ledger {
                    Some(prev) => prev.max(last),
                    None => last,
                });
            }
            Entry::Vacant(vac) => {
                vac.insert(AccountOverride::from_first(st));
            }
        }
    }
    overrides
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                AccountOverridePublic {
                    last_seen_ledger: v.last_seen_ledger,
                    first_seen_ledger: v.first_seen_ledger,
                    sequence_number: if v.sequence_set { v.sequence_number } else { 0 },
                    home_domain: v.home_domain,
                },
            )
        })
        .collect()
}

/// The 0383 presence rows derived from one Soroban token event: the G-account
/// participants and (for SAC-wrapped classic/native only) the touched asset
/// surrogate. Shared by ingest (below) and the `soroban_token_flow` backfill so
/// both emit byte-identical rows — the surrogate hashing is `cityhash_102_128`
/// (see [`ids`]) and cannot be reproduced in CH SQL, so a single Rust source of
/// truth is the only way to guarantee the backfill dedups against live rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedTokenEvent {
    /// `from`/`to` operands that are G-accounts (contract `C…` addresses dropped).
    pub participant_strkeys: Vec<String>,
    /// Asset surrogate resolved by [`event_asset_surrogate`] (task 0393): Native →
    /// `NATIVE_ASSET_ID`, Credit → `credit_asset_id(code, issuer)` (both match the
    /// op-derived `operation_asset_appearances` keys exactly, so event rows dedup
    /// against op rows), and bespoke `EventAsset::Bespoke` → the **emitting
    /// contract's own surrogate** (a type-3 token's asset_id == its contract id).
    /// `None` only when a bespoke event carries no emitting contract id. (Before
    /// task 0393 bespoke returned `None` and relied on arm B / invocation
    /// appearances; 0393 needs the amount, which arm B has no concept of, so
    /// bespoke now writes arm A here.)
    pub asset_id: Option<i64>,
}

/// The `operation_asset_appearances` surrogate for a decoded token event's asset
/// — the one place the event paths (presence + value-moved) resolve it, so their
/// keys always match. Native → `NATIVE_ASSET_ID`, Credit → `ids::credit_asset_id`,
/// bespoke (`Bespoke`, no SEP-11 asset string) → the **emitting contract's own
/// surrogate** (a type-3 token's asset_id == its contract id, verified 4172/4172;
/// task 0393). `None` only when a bespoke event carries no emitting contract id.
///
/// Bespoke NFTs also resolve here, but they have **no `assets` row** (that table
/// only gets a row for `contract_type == Fungible`, stage.rs), so the value read's
/// `INNER JOIN assets` drops them — only genuine fungible bespoke tokens surface a
/// value. (Before task 0393 this returned `None` for bespoke, since arm B —
/// `soroban_invocations_appearances` — already covered their asset page; 0393
/// needs the amount, which arm B has no concept of, so bespoke now writes arm A.)
fn event_asset_surrogate(asset: &EventAsset, emitting_contract_id: Option<i64>) -> Option<i64> {
    match asset {
        EventAsset::Native => Some(ids::NATIVE_ASSET_ID),
        EventAsset::Credit { code, issuer } => Some(ids::credit_asset_id(code, issuer)),
        // A bespoke token's asset_id IS its emitting contract's surrogate; the
        // caller passes it already-resolved (live hashes the C-StrKey; the
        // backfill reads the surrogate straight from `soroban_events.contract_id`).
        EventAsset::Bespoke => emitting_contract_id,
    }
}

/// Decode one Soroban contract event into its 0383 presence rows. `None` when
/// the event is not a SEP-41 / CAP-67 token event. `emitting_contract_id` is the
/// event's own contract **surrogate** — the asset identity for a bespoke token
/// (task 0393). Callers pass it already-resolved: live via `ids::contract_id(
/// strkey)`, the backfill straight from `soroban_events.contract_id`.
pub fn derive_token_event(
    topics: &Value,
    emitting_contract_id: Option<i64>,
) -> Option<DerivedTokenEvent> {
    let token = xdr_parser::parse_token_event(topics)?;
    let participant_strkeys = [token.from, token.to]
        .into_iter()
        .flatten()
        .filter(|k| is_strkey_account(k))
        .collect();
    Some(DerivedTokenEvent {
        participant_strkeys,
        asset_id: event_asset_surrogate(&token.asset, emitting_contract_id),
    })
}

/// Net-settled value per asset for ONE transaction (classic OR Soroban), from its
/// per-(holder, asset) ledger balance deltas. Each delta is resolved to its
/// `asset_id` and handed to `net_settled`, which reduces to `max(Σ+, Σ−)` per asset.
///
/// The deltas come from `xdr_parser::ledger_balance_deltas` over the tx's
/// `TransactionMeta` and cover EVERY value flow uniformly from the ledger —
/// native, classic credit, SAC contract-held balances, and bespoke token balances
/// — because all settle as `AccountEntry` / `TrustLineEntry` / `ContractData`
/// balance changes. This is the authoritative, unspoofable source; token events
/// (logs) are never used for value. The fee is charged in a separate ledger phase
/// (not `TransactionMeta`), so it is absent by construction (formula rule 3).
///
/// `sac_classic` re-maps a contract-held SAC balance (keyed by the SAC contract
/// surrogate) onto the wrapped classic/native `asset_id` — the same surrogate the
/// account-side trustline leg resolves to, so a mixed account↔contract transfer
/// nets as ONE asset, not two.
pub fn ledger_deltas_net_settled(
    deltas: &[LedgerDelta],
    sac_classic: &HashMap<i64, i64>,
) -> Vec<NetSettled> {
    let resolved: Vec<AccountDelta> = deltas
        .iter()
        .filter_map(|d| {
            let asset_id = match &d.asset {
                LedgerAsset::Native => ids::NATIVE_ASSET_ID,
                LedgerAsset::Credit { code, issuer } => ids::credit_asset_id(code, issuer),
                // SAC-wrapped classic → the wrapped classic asset via the forward-
                // derived registry; an unknown SAC (not yet mapped) drops the delta
                // (`?`) — no value rather than the wrong asset.
                LedgerAsset::SacWrapped(sac) => *sac_classic.get(&ids::contract_id(sac))?,
                // Bespoke token → its own contract surrogate (the token IS the asset).
                LedgerAsset::Bespoke(contract) => ids::contract_id(contract),
            };
            // The signed delta passes straight through. `net_settled` buckets by
            // sign and is overflow-checked, so an attacker-authored i128 (a bespoke
            // token's contract-written balance) surfaces as "not computed", never a
            // wrapped figure. No `abs` here: the magnitude is not pre-taken, so
            // i128::MIN stays representable instead of dropping the whole delta.
            Some(AccountDelta {
                asset_id,
                account: d.account.clone(),
                delta: d.delta,
            })
        })
        .collect();
    xdr_parser::net_settled(&resolved)
}

#[cfg(test)]
mod pool_fill_amount_tests {
    use super::*;
    use serde_json::json;

    const POOL_HEX: &str = "41270552ed479ebf3e86c420f1d9401d4cde64720b4c201bf9a5352e36bf05cf";
    const TF_ISSUER: &str = "GB5WIXCUO5DWAJSVLVIJH5SBWGIRKGD27YYHLPOISGBO7MW2UH3EJXLM";

    fn atom(sold: &str, amount_sold: i64, bought: &str, amount_bought: i64) -> Value {
        json!({
            "poolId": POOL_HEX,
            "assetSold": sold,
            "amountSold": amount_sold,
            "assetBought": bought,
            "amountBought": amount_bought,
        })
    }

    /// The pool SOLD native and BOUGHT the credit asset, twice in one op —
    /// exactly the CAP-38 interleaved take that makes per-atom rows unsafe.
    /// Both legs must fold into one row each, native negative (left the pool)
    /// and credit positive (entered it).
    #[test]
    fn sums_repeated_takes_of_one_pool_and_signs_from_the_pools_side() {
        let tf = format!("TF:{TF_ISSUER}");
        let details = json!({
            "claimedAtoms": [
                atom("native", 1_396_629, &tf, 5_596_939),
                atom("native", 2_548_470, &tf, 10_208_417),
            ],
        });

        let native_id = ids::NATIVE_ASSET_ID;
        let tf_id = ids::credit_asset_id("TF", TF_ISSUER);
        let pool_id = <[u8; 32]>::try_from(hex::decode(POOL_HEX).unwrap().as_slice()).unwrap();

        let mut got = pool_fill_amounts(&details);
        got.sort_unstable_by_key(|(_, asset_id, _)| *asset_id);
        let mut want = vec![
            (pool_id, native_id, -(1_396_629 + 2_548_470)),
            (pool_id, tf_id, 5_596_939 + 10_208_417),
        ];
        want.sort_unstable_by_key(|(_, asset_id, _)| *asset_id);
        assert_eq!(got, want);
    }

    /// Deposits and withdrawals arrive as one already-signed `poolDelta`
    /// instead of atoms, and must land in the same rows.
    #[test]
    fn reads_deposit_and_withdrawal_deltas() {
        let tf = format!("TF:{TF_ISSUER}");
        let details = json!({
            "poolDelta": {
                "poolId": POOL_HEX,
                "assetA": "native",
                "amountA": -1_000,
                "assetB": tf,
                "amountB": -2_000,
            },
        });

        let pool_id = <[u8; 32]>::try_from(hex::decode(POOL_HEX).unwrap().as_slice()).unwrap();
        let got = pool_fill_amounts(&details);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&(pool_id, ids::NATIVE_ASSET_ID, -1_000)));
        assert!(got.contains(&(pool_id, ids::credit_asset_id("TF", TF_ISSUER), -2_000)));
    }

    #[test]
    fn no_atoms_no_rows() {
        assert!(pool_fill_amounts(&json!({})).is_empty());
        assert!(pool_fill_amounts(&json!({ "claimedAtoms": [] })).is_empty());
    }
}

#[cfg(test)]
mod ledger_deltas_net_settled_tests {
    use super::*;

    /// Reduce with an empty SAC registry — these tests use native/credit deltas
    /// only, which don't need it. The SAC-registry path is covered separately.
    fn reduce(deltas: &[LedgerDelta]) -> Vec<NetSettled> {
        ledger_deltas_net_settled(deltas, &HashMap::new())
    }

    const ISSUER: &str = "GB5WIXCUO5DWAJSVLVIJH5SBWGIRKGD27YYHLPOISGBO7MW2UH3EJXLM";
    const G_A: &str = "GBLVLKGRDU66WLWY4XRORJXCC4LDZ347AQTUYBEPBABIZTVITW2OAGIP";
    const G_B: &str = "GADKLS7RS3OC2MXGEZXQA46JNF3FBVSTHTWLDPRF7TWI6GXVP4OUE3ZR";

    fn native(account: &str, delta: i128) -> LedgerDelta {
        LedgerDelta {
            account: account.to_string(),
            asset: LedgerAsset::Native,
            delta,
        }
    }
    fn credit(account: &str, delta: i128) -> LedgerDelta {
        LedgerDelta {
            account: account.to_string(),
            asset: LedgerAsset::Credit {
                code: "USDC".to_string(),
                issuer: ISSUER.to_string(),
            },
            delta,
        }
    }

    #[test]
    fn native_payment_nets_to_amount() {
        // A -100, B +100: net native 100.
        let r = reduce(&[native(G_A, -100), native(G_B, 100)]);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].asset_id, r[0].amount),
            (ids::NATIVE_ASSET_ID, Some(100))
        );
    }

    #[test]
    fn credit_delta_resolves_to_its_surrogate() {
        let want = ids::credit_asset_id("USDC", ISSUER);
        let r = reduce(&[credit(G_A, -150)]);
        assert_eq!((r[0].asset_id, r[0].amount), (want, Some(150)));
    }

    #[test]
    fn one_sided_burn_delta_is_counted() {
        // A single negative delta (e.g. clawback / payment-to-issuer): max(Σ+,Σ−)
        // keeps it non-zero.
        let r = reduce(&[native(G_A, -250)]);
        assert_eq!(r[0].amount, Some(250));
    }

    #[test]
    fn swap_splits_into_two_asset_rows() {
        // A sends 300 native, receives 250 USDC; B is the counterparty.
        let usdc = ids::credit_asset_id("USDC", ISSUER);
        let r = reduce(&[
            native(G_A, -300),
            native(G_B, 300),
            credit(G_A, 250),
            credit(G_B, -250),
        ]);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.iter()
                .find(|n| n.asset_id == ids::NATIVE_ASSET_ID)
                .unwrap()
                .amount,
            Some(300)
        );
        assert_eq!(
            r.iter().find(|n| n.asset_id == usdc).unwrap().amount,
            Some(250)
        );
    }

    #[test]
    fn contract_and_account_legs_of_a_sac_transfer_net_as_one_asset() {
        // A contract sends 100 USDC to a G-account. The contract leg is a
        // ContractData SAC balance (SacWrapped), the account leg is a trustline
        // (Credit). Both MUST resolve to the SAME asset_id via the registry, or the
        // single transfer double-counts as two assets. (Verified in code that
        // sac_classic maps to exactly `credit_asset_id`; this pins it.)
        let usdc = ids::credit_asset_id("USDC", ISSUER);
        let sac_strkey = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
        let sac_classic: HashMap<i64, i64> =
            [(ids::contract_id(sac_strkey), usdc)].into_iter().collect();

        let contract_leg = LedgerDelta {
            account: "CONTRACT_HOLDER".to_string(),
            asset: LedgerAsset::SacWrapped(sac_strkey.to_string()),
            delta: -100,
        };
        let r = ledger_deltas_net_settled(&[contract_leg, credit(G_B, 100)], &sac_classic);
        assert_eq!(r.len(), 1, "must net as ONE asset, not double-count: {r:?}");
        assert_eq!((r[0].asset_id, r[0].amount), (usdc, Some(100)));
    }

    #[test]
    fn bespoke_contract_token_resolves_to_its_own_surrogate() {
        let token = "CBQFOPGGSP4VCDFXJ4YEPCQNLN6EFRC4M7OOLQOEEY7H4VPF6N4WEE2N";
        let d = LedgerDelta {
            account: "HOLDER".to_string(),
            asset: LedgerAsset::Bespoke(token.to_string()),
            delta: -50,
        };
        let r = ledger_deltas_net_settled(&[d], &HashMap::new());
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].asset_id, r[0].amount),
            (ids::contract_id(token), Some(50))
        );
    }
}

#[cfg(test)]
mod derive_token_event_tests {
    use super::*;
    use serde_json::json;

    const ISSUER: &str = "GB5WIXCUO5DWAJSVLVIJH5SBWGIRKGD27YYHLPOISGBO7MW2UH3EJXLM";
    const G_FROM: &str = "GBLVLKGRDU66WLWY4XRORJXCC4LDZ347AQTUYBEPBABIZTVITW2OAGIP";
    const G_TO: &str = "GADKLS7RS3OC2MXGEZXQA46JNF3FBVSTHTWLDPRF7TWI6GXVP4OUE3ZR";

    fn addr(v: &str) -> Value {
        json!({ "type": "address", "value": v })
    }
    fn sym(v: &str) -> Value {
        json!({ "type": "sym", "value": v })
    }
    fn strv(v: &str) -> Value {
        json!({ "type": "string", "value": v })
    }

    #[test]
    fn sac_transfer_yields_participants_and_classic_asset() {
        let d = derive_token_event(
            &json!([
                sym("transfer"),
                addr(G_FROM),
                addr(G_TO),
                strv(&format!("USDC:{ISSUER}"))
            ]),
            None,
        )
        .unwrap();
        assert_eq!(
            d.participant_strkeys,
            vec![G_FROM.to_string(), G_TO.to_string()]
        );
        assert_eq!(
            d.asset_id,
            Some(ids::asset_id(1, "USDC", ids::account_id(ISSUER), 0))
        );
    }

    #[test]
    fn native_transfer_yields_native_asset() {
        let d = derive_token_event(
            &json!([sym("transfer"), addr(G_FROM), addr(G_TO), strv("native")]),
            None,
        )
        .unwrap();
        assert_eq!(d.asset_id, Some(ids::NATIVE_ASSET_ID));
    }

    #[test]
    fn mint_yields_only_to_participant() {
        let d = derive_token_event(
            &json!([sym("mint"), addr(G_TO), strv(&format!("USDC:{ISSUER}"))]),
            None,
        )
        .unwrap();
        assert_eq!(d.participant_strkeys, vec![G_TO.to_string()]);
        assert!(d.asset_id.is_some());
    }

    #[test]
    fn bespoke_contract_event_resolves_asset_to_emitting_contract() {
        // Bespoke transfer (no SEP-11 asset string): asset_id = the EMITTING
        // contract's surrogate (task 0393). Accounts still tx participants.
        const CTOKEN: &str = "CBQFOPGGSP4VCDFXJ4YEPCQNLN6EFRC4M7OOLQOEEY7H4VPF6N4WEE2N";
        let d = derive_token_event(
            &json!([sym("transfer"), addr(G_FROM), addr(G_TO)]),
            Some(ids::contract_id(CTOKEN)),
        )
        .unwrap();
        assert_eq!(
            d.participant_strkeys,
            vec![G_FROM.to_string(), G_TO.to_string()]
        );
        assert_eq!(d.asset_id, Some(ids::contract_id(CTOKEN)));
    }

    #[test]
    fn bespoke_event_without_emitting_contract_has_no_asset() {
        let d =
            derive_token_event(&json!([sym("transfer"), addr(G_FROM), addr(G_TO)]), None).unwrap();
        assert_eq!(d.asset_id, None);
    }

    #[test]
    fn contract_address_participant_is_dropped() {
        // from = a C-contract address → not a G-account, dropped from participants.
        let d = derive_token_event(
            &json!([
                sym("transfer"),
                addr("CCONTRACTADDR"),
                addr(G_TO),
                strv("native")
            ]),
            None,
        )
        .unwrap();
        assert_eq!(d.participant_strkeys, vec![G_TO.to_string()]);
    }

    #[test]
    fn non_token_event_is_none() {
        assert!(derive_token_event(&json!([sym("swap"), addr(G_FROM)]), None).is_none());
    }
}

#[cfg(test)]
mod balance_tests {
    use super::*;
    use xdr_parser::ExtractedSorobanBalance;

    #[test]
    fn build_balance_rows_maps_holder_and_asset_surrogates() {
        let extracted = vec![ExtractedSorobanBalance {
            contract_id: "CTOKEN1".into(),
            holder: "GHOLDER1".into(),
            balance: 800_009_446_178_i128,
            ledger: 100,
            closed: false,
        }];
        // No SAC map → type-3 keying: asset_id == the token's contract surrogate.
        let rows = build_balance_rows(&extracted, &HashMap::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].holder_id, ids::address_id("GHOLDER1"));
        assert_eq!(rows[0].asset_id, ids::contract_id("CTOKEN1"));
        assert_eq!(rows[0].amount, 800_009_446_178_i128);
        assert_eq!(rows[0].last_updated_ledger, 100);
    }

    #[test]
    fn build_balance_rows_rekeys_sac_via_map_but_not_type3() {
        // A SAC contract-held balance + a type-3 token balance. The map re-keys
        // only the SAC (its surrogate → classic id); the type-3 token is absent
        // and keeps its own surrogate.
        let classic_usdc = ids::asset_id(1, "USDC", 42, 0);
        let sac_classic = HashMap::from([(ids::contract_id("CSAC"), classic_usdc)]);
        let rows = build_balance_rows(
            &[
                ExtractedSorobanBalance {
                    contract_id: "CSAC".into(),
                    holder: "CPOOL".into(),
                    balance: 100,
                    ledger: 10,
                    closed: false,
                },
                ExtractedSorobanBalance {
                    contract_id: "CTOKEN3".into(),
                    holder: "GHOLDER".into(),
                    balance: 200,
                    ledger: 10,
                    closed: false,
                },
            ],
            &sac_classic,
        );

        assert_eq!(
            rows[0].asset_id, classic_usdc,
            "SAC contract-held → classic id"
        );
        assert_eq!(
            rows[1].asset_id,
            ids::contract_id("CTOKEN3"),
            "type-3 unchanged"
        );
    }

    // ------------------------------------------------------------------
    // extract_event_signature — the three mainnet topic conventions plus
    // the monitored fourth arm (task 0517). Shapes are verbatim from the
    // production measurement of 2026-09-02.
    // ------------------------------------------------------------------

    #[test]
    fn signature_from_a_symbol_first_topic() {
        let topics = serde_json::json!([
            {"type": "sym", "value": "transfer"},
            {"type": "address", "value": "GAAAA"}
        ]);
        assert_eq!(
            extract_event_signature(&topics).as_deref(),
            Some("transfer")
        );
    }

    #[test]
    fn signature_from_the_label_convention() {
        // SoroswapPair / DeFindexVault / BlendStrategy: a String protocol
        // label first, the Symbol name second. The label is NOT lifted.
        let topics = serde_json::json!([
            {"type": "string", "value": "SoroswapPair"},
            {"type": "sym", "value": "sync"}
        ]);
        assert_eq!(extract_event_signature(&topics).as_deref(), Some("sync"));
    }

    #[test]
    fn signature_from_the_phoenix_plain_str_convention() {
        // Phoenix publishes ("swap", "sender") as two Strings — the FIRST
        // is the name, the second discriminates the field.
        let topics = serde_json::json!([
            {"type": "string", "value": "swap"},
            {"type": "string", "value": "sender"}
        ]);
        assert_eq!(extract_event_signature(&topics).as_deref(), Some("swap"));
        // Single-String and String+bytes variants of the same family.
        let single = serde_json::json!([{"type": "string", "value": "Message"}]);
        assert_eq!(extract_event_signature(&single).as_deref(), Some("Message"));
        let with_bytes = serde_json::json!([
            {"type": "string", "value": "OrderCreated"},
            {"type": "bytes", "value": "AAAA"}
        ]);
        assert_eq!(
            extract_event_signature(&with_bytes).as_deref(),
            Some("OrderCreated")
        );
    }

    #[test]
    fn an_unknown_convention_keeps_null_and_an_empty_vector_stays_silent() {
        // A hypothetical fourth convention (non-sym, non-string first
        // topic) resolves nowhere — NULL plus the monitor warn.
        let unknown = serde_json::json!([
            {"type": "u64", "value": "7"},
            {"type": "sym", "value": "name_here_is_not_taken"}
        ]);
        assert_eq!(extract_event_signature(&unknown), None);
        // No topics — nothing to resolve, silently.
        assert_eq!(extract_event_signature(&serde_json::json!([])), None);
        // Empty strings never become names.
        let empty = serde_json::json!([{"type": "string", "value": ""}]);
        assert_eq!(extract_event_signature(&empty), None);
    }
}
