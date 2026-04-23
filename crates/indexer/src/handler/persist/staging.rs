//! Pre-transaction staging — all synchronous prep that has to happen before
//! `pool.begin()` so the DB transaction window is pure I/O:
//!
//! * Collect every StrKey referenced anywhere → `accounts` universe
//! * Hex-decode every 32-byte hash once → reusable `[u8; 32]`
//! * Unpack `operations.details` JSON into typed column values
//! * Collapse contract events into `soroban_events_appearances` rows
//!   (ADR 0033 — one row per `(contract, tx, ledger)` trio)
//! * Split `account_balances_current` rows into native (NULL code/issuer) and
//!   credit (both NOT NULL) per the `ck_abc_native` CHECK
//! * Derive `transactions.has_soroban` from presence of events/invocations
//! * Build tx_participants union (source + op destinations + invokers + …)

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use domain::{
    AssetType, ContractEventType, ContractType, NftEventType, OperationType, TokenAssetType,
};
use serde_json::Value;
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedContractInterface, ExtractedEvent,
    ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedLpPosition, ExtractedNft, ExtractedNftEvent, ExtractedOperation, ExtractedToken,
    ExtractedTransaction,
};

use super::HandlerError;

// ---------------------------------------------------------------------------
// Row DTOs — the shape the write layer binds directly.
// ---------------------------------------------------------------------------

/// `transactions` row, ready for UNNEST. `source_str_key` is resolved to
/// `accounts.id` by the write layer.
pub(super) struct TxRow {
    pub hash_hex: String,
    pub hash: [u8; 32],
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_str_key: String,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<[u8; 32]>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub parse_error: bool,
    pub created_at: DateTime<Utc>,
}

/// `operations` row. `source` and `destination` StrKeys are resolved by the
/// write layer; `pool_id` is pre-decoded.
pub(super) struct OpRow {
    pub tx_hash_hex: String,
    pub application_order: i16,
    pub op_type: OperationType,
    pub source_str_key: Option<String>,
    pub destination_str_key: Option<String>,
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer_str_key: Option<String>,
    pub pool_id: Option<[u8; 32]>,
    pub transfer_amount: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// `soroban_events_appearances` staging row (ADR 0033).
///
/// Carries only the minimum needed to aggregate into the 4-column
/// appearance index — `contract_id` (StrKey, resolved to BIGINT FK in the
/// write layer), plus the transaction-identity key. Diagnostic events are
/// filtered out before staging (they live only on the S3 read path).
pub(super) struct EventRow {
    pub tx_hash_hex: String,
    pub contract_id: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// `soroban_invocations_appearances` row (pre-aggregation, one per
/// invocation-tree node). ADR 0034: `contract_id` + `tx_hash_hex` +
/// `ledger_sequence` + `created_at` form the aggregation key;
/// `caller_str_key` is collapsed to the trio's first non-NULL caller
/// at write time. Per-node detail (function name, per-node index,
/// successful, args, return value, depth) is not carried — the API
/// re-extracts it from XDR at read time via
/// `xdr_parser::extract_invocations`.
pub(super) struct InvRow {
    pub tx_hash_hex: String,
    pub contract_id: Option<String>,
    pub caller_str_key: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

pub(super) struct ParticipantRow {
    pub tx_hash_hex: String,
    pub account_str_key: String,
    pub created_at: DateTime<Utc>,
}

pub(super) struct PoolRow {
    pub pool_id: [u8; 32],
    pub asset_a_type: AssetType,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer_str_key: Option<String>,
    pub asset_b_type: AssetType,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer_str_key: Option<String>,
    pub fee_bps: i32,
    pub created_at_ledger: Option<i64>,
    pub last_updated_ledger: i64,
}

pub(super) struct SnapshotRow {
    pub pool_id: [u8; 32],
    pub ledger_sequence: i64,
    pub reserve_a: String,
    pub reserve_b: String,
    pub total_shares: String,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub(super) struct LpPositionRow {
    pub pool_id: [u8; 32],
    pub account_str_key: String,
    pub shares: String,
    pub first_deposit_ledger: Option<i64>,
    pub last_updated_ledger: i64,
}

pub(super) struct NftRow {
    pub contract_id: String,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub metadata: Option<Value>,
    pub minted_at_ledger: Option<i64>,
    pub current_owner_str_key: Option<String>,
    pub current_owner_ledger: Option<i64>,
}

pub(super) struct NftOwnershipRow {
    pub contract_id: String,
    pub token_id: String,
    pub tx_hash_hex: String,
    pub owner_str_key: Option<String>,
    pub event_type: NftEventType,
    pub ledger_sequence: i64,
    pub event_order: i16,
    pub created_at: DateTime<Utc>,
}

pub(super) struct TokenRow {
    pub asset_type: TokenAssetType,
    pub asset_code: Option<String>,
    pub issuer_str_key: Option<String>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
}

pub(super) struct WasmRow {
    pub wasm_hash: [u8; 32],
    pub metadata: Value,
}

pub(super) struct ContractRow {
    pub contract_id: String,
    pub wasm_hash: Option<[u8; 32]>,
    pub wasm_uploaded_at_ledger: Option<i64>,
    pub deployer_str_key: Option<String>,
    pub deployed_at_ledger: Option<i64>,
    pub contract_type: ContractType,
    pub is_sac: bool,
    pub metadata: Option<Value>,
}

/// Either a native-XLM balance (all identifying cols NULL) or a credit-asset
/// balance (`asset_code` + `issuer_str_key` both NOT NULL) — matches the
/// `ck_abc_native` CHECK on `account_balances_current`.
pub(super) struct BalanceRow {
    pub account_str_key: String,
    pub asset_type: AssetType,
    pub asset_code: Option<String>,
    pub issuer_str_key: Option<String>,
    pub balance: String,
    pub last_updated_ledger: i64,
    pub created_at: DateTime<Utc>,
}

pub(super) struct TrustlineRemoval {
    pub account_str_key: String,
    pub asset_code: String,
    pub issuer_str_key: String,
}

/// Watermark-guarded overrides the `accounts` upsert applies when the parser
/// produced a real state change (vs. a pure StrKey reference).
pub(super) struct AccountStateOverride {
    pub first_seen_ledger: Option<i64>,
    pub sequence_number: i64,
    pub home_domain: Option<String>,
}

// ---------------------------------------------------------------------------
// Staged — aggregate of everything ready to bind.
// ---------------------------------------------------------------------------

pub(super) struct Staged {
    pub ledger_sequence: u32,
    pub ledger_sequence_i64: i64,
    pub ledger_hash: [u8; 32],
    pub ledger_closed_at: DateTime<Utc>,
    pub ledger_protocol_version: i32,
    pub ledger_transaction_count: i32,
    pub ledger_base_fee: i64,

    pub account_keys: Vec<String>,
    pub account_state_overrides: HashMap<String, AccountStateOverride>,

    pub wasm_rows: Vec<WasmRow>,
    pub contract_rows: Vec<ContractRow>,
    /// Task 0118 Phase 2 — classification derived from every wasm spec
    /// observed this ledger. Keyed by `wasm_hash`. Non-`Other` values drive
    /// the post-wasm `soroban_contracts.contract_type` UPDATE and the
    /// staging-time override applied to `contract_rows` built in this
    /// pass. `Other` entries are intentionally retained so callers can
    /// tell "we saw a spec but it didn't classify" from "we haven't seen
    /// a spec at all" — only the definitive variants are forwarded to the
    /// DB UPDATE or the per-worker cache.
    pub wasm_classification: HashMap<[u8; 32], ContractType>,

    pub tx_rows: Vec<TxRow>,

    pub participant_rows: Vec<ParticipantRow>,
    pub op_rows: Vec<OpRow>,
    pub event_rows: Vec<EventRow>,
    pub inv_rows: Vec<InvRow>,

    pub pool_rows: Vec<PoolRow>,
    pub snapshot_rows: Vec<SnapshotRow>,
    pub lp_position_rows: Vec<LpPositionRow>,

    pub token_rows: Vec<TokenRow>,

    pub nft_rows: Vec<NftRow>,
    pub nft_ownership_rows: Vec<NftOwnershipRow>,

    pub balance_rows: Vec<BalanceRow>,
    pub trustline_removals: Vec<TrustlineRemoval>,
    pub balance_history_rows: Vec<BalanceRow>,
}

impl Staged {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
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
        tokens: &[ExtractedToken],
        nfts: &[ExtractedNft],
        nft_events: &[ExtractedNftEvent],
        lp_positions: &[ExtractedLpPosition],
        inner_tx_hashes: &HashMap<String, Option<String>>,
    ) -> Result<Self, HandlerError> {
        let ledger_hash = decode_hash(&ledger.hash, "ledger.hash")?;
        let ledger_closed_at = ts_from_unix(ledger.closed_at)?;
        let ledger_sequence_i64 = i64::from(ledger.sequence);

        // --- Accounts universe + per-tx participant set -----------------------
        let mut account_keys_set: HashSet<String> = HashSet::new();
        let mut participants_per_tx: HashMap<String, HashSet<String>> = HashMap::new();
        let has_soroban: HashMap<String, bool> = tx_has_soroban_map(events, invocations);

        for tx in transactions {
            account_keys_set.insert(tx.source_account.clone());
            participants_per_tx
                .entry(tx.hash.clone())
                .or_default()
                .insert(tx.source_account.clone());
        }

        // operations: source override + destinations + issuers + callers + poolIds
        for (tx_hash, ops) in operations {
            let participants = participants_per_tx.entry(tx_hash.clone()).or_default();
            for op in ops {
                if let Some(src) = &op.source_account {
                    account_keys_set.insert(src.clone());
                    participants.insert(src.clone());
                }
                for key in op_participant_str_keys(op.op_type, &op.details) {
                    account_keys_set.insert(key.clone());
                    participants.insert(key);
                }
            }
        }

        for (tx_hash, evs) in events {
            let participants = participants_per_tx.entry(tx_hash.clone()).or_default();
            for ev in evs {
                if let Some((from, to)) = xdr_parser::transfer_participants(&ev.topics) {
                    account_keys_set.insert(from.clone());
                    account_keys_set.insert(to.clone());
                    participants.insert(from);
                    participants.insert(to);
                }
            }
        }

        for (tx_hash, invs) in invocations {
            let participants = participants_per_tx.entry(tx_hash.clone()).or_default();
            for inv in invs {
                if let Some(caller) = &inv.caller_account
                    && is_strkey_account(caller)
                {
                    account_keys_set.insert(caller.clone());
                    participants.insert(caller.clone());
                }
            }
        }

        for dep in contract_deployments {
            if let Some(deployer) = &dep.deployer_account {
                account_keys_set.insert(deployer.clone());
            }
        }
        for st in account_states {
            account_keys_set.insert(st.account_id.clone());
            for b in st.balances.as_array().into_iter().flatten() {
                if let Some(issuer) = b.get("issuer").and_then(Value::as_str)
                    && !issuer.is_empty()
                {
                    account_keys_set.insert(issuer.to_string());
                }
            }
            for rm in &st.removed_trustlines {
                if let Some(issuer) = rm.get("issuer").and_then(Value::as_str)
                    && !issuer.is_empty()
                {
                    account_keys_set.insert(issuer.to_string());
                }
            }
        }
        for pool in liquidity_pools {
            if let Some(issuer) = asset_issuer(&pool.asset_a) {
                account_keys_set.insert(issuer);
            }
            if let Some(issuer) = asset_issuer(&pool.asset_b) {
                account_keys_set.insert(issuer);
            }
        }
        for token in tokens {
            if let Some(issuer) = &token.issuer_address {
                account_keys_set.insert(issuer.clone());
            }
        }
        for nft in nfts {
            if let Some(owner) = &nft.owner_account {
                account_keys_set.insert(owner.clone());
            }
        }
        for ev in nft_events {
            if let Some(owner) = &ev.owner_account {
                account_keys_set.insert(owner.clone());
            }
            participants_per_tx
                .entry(ev.transaction_hash.clone())
                .or_default()
                .extend(ev.owner_account.clone());
        }
        for lpp in lp_positions {
            account_keys_set.insert(lpp.account_id.clone());
        }

        let account_keys: Vec<String> = account_keys_set.into_iter().collect();

        let mut account_state_overrides: HashMap<String, AccountStateOverride> = HashMap::new();
        for st in account_states {
            // sequence_number = -1 is the sentinel for "trustline-only change" —
            // we must not overwrite the existing seq_num with it.
            let seq = if st.sequence_number >= 0 {
                st.sequence_number
            } else {
                0
            };
            account_state_overrides.insert(
                st.account_id.clone(),
                AccountStateOverride {
                    first_seen_ledger: st.first_seen_ledger.map(i64::from),
                    sequence_number: seq,
                    home_domain: st.home_domain.clone(),
                },
            );
        }

        // --- wasm_interface_metadata rows (deduped by wasm_hash) ------------
        let mut wasm_seen: HashSet<[u8; 32]> = HashSet::new();
        let mut wasm_rows: Vec<WasmRow> = Vec::with_capacity(contract_interfaces.len());
        let mut wasm_classification: HashMap<[u8; 32], ContractType> =
            HashMap::with_capacity(contract_interfaces.len());
        for iface in contract_interfaces {
            let hash = decode_hash(&iface.wasm_hash, "wasm_hash")?;
            if !wasm_seen.insert(hash) {
                continue;
            }
            // Task 0118 Phase 2 — run the wasm-spec classifier here so the
            // verdict is available to the contract_rows staging below and
            // to the `reclassify_contracts_from_wasm` write step.
            let classification = xdr_parser::classify_contract_from_wasm_spec(&iface.functions);
            wasm_classification.insert(hash, classification.into());

            let metadata = serde_json::json!({
                "functions": iface.functions,
                "wasm_byte_len": iface.wasm_byte_len,
            });
            wasm_rows.push(WasmRow {
                wasm_hash: hash,
                metadata,
            });
        }

        // --- soroban_contracts rows (deduped by contract_id) ----------------
        let mut contract_seen: HashSet<String> = HashSet::new();
        let mut contract_rows: Vec<ContractRow> = Vec::with_capacity(contract_deployments.len());
        for dep in contract_deployments {
            if !contract_seen.insert(dep.contract_id.clone()) {
                continue;
            }
            let wasm_hash = match &dep.wasm_hash {
                Some(h) => Some(decode_hash(h, "deployment.wasm_hash")?),
                None => None,
            };
            // Task 0118 Phase 2 — if this deployment's wasm_hash was
            // classified in the same ledger and carries a definitive
            // verdict (Nft / Fungible), override the parser default
            // (Other) before the row reaches the DB. SAC deployments stay
            // `Token` (is_sac short-circuits WASM-spec classification —
            // SACs have no WASM).
            let mut contract_type = dep.contract_type;
            if !dep.is_sac
                && let Some(hash) = wasm_hash
                && let Some(&classified) = wasm_classification.get(&hash)
                && matches!(classified, ContractType::Nft | ContractType::Fungible)
            {
                contract_type = classified;
            }
            contract_rows.push(ContractRow {
                contract_id: dep.contract_id.clone(),
                wasm_hash,
                wasm_uploaded_at_ledger: Some(i64::from(dep.deployed_at_ledger)),
                deployer_str_key: dep.deployer_account.clone(),
                deployed_at_ledger: Some(i64::from(dep.deployed_at_ledger)),
                contract_type,
                is_sac: dep.is_sac,
                metadata: Some(dep.metadata.clone()),
            });
        }
        // Also register any contracts referenced by ops/events/invocations that
        // weren't deployed in this ledger — they may already exist in the DB, so
        // the UNNEST upsert will be a DO NOTHING. We skip these here because we
        // can't fabricate metadata; the FK is already satisfied by prior rows.

        // --- transactions rows ---------------------------------------------
        let mut tx_rows: Vec<TxRow> = Vec::with_capacity(transactions.len());
        for (app_order, tx) in transactions.iter().enumerate() {
            let hash = decode_hash(&tx.hash, "tx.hash")?;
            let inner_tx_hash = match inner_tx_hashes.get(&tx.hash).and_then(Option::as_ref) {
                Some(hex_str) => Some(decode_hash(hex_str, "inner_tx_hash")?),
                None => None,
            };
            let has_soroban_flag = *has_soroban.get(&tx.hash).unwrap_or(&false);
            let op_count = operations
                .iter()
                .find(|(h, _)| h == &tx.hash)
                .map(|(_, ops)| ops.len())
                .unwrap_or(0);
            tx_rows.push(TxRow {
                hash_hex: tx.hash.clone(),
                hash,
                ledger_sequence: ledger_sequence_i64,
                application_order: app_order
                    .try_into()
                    .map_err(|_| staging_err("tx application_order overflow"))?,
                source_str_key: tx.source_account.clone(),
                fee_charged: tx.fee_charged,
                inner_tx_hash,
                successful: tx.successful,
                operation_count: op_count
                    .try_into()
                    .map_err(|_| staging_err("operation_count overflow"))?,
                has_soroban: has_soroban_flag,
                parse_error: tx.parse_error,
                created_at: ts_from_unix(tx.created_at)?,
            });
        }

        // --- participants flatten ------------------------------------------
        let mut participant_rows: Vec<ParticipantRow> = Vec::new();
        for tx in &tx_rows {
            let Some(set) = participants_per_tx.get(&tx.hash_hex) else {
                continue;
            };
            for key in set {
                participant_rows.push(ParticipantRow {
                    tx_hash_hex: tx.hash_hex.clone(),
                    account_str_key: key.clone(),
                    created_at: tx.created_at,
                });
            }
        }

        // --- operations flatten + details unpack ---------------------------
        let mut op_rows: Vec<OpRow> = Vec::new();
        let tx_created_at: HashMap<String, DateTime<Utc>> = tx_rows
            .iter()
            .map(|t| (t.hash_hex.clone(), t.created_at))
            .collect();
        for (tx_hash, ops) in operations {
            let Some(&created_at) = tx_created_at.get(tx_hash) else {
                continue;
            };
            for op in ops {
                let typed = OpTyped::from_details(op.op_type, &op.details);
                let pool_id = match &typed.pool_id_hex {
                    Some(hex_str) => Some(decode_hash(hex_str, "op.pool_id")?),
                    None => None,
                };
                op_rows.push(OpRow {
                    tx_hash_hex: tx_hash.clone(),
                    application_order: op
                        .operation_index
                        .try_into()
                        .map_err(|_| staging_err("op application_order overflow"))?,
                    op_type: op.op_type,
                    source_str_key: op.source_account.clone(),
                    destination_str_key: typed.destination,
                    contract_id: typed.contract_id,
                    asset_code: typed.asset_code,
                    asset_issuer_str_key: typed.asset_issuer,
                    pool_id,
                    transfer_amount: typed.transfer_amount,
                    ledger_sequence: ledger_sequence_i64,
                    created_at,
                });
            }
        }

        // --- events flatten for appearance aggregation ---------------------
        //
        // Filter out event_type = "diagnostic". Per Stellar docs these are
        // debug-only traces (fn_call / fn_return / core_metrics / log /
        // error / host_fn_failed) emitted by the Soroban host VM and by
        // stellar-core's InvokeHostFunctionOpFrame; they are explicitly
        // "not hashed into the ledger, and therefore are not part of the
        // protocol" and "not useful for most users". ADR 0033 routes them
        // (and all other event detail) to the public archive; the DB
        // appearance index only counts "contract" and "system" events.
        // On a mainnet sample diagnostic events are ~85 % of event volume
        // and previously dominated events_ms in persist_ledger.
        let mut event_rows: Vec<EventRow> = Vec::new();
        let mut diagnostic_dropped = 0usize;
        for (tx_hash, evs) in events {
            let Some(&created_at) = tx_created_at.get(tx_hash) else {
                continue;
            };
            for ev in evs {
                if ev.event_type == ContractEventType::Diagnostic {
                    diagnostic_dropped += 1;
                    continue;
                }
                event_rows.push(EventRow {
                    tx_hash_hex: tx_hash.clone(),
                    contract_id: ev.contract_id.clone(),
                    ledger_sequence: ledger_sequence_i64,
                    created_at,
                });
            }
        }
        if diagnostic_dropped > 0 {
            tracing::debug!(
                ledger_sequence = ledger.sequence,
                diagnostic_dropped,
                staged = event_rows.len(),
                "staged events for appearance aggregation (diagnostic filtered — S3 lane per ADR 0033)"
            );
        }

        // --- invocations flatten -------------------------------------------
        //
        // One pre-aggregation row per tree node (parser emits depth-first,
        // root before sub-invocations). `insert_invocations` folds these
        // into per-trio appearance rows at write time; carrying the raw
        // node order here keeps the root-caller-wins invariant explicit.
        let mut inv_rows: Vec<InvRow> = Vec::new();
        for (tx_hash, invs) in invocations {
            let Some(&created_at) = tx_created_at.get(tx_hash) else {
                continue;
            };
            for inv in invs {
                let caller = inv
                    .caller_account
                    .as_ref()
                    .filter(|k| is_strkey_account(k))
                    .cloned();
                inv_rows.push(InvRow {
                    tx_hash_hex: tx_hash.clone(),
                    contract_id: inv.contract_id.clone(),
                    caller_str_key: caller,
                    ledger_sequence: ledger_sequence_i64,
                    created_at,
                });
            }
        }

        // --- pools (dedup by pool_id — "latest wins" on repeated updates) --
        let mut pool_indices: HashMap<[u8; 32], usize> = HashMap::new();
        let mut pool_rows: Vec<PoolRow> = Vec::with_capacity(liquidity_pools.len());
        for pool in liquidity_pools {
            let pool_id = decode_hash(&pool.pool_id, "pool_id")?;
            // Skip pools whose asset shape doesn't match a known XDR
            // AssetType — the SMALLINT CHECK on liquidity_pools would
            // reject them anyway, and indexing the rest of the ledger
            // shouldn't fail because of one malformed pool.
            let (Some((a_type, a_code, a_issuer)), Some((b_type, b_code, b_issuer))) = (
                split_pool_asset(&pool.asset_a),
                split_pool_asset(&pool.asset_b),
            ) else {
                continue;
            };
            let row = PoolRow {
                pool_id,
                asset_a_type: a_type,
                asset_a_code: a_code,
                asset_a_issuer_str_key: a_issuer,
                asset_b_type: b_type,
                asset_b_code: b_code,
                asset_b_issuer_str_key: b_issuer,
                fee_bps: pool.fee_bps,
                created_at_ledger: pool.created_at_ledger.map(i64::from),
                last_updated_ledger: i64::from(pool.last_updated_ledger),
            };
            match pool_indices.get(&pool_id).copied() {
                Some(idx) => {
                    let existing = &mut pool_rows[idx];
                    // Keep the earliest `created_at_ledger` (pool creation
                    // watermark) and the latest `last_updated_ledger`. Asset
                    // identity is stable per pool so the typed columns stay.
                    if row.last_updated_ledger >= existing.last_updated_ledger {
                        existing.last_updated_ledger = row.last_updated_ledger;
                    }
                    existing.created_at_ledger =
                        match (existing.created_at_ledger, row.created_at_ledger) {
                            (Some(a), Some(b)) => Some(a.min(b)),
                            (Some(a), None) => Some(a),
                            (None, Some(b)) => Some(b),
                            (None, None) => None,
                        };
                }
                None => {
                    pool_indices.insert(pool_id, pool_rows.len());
                    pool_rows.push(row);
                }
            }
        }

        let mut snapshot_rows: Vec<SnapshotRow> = Vec::with_capacity(pool_snapshots.len());
        for snap in pool_snapshots {
            let pool_id = decode_hash(&snap.pool_id, "snapshot.pool_id")?;
            let reserve_a = snap
                .reserves
                .get("a")
                .and_then(Value::as_i64)
                .map(format_stroops)
                .unwrap_or_else(|| "0.0000000".to_string());
            let reserve_b = snap
                .reserves
                .get("b")
                .and_then(Value::as_i64)
                .map(format_stroops)
                .unwrap_or_else(|| "0.0000000".to_string());
            snapshot_rows.push(SnapshotRow {
                pool_id,
                ledger_sequence: i64::from(snap.ledger_sequence),
                reserve_a,
                reserve_b,
                total_shares: format_stroops_str(&snap.total_shares),
                tvl: snap.tvl.clone(),
                volume: snap.volume.clone(),
                fee_revenue: snap.fee_revenue.clone(),
                created_at: ts_from_unix(snap.created_at)?,
            });
        }

        let mut lp_position_rows: Vec<LpPositionRow> = Vec::with_capacity(lp_positions.len());
        for pos in lp_positions {
            let pool_id = decode_hash(&pos.pool_id, "lp_position.pool_id")?;
            lp_position_rows.push(LpPositionRow {
                pool_id,
                account_str_key: pos.account_id.clone(),
                shares: pos.shares.clone(),
                first_deposit_ledger: pos.first_deposit_ledger.map(i64::from),
                last_updated_ledger: i64::from(pos.last_updated_ledger),
            });
        }

        // --- tokens (dedup by identity — native singleton, classic/sac by
        // (code,issuer), soroban by contract_id; SAC satisfies both uniques so
        // collapse on either key)
        let mut token_rows: Vec<TokenRow> = Vec::with_capacity(tokens.len());
        let mut token_seen: HashSet<String> = HashSet::new();
        for t in tokens {
            // Identity fingerprint matches the per-asset_type partial UNIQUE
            // indexes on `tokens` (uidx_tokens_native / _classic_asset / _soroban).
            // Native is a singleton; classic/SAC dedup by (code, issuer);
            // soroban/SAC dedup by contract_id. SAC satisfies both uniques —
            // we collapse on either key to avoid emitting two rows that the
            // ON CONFLICT would have to merge.
            let fp = match t.asset_type {
                TokenAssetType::Native => "native".to_string(),
                TokenAssetType::Classic => format!(
                    "classic|{}|{}",
                    t.asset_code.as_deref().unwrap_or(""),
                    t.issuer_address.as_deref().unwrap_or("")
                ),
                TokenAssetType::Sac => {
                    format!("sac|{}", t.contract_id.as_deref().unwrap_or(""))
                }
                TokenAssetType::Soroban => {
                    format!("soroban|{}", t.contract_id.as_deref().unwrap_or(""))
                }
            };
            if !token_seen.insert(fp) {
                continue;
            }
            token_rows.push(TokenRow {
                asset_type: t.asset_type,
                asset_code: t.asset_code.clone(),
                issuer_str_key: t.issuer_address.clone(),
                contract_id: t.contract_id.clone(),
                name: t.name.clone(),
                total_supply: t.total_supply.clone(),
                holder_count: t.holder_count,
            });
        }

        // --- nfts (dedup by (contract_id, token_id) — "latest-seen wins")
        let mut nft_indices: HashMap<(String, String), usize> = HashMap::new();
        let mut nft_rows: Vec<NftRow> = Vec::with_capacity(nfts.len());
        for nft in nfts {
            let key = (nft.contract_id.clone(), nft.token_id.clone());
            let incoming_ledger = i64::from(nft.last_seen_ledger);
            match nft_indices.get(&key).copied() {
                Some(idx) => {
                    let existing = &mut nft_rows[idx];
                    // Keep the later ownership watermark; preserve earliest mint.
                    if Some(incoming_ledger) >= existing.current_owner_ledger {
                        existing.current_owner_str_key = nft.owner_account.clone();
                        existing.current_owner_ledger = Some(incoming_ledger);
                    }
                    existing.minted_at_ledger = match (
                        existing.minted_at_ledger,
                        nft.minted_at_ledger.map(i64::from),
                    ) {
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
                    existing.metadata = existing.metadata.clone().or_else(|| nft.metadata.clone());
                }
                None => {
                    nft_indices.insert(key, nft_rows.len());
                    nft_rows.push(NftRow {
                        contract_id: nft.contract_id.clone(),
                        token_id: nft.token_id.clone(),
                        collection_name: nft.collection_name.clone(),
                        name: nft.name.clone(),
                        media_url: nft.media_url.clone(),
                        metadata: nft.metadata.clone(),
                        minted_at_ledger: nft.minted_at_ledger.map(i64::from),
                        current_owner_str_key: nft.owner_account.clone(),
                        current_owner_ledger: Some(incoming_ledger),
                    });
                }
            }
        }

        // --- nft_ownership (from nft_events; empty until parser catches up)
        let mut nft_ownership_rows: Vec<NftOwnershipRow> = Vec::with_capacity(nft_events.len());
        for ev in nft_events {
            nft_ownership_rows.push(NftOwnershipRow {
                contract_id: ev.contract_id.clone(),
                token_id: ev.token_id.clone(),
                tx_hash_hex: ev.transaction_hash.clone(),
                owner_str_key: ev.owner_account.clone(),
                event_type: ev.event_type,
                ledger_sequence: i64::from(ev.ledger_sequence),
                event_order: ev
                    .event_order
                    .try_into()
                    .map_err(|_| staging_err("nft event_order overflow"))?,
                created_at: ts_from_unix(ev.created_at)?,
            });
        }

        // --- balances split into native / credit; dedup by natural identity
        // so ON CONFLICT never fires twice for the same row. Multiple txs in
        // one ledger can touch the same account (issuer-of-issuer, custody
        // flows), and each tx emits its own ExtractedAccountState — so the
        // per-account merge has to happen here. Latest-ledger wins; ties go
        // to the last-seen write order (matches apply-order from the parser).
        let mut balance_index: HashMap<(String, Option<String>, Option<String>), usize> =
            HashMap::new();
        let mut balance_rows: Vec<BalanceRow> = Vec::new();
        let mut trustline_removals_index: HashMap<(String, String, String), bool> = HashMap::new();
        let mut trustline_removals: Vec<TrustlineRemoval> = Vec::new();

        for st in account_states {
            let created_at = ts_from_unix(st.created_at)?;
            let ledger_seq = i64::from(st.last_seen_ledger);

            for b in st.balances.as_array().into_iter().flatten() {
                // Parser embeds asset_type as the canonical XDR string in the
                // balances JSON (state.rs). Map it to the typed enum here so
                // every downstream row carries SMALLINT-compatible state.
                // Rows with an unknown / missing asset_type can't satisfy
                // the ck_abc_* CHECK; skip them rather than fail the ledger.
                let Some(asset_type) = b
                    .get("asset_type")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<AssetType>().ok())
                else {
                    continue;
                };
                let balance = b
                    .get("balance")
                    .and_then(Value::as_str)
                    .unwrap_or("0")
                    .to_string();
                let row = if asset_type == AssetType::Native {
                    BalanceRow {
                        account_str_key: st.account_id.clone(),
                        asset_type,
                        asset_code: None,
                        issuer_str_key: None,
                        balance,
                        last_updated_ledger: ledger_seq,
                        created_at,
                    }
                } else {
                    let code = b
                        .get("asset_code")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let issuer = b
                        .get("issuer")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if code.is_empty() || issuer.is_empty() {
                        // Credit rows require both code and issuer (ck_abc_native /
                        // ck_abh_native). Skip malformed rows rather than violate CHECK.
                        continue;
                    }
                    BalanceRow {
                        account_str_key: st.account_id.clone(),
                        asset_type,
                        asset_code: Some(code),
                        issuer_str_key: Some(issuer),
                        balance,
                        last_updated_ledger: ledger_seq,
                        created_at,
                    }
                };

                let key = (
                    row.account_str_key.clone(),
                    row.asset_code.clone(),
                    row.issuer_str_key.clone(),
                );
                match balance_index.get(&key).copied() {
                    Some(idx) => {
                        let existing = &mut balance_rows[idx];
                        if row.last_updated_ledger >= existing.last_updated_ledger {
                            existing.balance = row.balance;
                            existing.last_updated_ledger = row.last_updated_ledger;
                            existing.created_at = row.created_at;
                        }
                    }
                    None => {
                        balance_index.insert(key, balance_rows.len());
                        balance_rows.push(row);
                    }
                }
            }

            for rm in &st.removed_trustlines {
                let code = rm.get("asset_code").and_then(Value::as_str).unwrap_or("");
                let issuer = rm.get("issuer").and_then(Value::as_str).unwrap_or("");
                if code.is_empty() || issuer.is_empty() {
                    continue;
                }
                // Cross-tx: if the trustline was re-added later this ledger, the
                // merged balance row exists and we must not delete it.
                let still_present = balance_index.contains_key(&(
                    st.account_id.clone(),
                    Some(code.to_string()),
                    Some(issuer.to_string()),
                ));
                if still_present {
                    continue;
                }
                let dedup_key = (st.account_id.clone(), code.to_string(), issuer.to_string());
                if trustline_removals_index.insert(dedup_key, true).is_none() {
                    trustline_removals.push(TrustlineRemoval {
                        account_str_key: st.account_id.clone(),
                        asset_code: code.to_string(),
                        issuer_str_key: issuer.to_string(),
                    });
                }
            }
        }

        // History: one row per (account, asset, ledger) — identical to current
        // state after merging, so we derive it directly from the deduped
        // balance_rows instead of re-walking account_states.
        let balance_history_rows: Vec<BalanceRow> =
            balance_rows.iter().map(clone_balance_row).collect();

        Ok(Self {
            ledger_sequence: ledger.sequence,
            ledger_sequence_i64,
            ledger_hash,
            ledger_closed_at,
            ledger_protocol_version: i32::try_from(ledger.protocol_version)
                .map_err(|_| staging_err("protocol_version overflow"))?,
            ledger_transaction_count: i32::try_from(ledger.transaction_count)
                .map_err(|_| staging_err("transaction_count overflow"))?,
            ledger_base_fee: i64::from(ledger.base_fee),
            account_keys,
            account_state_overrides,
            wasm_rows,
            contract_rows,
            wasm_classification,
            tx_rows,
            participant_rows,
            op_rows,
            event_rows,
            inv_rows,
            pool_rows,
            snapshot_rows,
            lp_position_rows,
            token_rows,
            nft_rows,
            nft_ownership_rows,
            balance_rows,
            trustline_removals,
            balance_history_rows,
        })
    }
}

fn clone_balance_row(src: &BalanceRow) -> BalanceRow {
    BalanceRow {
        account_str_key: src.account_str_key.clone(),
        asset_type: src.asset_type,
        asset_code: src.asset_code.clone(),
        issuer_str_key: src.issuer_str_key.clone(),
        balance: src.balance.clone(),
        last_updated_ledger: src.last_updated_ledger,
        created_at: src.created_at,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn decode_hash(hex_str: &str, field: &'static str) -> Result<[u8; 32], HandlerError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| staging_err(&format!("hex decode {field}: {e} (value={hex_str})")))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        staging_err(&format!(
            "hex length {field}: expected 32 bytes, got {} (value={hex_str})",
            bytes.len()
        ))
    })
}

fn ts_from_unix(seconds: i64) -> Result<DateTime<Utc>, HandlerError> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .ok_or_else(|| staging_err(&format!("timestamp out of range: {seconds}")))
}

fn staging_err(msg: &str) -> HandlerError {
    HandlerError::Staging(msg.to_string())
}

/// Derive `transactions.has_soroban` from the presence of any event or
/// invocation for a given tx. Cheap and exact.
fn tx_has_soroban_map(
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
) -> HashMap<String, bool> {
    let mut out: HashMap<String, bool> = HashMap::new();
    for (tx_hash, evs) in events {
        if !evs.is_empty() {
            out.insert(tx_hash.clone(), true);
        }
    }
    for (tx_hash, invs) in invocations {
        if !invs.is_empty() {
            out.insert(tx_hash.clone(), true);
        }
    }
    out
}

/// Typed column values unpacked from an `ExtractedOperation.details` JSON.
#[derive(Default)]
struct OpTyped {
    destination: Option<String>,
    contract_id: Option<String>,
    asset_code: Option<String>,
    asset_issuer: Option<String>,
    pool_id_hex: Option<String>,
    /// NUMERIC(28,7) as TEXT — written with 7 decimal places from stroops.
    transfer_amount: Option<String>,
}

impl OpTyped {
    fn from_details(op_type: OperationType, details: &Value) -> Self {
        let mut out = Self::default();
        match op_type {
            OperationType::CreateAccount => {
                out.destination = str_field(details, "destination");
                out.transfer_amount = stroops_as_numeric(details.get("startingBalance"));
            }
            OperationType::Payment => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("asset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
                out.transfer_amount = stroops_as_numeric(details.get("amount"));
            }
            OperationType::PathPaymentStrictReceive => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("destAsset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
                out.transfer_amount = stroops_as_numeric(details.get("destAmount"));
            }
            OperationType::PathPaymentStrictSend => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("destAsset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
                out.transfer_amount = stroops_as_numeric(details.get("destMin"));
            }
            OperationType::AccountMerge => {
                out.destination = str_field(details, "destination");
            }
            OperationType::Clawback => {
                out.destination = str_field(details, "from");
                if let Some(asset) = details.get("asset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
                out.transfer_amount = stroops_as_numeric(details.get("amount"));
            }
            OperationType::LiquidityPoolDeposit => {
                out.pool_id_hex = str_field(details, "liquidityPoolId");
            }
            OperationType::LiquidityPoolWithdraw => {
                out.pool_id_hex = str_field(details, "liquidityPoolId");
                out.transfer_amount = stroops_as_numeric(details.get("amount"));
            }
            OperationType::InvokeHostFunction => {
                out.contract_id = str_field(details, "contractId");
            }
            OperationType::ChangeTrust => {
                if let Some(asset) = details.get("asset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
            }
            OperationType::SetTrustLineFlags => {
                out.destination = str_field(details, "trustor");
                if let Some(asset) = details.get("asset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
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
            // Other op types carry no per-row typed columns beyond the base
            // (source / type / created_at). New op types added by future
            // protocol upgrades must extend `OperationType` first; the
            // exhaustive match below is intentionally left to `_` so the
            // compiler doesn't force a dummy arm per addition — all the
            // typed fields stay as their `Option::None` defaults.
            _ => {}
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

/// Operation details assets are encoded as either the literal string `"native"`
/// or `"CODE:ISSUER"` (see `format_asset` in xdr_parser::operation).
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

/// Pool params assets are either the bare string `"native"` or a JSON object
/// `{type, code, issuer}` (see `ExtractedLiquidityPool.asset_a/b`).
fn asset_issuer(asset: &Value) -> Option<String> {
    asset
        .as_object()?
        .get("issuer")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Decode the parser's pool-asset JSON shape into typed columns.
///
/// The parser embeds `"native"` as a bare string and credit / pool_share
/// assets as `{"type": "...", "code": "...", "issuer": "..."}` (see
/// `xdr_parser::ledger_entry_changes`). Returns `None` for anything that
/// doesn't map to a known XDR `AssetType` discriminator — caller must
/// drop the row to avoid violating the SMALLINT CHECK on
/// `liquidity_pools.asset_*_type`.
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

/// Convert raw stroops (i64) to NUMERIC(28,7) decimal string.
fn format_stroops(stroops: i64) -> String {
    let sign = if stroops < 0 { "-" } else { "" };
    let abs = stroops.unsigned_abs();
    let whole = abs / 10_000_000;
    let frac = abs % 10_000_000;
    format!("{sign}{whole}.{frac:07}")
}

/// Accept either an already-formatted decimal ("123.4500000") or a raw stroops
/// string ("12345000") and return NUMERIC-safe text.
fn format_stroops_str(s: &str) -> String {
    if s.contains('.') {
        s.to_string()
    } else if let Ok(n) = s.parse::<i64>() {
        format_stroops(n)
    } else {
        s.to_string()
    }
}

fn stroops_as_numeric(val: Option<&Value>) -> Option<String> {
    let v = val?;
    if let Some(n) = v.as_i64() {
        return Some(format_stroops(n));
    }
    if let Some(s) = v.as_str() {
        return Some(format_stroops_str(s));
    }
    None
}

/// Quick StrKey-account shape check (G... or M..., length 56/69). Used to
/// filter out contract addresses (C...) before hitting `accounts` lookup.
fn is_strkey_account(s: &str) -> bool {
    matches!(s.chars().next(), Some('G' | 'M'))
}

/// Return every StrKey this op implicitly references (destinations, issuers,
/// trustors, sponsored ids). Used to populate `transaction_participants` plus
/// the `accounts` universe.
fn op_participant_str_keys(op_type: OperationType, details: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: Option<String>| {
        if let Some(s) = v
            && is_strkey_account(&s)
        {
            out.push(s);
        }
    };

    use OperationType as Op;
    match op_type {
        Op::CreateAccount
        | Op::Payment
        | Op::PathPaymentStrictReceive
        | Op::PathPaymentStrictSend
        | Op::AccountMerge => {
            push(str_field(details, "destination"));
        }
        Op::Clawback => {
            push(str_field(details, "from"));
        }
        Op::AllowTrust | Op::SetTrustLineFlags => {
            push(str_field(details, "trustor"));
        }
        Op::BeginSponsoringFutureReserves => {
            push(str_field(details, "sponsoredId"));
        }
        _ => {}
    }

    // Asset issuers (from typed `asset`/`destAsset`/`sendAsset` fields)
    for field in ["asset", "destAsset", "sendAsset"] {
        if let Some(asset) = details.get(field) {
            let (_, issuer) = split_asset_ref(asset);
            push(issuer);
        }
    }

    out
}
