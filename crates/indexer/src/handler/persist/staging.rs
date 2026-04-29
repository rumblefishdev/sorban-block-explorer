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
//! * Derive `transactions.has_soroban` strictly from envelope op type
//!   (`InvokeHostFunction | ExtendFootprintTtl | RestoreFootprint`)
//! * Build tx_participants union (source + op destinations + invokers + …)

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use domain::{AssetType, ContractType, NftEventType, OperationType, TokenAssetType};
use serde_json::Value;
use xdr_parser::types::{
    EventSource, ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment,
    ExtractedContractInterface, ExtractedEvent, ExtractedInvocation, ExtractedLedger,
    ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft,
    ExtractedNftEvent, ExtractedOperation, ExtractedTransaction,
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

/// `operations_appearances` row (task 0163). `source` and `destination`
/// StrKeys are resolved by the write layer; `pool_id` is pre-decoded.
/// `amount` counts how many operations of identical identity were folded
/// into this row within the same transaction. Identity keys match the
/// `uq_ops_app_identity` UNIQUE NULLS NOT DISTINCT constraint.
pub(super) struct OpRow {
    pub tx_hash_hex: String,
    pub op_type: OperationType,
    pub source_str_key: Option<String>,
    pub destination_str_key: Option<String>,
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer_str_key: Option<String>,
    pub pool_id: Option<[u8; 32]>,
    pub amount: i64,
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

pub(super) struct AssetRow {
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

    pub asset_rows: Vec<AssetRow>,

    pub nft_rows: Vec<NftRow>,
    pub nft_ownership_rows: Vec<NftOwnershipRow>,

    pub balance_rows: Vec<BalanceRow>,
    pub trustline_removals: Vec<TrustlineRemoval>,
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
        assets: &[ExtractedAsset],
        nfts: &[ExtractedNft],
        nft_events: &[ExtractedNftEvent],
        lp_positions: &[ExtractedLpPosition],
    ) -> Result<Self, HandlerError> {
        let ledger_hash = decode_hash(&ledger.hash, "ledger.hash")?;
        let ledger_closed_at = ts_from_unix(ledger.closed_at)?;
        let ledger_sequence_i64 = i64::from(ledger.sequence);

        // --- Accounts universe + per-tx participant set -----------------------
        let mut account_keys_set: HashSet<String> = HashSet::new();
        let mut participants_per_tx: HashMap<String, HashSet<String>> = HashMap::new();
        let has_soroban: HashMap<String, bool> = tx_has_soroban_map(operations);

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
                    // CAP-67 unification (Protocol 23+) routes classic-op
                    // SAC `transfer` events through per-operation event
                    // slots, and those events can carry non-account
                    // ScAddress variants in their topics — most commonly
                    // ClaimableBalance (B…, 58 chars) and LiquidityPool
                    // (L…), neither of which is an account and neither
                    // fits `accounts.account_id VARCHAR(56)`. Filter to
                    // the same G/M-account shape the invocations and
                    // operations paths use; mismatched sides are skipped
                    // independently so a (B, G) transfer still tracks G.
                    for participant in [from, to] {
                        if is_strkey_account(&participant) {
                            account_keys_set.insert(participant.clone());
                            participants.insert(participant);
                        }
                    }
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
        for asset in assets {
            if let Some(issuer) = &asset.issuer_address {
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

        // Defense-in-depth shape filter for `accounts.account_id VARCHAR(56)`.
        // Upstream collectors (events / invocations / ops / participants /
        // account_states / NFT events / liquidity_pools) each apply their
        // own filter, but CAP-67 unification (Protocol 23+) surfaces
        // ScAddress variants whose StrKey rendering exceeds 56 chars in
        // event topics — most commonly ClaimableBalance (B…, 58 chars) and
        // MuxedAccount (M…, 69 chars). These are not accounts and must not
        // reach the column. Drop with an aggregate debug log so a single
        // ledger with hundreds of leaks doesn't spam the trace. Length is
        // `<= 56` rather than `== 56` so test fixtures with hand-crafted
        // shorter G-prefix strkeys still pass; real Stellar G-keys are
        // always exactly 56 chars and fit either way.
        //
        // KNOWN GAP: this filter changes the failure mode for a tx with
        // muxed source (M…) from PG VARCHAR overflow at accounts insert to
        // a resolve-id miss at write::insert_transactions. Both fail
        // loudly. The proper fix is canonicalize M → underlying G at the
        // parser level so that all downstream stages — accounts upsert,
        // tx source resolve, op participants resolve — see the same
        // 56-char G-key. Tracked separately in backlog task 0177 (muxed
        // transaction source leaks 69-char M-key into accounts.account_id
        // VARCHAR(56)).
        let total_keys = account_keys_set.len();
        let account_keys: Vec<String> = account_keys_set
            .into_iter()
            .filter(|k| k.len() <= 56 && k.starts_with('G'))
            .collect();
        let dropped_oversize = total_keys - account_keys.len();
        if dropped_oversize > 0 {
            tracing::debug!(
                ledger_sequence = ledger.sequence,
                dropped_oversize,
                kept = account_keys.len(),
                "dropped non-G-prefix or oversize StrKeys from accounts staging \
                 (CAP-67 non-account ScAddress variants in event topics)"
            );
        }

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
        for (idx, tx) in transactions.iter().enumerate() {
            // 1-based to match Stellar ecosystem convention (Horizon paging_token,
            // stellar-core, stellar.expert all use 1-based application_order).
            // See task 0172 / ADR 0028.
            let app_order = idx + 1;
            let hash = decode_hash(&tx.hash, "tx.hash")?;
            let inner_tx_hash = match tx.inner_tx_hash.as_deref() {
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

        // --- operations flatten + identity aggregation ---------------------
        //
        // Task 0163: collapse operations with identical identity within a
        // transaction into a single appearance row with `amount = COUNT(*)`.
        // Identity columns match the DB `uq_ops_app_identity` constraint.
        // Per-op detail (transfer amount, application order, memo, claimants,
        // function args, …) is not carried — the API re-materialises it from
        // XDR via `xdr_parser::extract_operations`.
        type OpIdentity = (
            String,           // tx_hash_hex
            OperationType,    // op_type
            Option<String>,   // source_str_key
            Option<String>,   // destination_str_key
            Option<String>,   // contract_id
            Option<String>,   // asset_code
            Option<String>,   // asset_issuer_str_key
            Option<[u8; 32]>, // pool_id
            i64,              // ledger_sequence
            DateTime<Utc>,    // created_at
        );
        let tx_created_at: HashMap<String, DateTime<Utc>> = tx_rows
            .iter()
            .map(|t| (t.hash_hex.clone(), t.created_at))
            .collect();
        let mut op_agg: HashMap<OpIdentity, i64> = HashMap::new();
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
                let key: OpIdentity = (
                    tx_hash.clone(),
                    op.op_type,
                    op.source_account.clone(),
                    typed.destination,
                    typed.contract_id,
                    typed.asset_code,
                    typed.asset_issuer,
                    pool_id,
                    ledger_sequence_i64,
                    created_at,
                );
                *op_agg.entry(key).or_insert(0) += 1;
            }
        }
        let mut op_rows: Vec<OpRow> = op_agg
            .into_iter()
            .map(|(k, amount)| OpRow {
                tx_hash_hex: k.0,
                op_type: k.1,
                source_str_key: k.2,
                destination_str_key: k.3,
                contract_id: k.4,
                asset_code: k.5,
                asset_issuer_str_key: k.6,
                pool_id: k.7,
                amount,
                ledger_sequence: k.8,
                created_at: k.9,
            })
            .collect();
        // Deterministic order for downstream chunking / replay — full identity
        // tuple is the tie-breaker, otherwise rows sharing a type in the same
        // tx fall back to HashMap iteration order.
        op_rows.sort_by(|a, b| {
            (
                a.tx_hash_hex.as_str(),
                a.op_type as i16,
                a.source_str_key.as_deref(),
                a.destination_str_key.as_deref(),
                a.contract_id.as_deref(),
                a.asset_code.as_deref(),
                a.asset_issuer_str_key.as_deref(),
                a.pool_id.as_ref(),
                a.ledger_sequence,
                a.created_at,
            )
                .cmp(&(
                    b.tx_hash_hex.as_str(),
                    b.op_type as i16,
                    b.source_str_key.as_deref(),
                    b.destination_str_key.as_deref(),
                    b.contract_id.as_deref(),
                    b.asset_code.as_deref(),
                    b.asset_issuer_str_key.as_deref(),
                    b.pool_id.as_ref(),
                    b.ledger_sequence,
                    b.created_at,
                ))
        });

        // --- events flatten for appearance aggregation ---------------------
        //
        // Drop the entire `*.diagnostic_events` container — debug-only
        // traces (fn_call / fn_return / core_metrics / log / error /
        // host_fn_failed) per Stellar docs, "not hashed into the ledger,
        // and therefore are not part of the protocol" (CAP-67). ADR 0033
        // routes diagnostic detail to the public archive; the DB
        // appearance index only counts consensus events.
        //
        // Filter on `EventSource::Diagnostic` — NOT on inner
        // `event_type == Diagnostic`. Stellar core mirrors every per-op
        // consensus Contract event into `v4.diagnostic_events` with
        // inner `type_ = Contract` (byte-identical), so a type-based
        // filter passes the duplicate through and inflates `amount` by
        // the per-op event count. Container-based filter drops both the
        // host-VM trace entries and the Contract-typed mirrors in one
        // step (task 0182).
        //
        // On a mainnet sample diagnostic events are ~85 % of event
        // volume and previously dominated events_ms in persist_ledger.
        let mut event_rows: Vec<EventRow> = Vec::new();
        let mut diagnostic_dropped = 0usize;
        for (tx_hash, evs) in events {
            let Some(&created_at) = tx_created_at.get(tx_hash) else {
                continue;
            };
            for ev in evs {
                if ev.source == EventSource::Diagnostic {
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
                "staged events for appearance aggregation (diagnostic container dropped — S3 lane per ADR 0033)"
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

        // Dedup by `(pool_id, account_id)` — the `lp_positions` UPSERT
        // lives inside `write::upsert_pools_and_snapshots` (`write.rs`)
        // as a single `INSERT … FROM UNNEST … ON CONFLICT
        // (pool_id, account_id) DO UPDATE`, and Postgres rejects
        // multiple proposed rows hitting the same conflict target in
        // one command:
        //   "ON CONFLICT DO UPDATE command cannot affect row a second time"
        // The parser legitimately emits more than one position per
        // `(pool, account)` per ledger when several txs touch the same
        // pool_share trustline — collapse to the latest by
        // `last_updated_ledger`. Equal-ledger ties keep the last-seen
        // entry (Stellar replays in operation order, so last-seen
        // matches the post-ledger state). `first_deposit_ledger` is
        // taken from whichever entry carried it (parser only sets it on
        // `created`); falling back to the latest's value keeps the
        // pre-existing `unwrap_or(last_updated_ledger)` shim in
        // `write.rs` valid for the ON CONFLICT side, while a true
        // earliest-Created in the same batch wins by appearing first
        // and being preserved by the `>=` comparison below.
        use std::collections::hash_map::Entry;
        let mut lp_position_dedup: HashMap<([u8; 32], String), LpPositionRow> = HashMap::new();
        for pos in lp_positions {
            let pool_id = decode_hash(&pos.pool_id, "lp_position.pool_id")?;
            let key = (pool_id, pos.account_id.clone());
            let new_row = LpPositionRow {
                pool_id,
                account_str_key: pos.account_id.clone(),
                shares: pos.shares.clone(),
                first_deposit_ledger: pos.first_deposit_ledger.map(i64::from),
                last_updated_ledger: i64::from(pos.last_updated_ledger),
            };
            match lp_position_dedup.entry(key) {
                Entry::Occupied(mut occ) => {
                    let existing = occ.get_mut();
                    if new_row.last_updated_ledger >= existing.last_updated_ledger {
                        // Preserve the first-seen `first_deposit_ledger`
                        // if the newcomer drops it (parser emits None on
                        // updated/restored/removed) — matches the intent
                        // of the LEAST() merge in write.rs.
                        let preserved_first = new_row
                            .first_deposit_ledger
                            .or(existing.first_deposit_ledger);
                        *existing = new_row;
                        existing.first_deposit_ledger = preserved_first;
                    } else if existing.first_deposit_ledger.is_none() {
                        existing.first_deposit_ledger = new_row.first_deposit_ledger;
                    }
                }
                Entry::Vacant(vac) => {
                    vac.insert(new_row);
                }
            }
        }
        let lp_position_rows: Vec<LpPositionRow> = lp_position_dedup.into_values().collect();

        // --- assets (dedup by identity — native singleton, classic_credit/sac by
        // (code,issuer), soroban by contract_id; SAC satisfies both uniques so
        // collapse on either key)
        let mut asset_rows: Vec<AssetRow> = Vec::with_capacity(assets.len());
        let mut asset_seen: HashSet<String> = HashSet::new();
        for t in assets {
            // Identity fingerprint matches the per-asset_type partial UNIQUE
            // indexes on `assets` (uidx_assets_native / _classic_asset / _soroban).
            // Native is a singleton; classic_credit/SAC dedup by (code, issuer);
            // soroban/SAC dedup by contract_id. SAC satisfies both uniques —
            // we collapse on either key to avoid emitting two rows that the
            // ON CONFLICT would have to merge.
            let fp = match t.asset_type {
                TokenAssetType::Native => "native".to_string(),
                TokenAssetType::ClassicCredit => format!(
                    "classic_credit|{}|{}",
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
            if !asset_seen.insert(fp) {
                continue;
            }
            asset_rows.push(AssetRow {
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
            asset_rows,
            nft_rows,
            nft_ownership_rows,
            balance_rows,
            trustline_removals,
        })
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

/// Derive `transactions.has_soroban` strictly from the operation list of
/// the (correctly-unwrapped) envelope: `true` iff the tx carries at least
/// one `INVOKE_HOST_FUNCTION` / `EXTEND_FOOTPRINT_TTL` / `RESTORE_FOOTPRINT`.
///
/// Events and invocations are NOT used as the signal: a classic payment on
/// a SAC-backed asset (HELIX, USDC, EURC, …) emits SAC transfer events as a
/// side-effect, which would make every such tx look "soroban" to a loose
/// derivation. The field name + the `idx_tx_has_soroban` partial index both
/// imply the strict reading — only txs whose **author** wrote a Soroban op.
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

/// Identity columns for the `operations_appearances` natural key (task 0163).
/// Transfer amount, starting balance, etc. are intentionally *not* extracted —
/// per-op detail lives in XDR and is re-materialised by the API.
#[derive(Default)]
struct OpTyped {
    destination: Option<String>,
    contract_id: Option<String>,
    asset_code: Option<String>,
    asset_issuer: Option<String>,
    pool_id_hex: Option<String>,
}

impl OpTyped {
    fn from_details(op_type: OperationType, details: &Value) -> Self {
        let mut out = Self::default();
        match op_type {
            OperationType::CreateAccount => {
                out.destination = str_field(details, "destination");
            }
            OperationType::Payment => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("asset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
            }
            OperationType::PathPaymentStrictReceive | OperationType::PathPaymentStrictSend => {
                out.destination = str_field(details, "destination");
                if let Some(asset) = details.get("destAsset") {
                    let (code, issuer) = split_asset_ref(asset);
                    out.asset_code = code;
                    out.asset_issuer = issuer;
                }
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
            }
            OperationType::LiquidityPoolDeposit | OperationType::LiquidityPoolWithdraw => {
                out.pool_id_hex = str_field(details, "liquidityPoolId");
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
            // Other op types carry no identity columns beyond the base
            // (source / type / created_at). New op types added by future
            // protocol upgrades must extend `OperationType` first; the
            // match below is intentionally left to `_` so the compiler
            // doesn't force a dummy arm per addition — all identity
            // fields stay as their `Option::None` defaults.
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
