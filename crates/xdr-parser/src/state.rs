//! Derived state extraction from raw ledger entry changes.
//!
//! Processes `ExtractedLedgerEntryChange` records to produce higher-level
//! entities: contract deployments, account states, liquidity pools,
//! assets, and NFTs. This is the final parsing stage before DB persistence.

use std::collections::HashMap;

use serde_json::Value;
use tracing::warn;

use crate::classification::{ContractClassification, classify_contract_from_wasm_spec};
use crate::types::{
    ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedLedgerEntryChange, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedLpPosition, ExtractedNft, NftEvent, SacAssetIdentity,
};
use domain::{ContractType, TokenAssetType};

// ---------------------------------------------------------------------------
// Step 1 + Step 7: Contract Deployment + SAC Detection
// ---------------------------------------------------------------------------

/// Extract contract deployments from ledger entry changes.
///
/// Identifies new contract instances by looking for `contract_data` entries
/// with the contract instance key. Detects SACs from the executable type.
///
/// `sac_identities` maps `contract_id` (the deterministic preimage hash
/// per stellar-core, see `crate::sac::derive_sac_contract_id`) to the
/// underlying classic asset for every SAC found in the current batch's
/// transaction envelopes (top-level `CreateContract` ops AND
/// `CreateContractHostFn` auth entries — task 0160). For SAC
/// deployments without a matching identity (e.g. replay from
/// mid-ledger without the original deploy tx) the deployment still
/// lands here with `sac_asset: None`; `detect_assets` then skips the
/// asset row with a `tracing::warn` rather than fabricate one.
pub fn extract_contract_deployments(
    changes: &[ExtractedLedgerEntryChange],
    tx_source_account: &str,
    sac_identities: &HashMap<String, SacAssetIdentity>,
) -> Vec<ExtractedContractDeployment> {
    let mut deployments = Vec::new();

    for change in changes {
        if change.entry_type != "contract_data" || change.change_type != "created" {
            continue;
        }
        let Some(ref data) = change.data else {
            continue;
        };
        if !is_contract_instance_key(&change.key) {
            continue;
        }

        let contract_id = change
            .key
            .get("contract")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if contract_id.is_empty() {
            continue;
        }

        let is_sac = is_sac_from_data(data);
        let wasm_hash = extract_wasm_hash(data);
        // ADR 0031: synthetic 2-variant classification. SACs wrap a classic
        // asset and are always assets; everything else is `Other` until the
        // explorer learns to recognise a richer taxonomy.
        let contract_type = if is_sac {
            ContractType::Token
        } else {
            ContractType::Other
        };

        // Task 0160: SAC identity is keyed by the deterministic preimage
        // hash (== contract_id). Lookup is O(1) and correlation-free —
        // works across multi-SAC tx, factory deploys (auth entries), and
        // batch boundaries.
        let sac_asset = if is_sac {
            sac_identities.get(&contract_id).cloned()
        } else {
            None
        };

        deployments.push(ExtractedContractDeployment {
            contract_id,
            wasm_hash,
            deployer_account: Some(tx_source_account.to_string()),
            deployed_at_ledger: change.ledger_sequence,
            contract_type,
            is_sac,
            name: None,
            sac_asset,
        });
    }

    // Second pass — populate `name` for the constructor pattern (deploy
    // tx writes the standard `Symbol("name")` storage entry in the same
    // ledger). For deploy-then-init contracts where the storage write
    // lands in a later ledger, the indexer's
    // `extract_contract_data_name_writes` path fills the column then.
    // Per ADR 0041 + task 0156.
    for deployment in deployments.iter_mut() {
        for change in changes {
            if change.entry_type != "contract_data" || change.change_type != "created" {
                continue;
            }
            if !is_symbol_name_key(&change.key, &deployment.contract_id) {
                continue;
            }
            let Some(ref data) = change.data else {
                continue;
            };
            if let Some(name) = decode_scval_string(data) {
                deployment.name = Some(name);
                break;
            }
        }
    }

    deployments
}

/// Extract `(contract_id, name)` pairs from `Symbol("name")` ContractData
/// `created` or `updated` entries, independently of any deployment in the
/// same ledger.
///
/// Used for two scenarios that `extract_contract_deployments`'s
/// constructor-pattern second pass cannot cover:
///
/// 1. **Late-init pattern** — contract deployed in ledger N (storage
///    empty at deploy time), `init()` invocation in ledger N+k writes
///    `Symbol("name")` to persistent storage. `extract_contract_deployments`
///    in ledger N+k produces no deployment for this contract (it was
///    already deployed), so the second pass there sees no deployment to
///    populate. The indexer applies a retroactive UPDATE on
///    `soroban_contracts.name` for each pair returned here, gated by
///    `name IS NULL` to keep the write idempotent.
///
/// 2. **Re-init / name update** — a contract updates its `Symbol("name")`
///    storage entry. The `change_type == "updated"` filter catches this
///    case as well; the indexer overwrites the existing name.
///
/// Per ADR 0041 + task 0156.
pub fn extract_contract_data_name_writes(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for change in changes {
        if change.entry_type != "contract_data" {
            continue;
        }
        if change.change_type != "created" && change.change_type != "updated" {
            continue;
        }
        let Some(contract_id) = extract_contract_id_from_key(&change.key) else {
            continue;
        };
        if !is_symbol_name_key(&change.key, &contract_id) {
            continue;
        }
        let Some(ref data) = change.data else {
            continue;
        };
        if let Some(name) = decode_scval_string(data) {
            out.push((contract_id, name));
        }
    }
    out
}

/// True when `key` is the persistent storage entry for `Symbol("name")` on
/// `contract_id` (the standard slot used by SEP-41 / OpenZeppelin Soroban
/// FungibleToken implementations to store the human-readable token name).
///
/// Match shape: `key.contract == contract_id` AND
/// `key.key.type == "sym"` AND `key.key.value == "name"`.
fn is_symbol_name_key(key: &Value, contract_id: &str) -> bool {
    let key_contract = key.get("contract").and_then(|v| v.as_str());
    if key_contract != Some(contract_id) {
        return false;
    }
    key.get("key")
        .and_then(|k| {
            let ty = k.get("type")?.as_str()?;
            let val = k.get("value")?.as_str()?;
            Some(ty == "sym" && val == "name")
        })
        .unwrap_or(false)
}

/// Pull the `contract` StrKey from a ContractData ledger key. Used by
/// `extract_contract_data_name_writes` to dispatch storage writes to
/// the right contract row when there is no enclosing deployment.
fn extract_contract_id_from_key(key: &Value) -> Option<String> {
    key.get("contract")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Decode `data.val` as a UTF-8 string for SCVal types that legally
/// represent a name (`string`, `sym`, `bytes`).
///
/// Returns `None` for any other SCVal variant (Vec/Map/Bool/numeric/etc.) —
/// the standard `Symbol("name")` slot is always one of the three string-y
/// shapes in conforming SEP-41 implementations, so a non-matching variant
/// is treated as "no extractable name" rather than a parse failure.
///
/// Robustness rationale: silently returning `None` for unsupported shapes
/// keeps a misbehaving contract from poisoning the parser. The caller's
/// caller (the indexer) treats `None` the same as an absent storage entry.
fn decode_scval_string(data: &Value) -> Option<String> {
    let val = data.get("val")?;
    let ty = val.get("type")?.as_str()?;
    let v = val.get("value")?;
    match ty {
        "string" | "sym" => v.as_str().map(String::from),
        "bytes" => {
            // Stellar XDR JSON intermediate encodes BytesM as hex string.
            let hex_str = v.as_str()?;
            let bytes = hex::decode(hex_str).ok()?;
            String::from_utf8(bytes).ok()
        }
        _ => None,
    }
}

fn is_contract_instance_key(key: &Value) -> bool {
    let key_field = key.get("key");
    match key_field {
        Some(k) => k
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "ledger_key_contract_instance"),
        None => false,
    }
}

fn is_sac_from_data(data: &Value) -> bool {
    data.get("val")
        .and_then(|v| v.get("value"))
        .and_then(|v| v.get("executable"))
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str())
        .is_some_and(|t| t == "stellar_asset")
}

fn extract_wasm_hash(data: &Value) -> Option<String> {
    data.get("val")
        .and_then(|v| v.get("value"))
        .and_then(|v| v.get("executable"))
        .and_then(|v| v.get("hash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Step 2: Account State Extraction
// ---------------------------------------------------------------------------

/// Convert raw stroops (i64) to Stellar-standard decimal string with 7 decimal places.
/// Example: 10_000_000 → "1.0000000", 1234 → "0.0001234"
fn format_stroops(stroops: i64) -> String {
    let whole = stroops / 10_000_000;
    let frac = (stroops % 10_000_000).unsigned_abs();
    format!("{whole}.{frac:07}")
}

/// Extract account states from ledger entry changes.
///
/// Processes both `account` and `trustline` entry types. Account entries provide
/// native XLM balance, sequence number, and home domain. Trustline entries provide
/// non-native asset balances (credit_alphanum4, credit_alphanum12).
///
/// Within a single transaction's changes, entries are merged by `account_id` so that
/// the output contains at most one `ExtractedAccountState` per account.
///
/// Trustline-only changes (no account entry in the same tx) produce an entry with
/// `sequence_number = -1` (sentinel), signalling the SQL layer to preserve the
/// existing value.
pub fn extract_account_states(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedAccountState> {
    use std::collections::HashMap;

    struct AccountAccum {
        native_balance: Option<i64>,
        sequence_number: Option<i64>,
        home_domain: Option<String>,
        is_creation: bool,
        ledger_sequence: u32,
        created_at: i64,
        trustline_balances: Vec<Value>,
        removed_trustlines: Vec<Value>,
    }

    let mut map: HashMap<String, AccountAccum> = HashMap::new();

    // Pass 1: account entries
    for change in changes {
        if change.entry_type != "account" {
            continue;
        }
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored"
        ) {
            continue;
        }
        let Some(ref data) = change.data else {
            continue;
        };

        let account_id = data
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if account_id.is_empty() {
            continue;
        }

        let balance = data.get("balance").and_then(|v| v.as_i64()).unwrap_or(0);
        let seq = data.get("seq_num").and_then(|v| v.as_i64()).unwrap_or(0);
        let hd = data
            .get("home_domain")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let is_creation = matches!(change.change_type.as_str(), "created" | "restored");

        let entry = map.entry(account_id).or_insert_with(|| AccountAccum {
            native_balance: None,
            sequence_number: None,
            home_domain: None,
            is_creation: false,
            ledger_sequence: change.ledger_sequence,
            created_at: change.created_at,
            trustline_balances: Vec::new(),
            removed_trustlines: Vec::new(),
        });
        entry.native_balance = Some(balance);
        entry.sequence_number = Some(seq);
        if hd.is_some() {
            entry.home_domain = hd;
        }
        entry.is_creation = entry.is_creation || is_creation;
        entry.ledger_sequence = change.ledger_sequence;
        entry.created_at = change.created_at;
    }

    // Pass 2: trustline entries
    for change in changes {
        if change.entry_type != "trustline" {
            continue;
        }

        match change.change_type.as_str() {
            "created" | "updated" | "restored" => {
                let Some(ref data) = change.data else {
                    continue;
                };
                let account_id = data
                    .get("account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if account_id.is_empty() {
                    continue;
                }

                let balance = data.get("balance").and_then(|v| v.as_i64()).unwrap_or(0);
                let asset = data.get("asset");

                let trustline_entry = match asset {
                    Some(Value::Object(obj)) => {
                        let asset_type = obj
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        // pool_share trustlines are LP positions, not asset
                        // balances — handled by the sibling producer
                        // `extract_lp_positions` (task 0162). Skipping here
                        // is intentional, not a data drop.
                        if asset_type == "pool_share" {
                            continue;
                        }
                        let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or("");
                        let issuer = obj.get("issuer").and_then(|v| v.as_str()).unwrap_or("");
                        serde_json::json!({
                            "asset_type": asset_type,
                            "asset_code": code,
                            "issuer": issuer,
                            "balance": format_stroops(balance),
                        })
                    }
                    // Native trustlines shouldn't exist; skip
                    _ => continue,
                };

                let entry = map.entry(account_id).or_insert_with(|| AccountAccum {
                    native_balance: None,
                    sequence_number: None,
                    home_domain: None,
                    is_creation: false,
                    ledger_sequence: change.ledger_sequence,
                    created_at: change.created_at,
                    trustline_balances: Vec::new(),
                    removed_trustlines: Vec::new(),
                });

                // Dedup: remove existing entry for same asset, then add new
                let new_code = trustline_entry.get("asset_code").cloned();
                let new_issuer = trustline_entry.get("issuer").cloned();
                entry.trustline_balances.retain(|tb| {
                    tb.get("asset_code") != new_code.as_ref()
                        || tb.get("issuer") != new_issuer.as_ref()
                });
                // Cancel any prior removal for the same asset (remove-then-recreate in same tx)
                entry.removed_trustlines.retain(|rt| {
                    rt.get("asset_code") != new_code.as_ref()
                        || rt.get("issuer") != new_issuer.as_ref()
                });
                entry.trustline_balances.push(trustline_entry);

                if change.ledger_sequence >= entry.ledger_sequence {
                    entry.ledger_sequence = change.ledger_sequence;
                    entry.created_at = change.created_at;
                }
            }
            "removed" => {
                // Trustline removed — extract account_id and asset from the key
                let account_id = change
                    .key
                    .get("account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if account_id.is_empty() {
                    continue;
                }

                let asset = change.key.get("asset");
                let removal_key = match asset {
                    Some(Value::Object(obj)) => {
                        let asset_type = obj
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        // pool_share removal is handled by `extract_lp_positions`
                        // (task 0162) which emits a zero-shares row from the
                        // change.key; skipping here keeps account-state focus.
                        if asset_type == "pool_share" {
                            continue;
                        }
                        let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or("");
                        let issuer = obj.get("issuer").and_then(|v| v.as_str()).unwrap_or("");
                        serde_json::json!({
                            "asset_type": asset_type,
                            "asset_code": code,
                            "issuer": issuer,
                        })
                    }
                    _ => continue,
                };

                let entry = map.entry(account_id).or_insert_with(|| AccountAccum {
                    native_balance: None,
                    sequence_number: None,
                    home_domain: None,
                    is_creation: false,
                    ledger_sequence: change.ledger_sequence,
                    created_at: change.created_at,
                    trustline_balances: Vec::new(),
                    removed_trustlines: Vec::new(),
                });

                // Also remove from trustline_balances if it was added in same tx
                let rm_code = removal_key.get("asset_code");
                let rm_issuer = removal_key.get("issuer");
                entry
                    .trustline_balances
                    .retain(|tb| tb.get("asset_code") != rm_code || tb.get("issuer") != rm_issuer);
                entry.removed_trustlines.push(removal_key);

                if change.ledger_sequence >= entry.ledger_sequence {
                    entry.ledger_sequence = change.ledger_sequence;
                    entry.created_at = change.created_at;
                }
            }
            _ => continue,
        }
    }

    // Build results
    map.into_iter()
        .map(|(account_id, accum)| {
            let mut balances_arr: Vec<Value> = Vec::new();
            if let Some(native) = accum.native_balance {
                balances_arr.push(
                    serde_json::json!({"asset_type": "native", "balance": format_stroops(native)}),
                );
            }
            balances_arr.extend(accum.trustline_balances);

            ExtractedAccountState {
                account_id,
                first_seen_ledger: if accum.is_creation {
                    Some(accum.ledger_sequence)
                } else {
                    None
                },
                last_seen_ledger: accum.ledger_sequence,
                sequence_number: accum.sequence_number.unwrap_or(-1),
                balances: Value::Array(balances_arr),
                removed_trustlines: accum.removed_trustlines,
                home_domain: accum.home_domain,
                created_at: accum.created_at,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Step 3 + Step 4: Liquidity Pool State + Snapshots
// ---------------------------------------------------------------------------

/// Extract liquidity pool states and snapshots from ledger entry changes.
///
/// Returns pool state updates and a snapshot for each change.
pub fn extract_liquidity_pools(
    changes: &[ExtractedLedgerEntryChange],
) -> (
    Vec<ExtractedLiquidityPool>,
    Vec<ExtractedLiquidityPoolSnapshot>,
) {
    let mut pools = Vec::new();
    let mut snapshots = Vec::new();

    for change in changes {
        if change.entry_type != "liquidity_pool" {
            continue;
        }
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored"
        ) {
            continue;
        }
        let Some(ref data) = change.data else {
            continue;
        };

        let pool_id = data
            .get("pool_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if pool_id.is_empty() {
            continue;
        }

        let params = data.get("params").cloned().unwrap_or(serde_json::json!({}));
        let asset_a = params
            .get("asset_a")
            .cloned()
            .unwrap_or(serde_json::json!(null));
        let asset_b = params
            .get("asset_b")
            .cloned()
            .unwrap_or(serde_json::json!(null));
        let fee_bps = params.get("fee").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        let reserve_a = data.get("reserve_a").and_then(|v| v.as_i64()).unwrap_or(0);
        let reserve_b = data.get("reserve_b").and_then(|v| v.as_i64()).unwrap_or(0);
        let reserves = serde_json::json!({ "a": reserve_a, "b": reserve_b });

        let total_shares = data
            .get("total_pool_shares")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .to_string();

        let is_creation = matches!(change.change_type.as_str(), "created" | "restored");
        let pool = ExtractedLiquidityPool {
            pool_id: pool_id.clone(),
            asset_a: asset_a.clone(),
            asset_b: asset_b.clone(),
            fee_bps,
            reserves: reserves.clone(),
            total_shares: total_shares.clone(),
            tvl: None,
            created_at_ledger: if is_creation {
                Some(change.ledger_sequence)
            } else {
                None
            },
            last_updated_ledger: change.ledger_sequence,
            created_at: change.created_at,
        };

        let snapshot = ExtractedLiquidityPoolSnapshot {
            pool_id,
            ledger_sequence: change.ledger_sequence,
            created_at: change.created_at,
            reserves,
            total_shares,
            tvl: None,
            volume: None,
            fee_revenue: None,
        };

        pools.push(pool);
        snapshots.push(snapshot);
    }

    (pools, snapshots)
}

// ---------------------------------------------------------------------------
// Step 4b: Liquidity-pool participant positions (task 0162)
// ---------------------------------------------------------------------------

/// Extract LP participant positions from `pool_share` trustline changes.
///
/// `extract_account_states` skips `pool_share` trustlines on purpose —
/// they are not classic asset balances and do not belong in the per-account
/// trustline_balances JSON. They DO encode `(account, pool_id, share balance)`
/// triples that the `lp_positions` table is shaped for, so this sibling fn
/// produces them as `ExtractedLpPosition` records on the same `changes`
/// slice. Two passes over `changes` is intentional: keeps each producer fn
/// single-purpose and matches the existing one-fn-per-output-type idiom in
/// this module.
///
/// Change-type semantics:
///
/// - `created` → emit with `first_deposit_ledger = Some(ledger_sequence)`;
///   staging layer COALESCEs to keep the original on subsequent updates.
/// - `updated` / `restored` → emit with `first_deposit_ledger = None`.
/// - `removed` → emit with `shares = "0.0000000"` and
///   `first_deposit_ledger = None`. Persist layer (task 0126) decides
///   whether zero-share rows are pruned or kept as historical
///   participant records — this fn just reports the data.
///
/// `state` change_type is observation-only (no balance change) and is
/// skipped here, matching the trustline path in `extract_account_states`.
pub fn extract_lp_positions(changes: &[ExtractedLedgerEntryChange]) -> Vec<ExtractedLpPosition> {
    let mut positions = Vec::new();

    for change in changes {
        if change.entry_type != "trustline" {
            continue;
        }

        let (asset_holder, account_id, shares, first_deposit) = match change.change_type.as_str() {
            "created" | "updated" | "restored" => {
                let Some(ref data) = change.data else {
                    continue;
                };
                let Some(account_id) = data.get("account_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(asset) = data.get("asset") else {
                    continue;
                };
                let balance = data.get("balance").and_then(|v| v.as_i64()).unwrap_or(0);
                let first_deposit = if change.change_type == "created" {
                    Some(change.ledger_sequence)
                } else {
                    None
                };
                (
                    asset.clone(),
                    account_id.to_string(),
                    format_stroops(balance),
                    first_deposit,
                )
            }
            "removed" => {
                let Some(account_id) = change.key.get("account_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(asset) = change.key.get("asset") else {
                    continue;
                };
                (
                    asset.clone(),
                    account_id.to_string(),
                    format_stroops(0),
                    None,
                )
            }
            _ => continue,
        };

        let Some(asset_obj) = asset_holder.as_object() else {
            continue;
        };
        if asset_obj.get("type").and_then(|v| v.as_str()) != Some("pool_share") {
            continue;
        }
        let Some(pool_id) = asset_obj.get("pool_id").and_then(|v| v.as_str()) else {
            continue;
        };

        positions.push(ExtractedLpPosition {
            pool_id: pool_id.to_string(),
            account_id,
            shares,
            first_deposit_ledger: first_deposit,
            last_updated_ledger: change.ledger_sequence,
        });
    }

    positions
}

// ---------------------------------------------------------------------------
// Step 5: Asset Detection
// ---------------------------------------------------------------------------

/// Detect assets from contract deployments.
///
/// Two paths produce an [`ExtractedAsset`]:
///
/// 1. **SAC deployments** — [`TokenAssetType::Sac`] row. Identity comes
///    from `deployment.sac_asset` (resolved from
///    `ContractIdPreimage::FromAsset` via `crate::sac::extract_sac_identities`
///    in the indexer). Two shapes:
///    - `Credit { code, issuer }` → row keyed by code+issuer+contract_id
///      (`uidx_assets_classic_asset` partial unique).
///    - `Native` → row keyed by contract_id only (NULL code/issuer);
///      `ck_assets_identity` permits this for `asset_type=2` after the
///      0160 schema loosening migration. Aligns with Horizon/SDK
///      rendering of native asset.
///    - `None` (SAC deployment whose creating preimage is not in this
///      batch) is logged as a warn and skipped — better to lose one row
///      than fabricate identity.
/// 2. **WASM-based deployments classifying as
///    [`ContractClassification::Fungible`]** — [`TokenAssetType::Soroban`]
///    row; identity is `contract_id` only. Classification uses
///    [`classify_contract_from_wasm_spec`] against the deployment's WASM
///    interface function list.
///
/// NFT-classified contracts (SEP-0050 surface: `owner_of`, `token_uri`, …)
/// do **not** produce an assets row — they live in the `nfts` table via
/// the NFT pipeline (task 0118). `Other`-classified contracts also produce
/// no row: a later WASM upload may promote them, in which case the
/// reclassification write step backfills the missing assets row
/// (`write::insert_assets_from_reclassified_contracts`, task 0120).
///
/// `name` / `total_supply` / `holder_count` are left `None` for Soroban
/// rows: on-chain name/symbol extraction from ContractData storage entries
/// is tracked as follow-up task 0156; `holder_count` is task 0135; a
/// separate scheduled-Lambda enrichment path for SEP-1 metadata lives
/// under task 0124.
pub fn detect_assets(
    deployments: &[ExtractedContractDeployment],
    interfaces: &[ExtractedContractInterface],
) -> Vec<ExtractedAsset> {
    // Pre-index interfaces by wasm_hash so the inner loop is O(1) per
    // deployment. Classification itself is O(|functions|) but amortised
    // across all deployments sharing that wasm_hash (shared-library
    // contracts are common on Stellar), so cache the verdict too.
    use std::collections::HashMap;
    let mut verdict_by_hash: HashMap<&str, ContractClassification> =
        HashMap::with_capacity(interfaces.len());
    for iface in interfaces {
        verdict_by_hash
            .entry(iface.wasm_hash.as_str())
            .or_insert_with(|| classify_contract_from_wasm_spec(&iface.functions));
    }

    let mut assets = Vec::new();
    for deployment in deployments {
        if deployment.is_sac {
            // Task 0160: populate classic asset identity for the SAC row
            // straight from the typed enum produced by the parser.
            //   Native             → NULL code, NULL issuer (schema-loosened
            //                        for asset_type=2; row keyed by contract_id).
            //   Credit{code,issuer}→ real code + issuer (classic-keyed row).
            //   None               → preimage not in this batch; skip with
            //                        a warn rather than fabricate identity.
            let (asset_code, issuer_address) = match &deployment.sac_asset {
                Some(SacAssetIdentity::Native) => (None, None),
                Some(SacAssetIdentity::Credit { code, issuer }) => {
                    (Some(code.clone()), Some(issuer.clone()))
                }
                None => {
                    warn!(
                        contract_id = %deployment.contract_id,
                        "SAC deployment without resolved asset identity; skipping assets row"
                    );
                    continue;
                }
            };
            assets.push(ExtractedAsset {
                asset_type: TokenAssetType::Sac,
                asset_code,
                issuer_address,
                contract_id: Some(deployment.contract_id.clone()),
                // SAC assets do not carry an on-chain `Symbol("name")` storage
                // entry (they wrap a classic asset; name is derived from
                // `asset_code` / SEP-1 metadata). Leave NULL for SAC rows.
                name: None,
                total_supply: None,
                holder_count: None,
            });
            continue;
        }

        // Non-SAC: classify by WASM spec. Requires a matching interface in
        // this batch — if absent (e.g. contract deployed in a prior ledger
        // whose WASM only arrives later), skip here; the late-WASM bridge
        // in the persist layer picks it up once classification succeeds.
        let Some(wasm_hash) = deployment.wasm_hash.as_deref() else {
            continue;
        };
        if verdict_by_hash.get(wasm_hash) == Some(&ContractClassification::Fungible) {
            assets.push(ExtractedAsset {
                asset_type: TokenAssetType::Soroban,
                asset_code: None,
                issuer_address: None,
                contract_id: Some(deployment.contract_id.clone()),
                // Per ADR 0041 / task 0156: thread the on-chain
                // `Symbol("name")` extracted at deploy time into the
                // asset row. Late-init / re-init paths land via the
                // indexer's `apply_contract_name_writes` helper; that
                // path also covers `assets.name` follow-up updates.
                name: deployment.name.clone(),
                total_supply: None,
                holder_count: None,
            });
        }
    }

    assets
}

// ---------------------------------------------------------------------------
// Step 6: NFT Detection
// ---------------------------------------------------------------------------

/// Detect NFTs from NFT events (produced by task 0026's `detect_nft_events`).
///
/// Converts `NftEvent` records into `ExtractedNft` entities for DB persistence.
pub fn detect_nfts(nft_events: &[NftEvent]) -> Vec<ExtractedNft> {
    let mut nfts = Vec::new();

    for event in nft_events {
        let token_id = token_id_to_string(&event.token_id);
        if token_id.is_empty() {
            continue;
        }

        let (owner_account, minted_at_ledger) = match event.event_kind.as_str() {
            "mint" => (event.to.clone(), Some(event.ledger_sequence)),
            "transfer" => (event.to.clone(), None),
            "burn" => (None, None),
            _ => continue,
        };

        nfts.push(ExtractedNft {
            contract_id: event.contract_id.clone(),
            token_id,
            collection_name: None,
            owner_account,
            name: None,
            media_url: None,
            metadata: None,
            minted_at_ledger,
            last_seen_ledger: event.ledger_sequence,
            created_at: event.created_at,
        });
    }

    nfts
}

/// Convert an NftEvent token_id JSON value to a string key for the DB.
fn token_id_to_string(token_id: &Value) -> String {
    if let Some(v) = token_id.get("value") {
        if v.is_null() {
            return String::new();
        }
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
        if let Some(n) = v.as_i64() {
            return n.to_string();
        }
        if let Some(n) = v.as_u64() {
            return n.to_string();
        }
        return v.to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_change(
        entry_type: &str,
        change_type: &str,
        key: Value,
        data: Option<Value>,
    ) -> ExtractedLedgerEntryChange {
        ExtractedLedgerEntryChange {
            transaction_hash: "abc123".into(),
            change_type: change_type.into(),
            entry_type: entry_type.into(),
            key,
            data,
            change_index: 0,
            operation_index: None,
            ledger_sequence: 100,
            created_at: 1700000000,
        }
    }

    // -- Contract Deployment Tests --

    #[test]
    fn extract_wasm_contract_deployment() {
        let changes = vec![make_change(
            "contract_data",
            "created",
            json!({
                "contract": "CABC123",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": "CABC123",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
                "val": { "type": "contract_instance", "value": {
                    "executable": { "type": "wasm", "hash": "aa".repeat(32) }
                }},
            })),
        )];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert_eq!(deployments.len(), 1);
        assert_eq!(deployments[0].contract_id, "CABC123");
        assert_eq!(
            deployments[0].deployer_account.as_deref(),
            Some("GDEPLOYER")
        );
        assert_eq!(deployments[0].wasm_hash, Some("aa".repeat(32)));
        assert!(!deployments[0].is_sac);
        assert_eq!(deployments[0].contract_type, ContractType::Other);
    }

    #[test]
    fn extract_sac_deployment() {
        let changes = vec![make_change(
            "contract_data",
            "created",
            json!({
                "contract": "CSAC456",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": "CSAC456",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
                "val": { "type": "contract_instance", "value": {
                    "executable": { "type": "stellar_asset" }
                }},
            })),
        )];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert_eq!(deployments.len(), 1);
        assert!(deployments[0].is_sac);
        assert_eq!(deployments[0].contract_type, ContractType::Token);
        assert!(deployments[0].wasm_hash.is_none());
    }

    #[test]
    fn skip_non_instance_contract_data() {
        let changes = vec![make_change(
            "contract_data",
            "created",
            json!({
                "contract": "CABC123",
                "key": { "type": "sym", "value": "counter" },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": "CABC123",
                "key": { "type": "sym", "value": "counter" },
                "durability": "persistent",
                "val": { "type": "u64", "value": 42 },
            })),
        )];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert!(deployments.is_empty());
    }

    #[test]
    fn skip_updated_contract_instance() {
        let changes = vec![make_change(
            "contract_data",
            "updated",
            json!({
                "contract": "CABC123",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": "CABC123",
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
                "val": { "type": "contract_instance", "value": {
                    "executable": { "type": "wasm", "hash": "bb".repeat(32) }
                }},
            })),
        )];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert!(deployments.is_empty());
    }

    // -- Symbol("name") extraction tests (ADR 0041 / task 0156) --

    /// Helper — build a `contract_data` change carrying a `Symbol("name")`
    /// key and a string SCVal value. Mirrors the JSON intermediate shape
    /// that `ledger_entry_changes::extract_ledger_entry_changes`
    /// produces for the standard token-name storage entry.
    fn make_name_change(
        contract_id: &str,
        change_type: &str,
        scval_type: &str,
        scval_value: &serde_json::Value,
    ) -> ExtractedLedgerEntryChange {
        make_change(
            "contract_data",
            change_type,
            json!({
                "contract": contract_id,
                "key":      { "type": "sym", "value": "name" },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": contract_id,
                "key":      { "type": "sym", "value": "name" },
                "durability": "persistent",
                "val":      { "type": scval_type, "value": scval_value },
            })),
        )
    }

    fn make_wasm_deploy_change(contract_id: &str) -> ExtractedLedgerEntryChange {
        make_change(
            "contract_data",
            "created",
            json!({
                "contract": contract_id,
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": contract_id,
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
                "val": { "type": "contract_instance", "value": {
                    "executable": { "type": "wasm", "hash": "aa".repeat(32) }
                }},
            })),
        )
    }

    /// Constructor pattern — deploy and `Symbol("name")` storage init in
    /// the same ledger. The deployment second pass should populate
    /// `deployment.name` with the decoded String.
    #[test]
    fn extract_constructor_pattern_with_name() {
        let changes = vec![
            make_wasm_deploy_change("CABC123"),
            make_name_change("CABC123", "created", "string", &json!("USD Coin")),
        ];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert_eq!(deployments.len(), 1);
        assert_eq!(deployments[0].contract_id, "CABC123");
        assert_eq!(deployments[0].name.as_deref(), Some("USD Coin"));
    }

    /// Constructor-pattern deploy without an accompanying `Symbol("name")`
    /// entry — `deployment.name` should stay `None`. Frontend renders the
    /// contract id then; the late-init pass may fill the column later.
    #[test]
    fn extract_constructor_pattern_without_name() {
        let changes = vec![make_wasm_deploy_change("CABC123")];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert_eq!(deployments.len(), 1);
        assert!(deployments[0].name.is_none());
    }

    /// Cross-contract isolation — a `Symbol("name")` entry on a different
    /// contract id must not leak into the deployment.
    #[test]
    fn extract_constructor_pattern_skips_other_contract_name() {
        let changes = vec![
            make_wasm_deploy_change("CABC123"),
            // Name belongs to a DIFFERENT contract; should be ignored.
            make_name_change("COTHER", "created", "string", &json!("Other Token")),
        ];

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new());
        assert_eq!(deployments.len(), 1);
        assert!(deployments[0].name.is_none());
    }

    /// Late-init pattern — contract was deployed in a prior ledger; only
    /// the `Symbol("name")` storage entry lands in the current ledger.
    /// `extract_contract_data_name_writes` returns the `(contract_id, name)`
    /// pair so the indexer can apply a retroactive UPDATE.
    #[test]
    fn name_writes_late_init_created() {
        let changes = vec![make_name_change(
            "CABC123",
            "created",
            "string",
            &json!("USD Coin"),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert_eq!(
            writes,
            vec![("CABC123".to_string(), "USD Coin".to_string())]
        );
    }

    /// Re-init pattern — an existing `Symbol("name")` storage entry is
    /// updated post-deploy. `change_type == "updated"` must also be
    /// captured so the indexer can overwrite the previous value.
    #[test]
    fn name_writes_re_init_updated() {
        let changes = vec![make_name_change(
            "CABC123",
            "updated",
            "string",
            &json!("Renamed Token"),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert_eq!(
            writes,
            vec![("CABC123".to_string(), "Renamed Token".to_string())]
        );
    }

    /// `Symbol("name")` written via Symbol SCVal (uncommon but legal —
    /// some SDK builders emit short tokens this way). Helper must accept it.
    #[test]
    fn name_writes_decodes_symbol_scval() {
        let changes = vec![make_name_change(
            "CABC123",
            "created",
            "sym",
            &json!("USDC"),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert_eq!(writes, vec![("CABC123".to_string(), "USDC".to_string())]);
    }

    /// `Symbol("name")` written via Bytes SCVal (UTF-8 encoded). Stellar
    /// XDR JSON intermediate uses lowercase hex for binary payloads.
    /// "USDC" → 55534443 hex.
    #[test]
    fn name_writes_decodes_bytes_scval_utf8() {
        // "USDC" as UTF-8 bytes → hex.
        let changes = vec![make_name_change(
            "CABC123",
            "created",
            "bytes",
            &json!("55534443"),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert_eq!(writes, vec![("CABC123".to_string(), "USDC".to_string())]);
    }

    /// Non-string SCVal variants (numeric, bool, vec, map) return `None`
    /// without panicking. Misbehaving contracts should not poison the
    /// parser.
    #[test]
    fn name_writes_rejects_non_string_scval_variants() {
        let cases = vec![
            ("u64", json!(42)),
            ("bool", json!(true)),
            ("vec", json!([1, 2, 3])),
            ("map", json!({"a": 1})),
        ];

        for (ty, value) in cases {
            let changes = vec![make_name_change("CABC123", "created", ty, &value)];
            let writes = extract_contract_data_name_writes(&changes);
            assert!(
                writes.is_empty(),
                "expected no writes for SCVal type {ty}, got {writes:?}"
            );
        }
    }

    /// `Symbol("name")` keys on `change_type = "deleted"` are NOT captured.
    /// Deletion semantics for the name column are out of scope for 0156;
    /// the existing value stays. (If a contract ever needs name removal,
    /// a follow-up task can extend the helper.)
    #[test]
    fn name_writes_skips_deleted_changes() {
        let changes = vec![make_name_change(
            "CABC123",
            "deleted",
            "string",
            &json!("USD Coin"),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert!(writes.is_empty());
    }

    /// Other Symbol(...) keys (decimals, symbol, etc.) on a contract are
    /// not extracted by `extract_contract_data_name_writes` — the helper
    /// is name-specific. A follow-up task adding `decimals` (etc.) would
    /// introduce its own helper.
    #[test]
    fn name_writes_skips_non_name_symbol_keys() {
        let changes = vec![make_change(
            "contract_data",
            "created",
            json!({
                "contract":  "CABC123",
                "key":       { "type": "sym", "value": "decimals" },
                "durability": "persistent",
            }),
            Some(json!({
                "contract":  "CABC123",
                "key":       { "type": "sym", "value": "decimals" },
                "durability": "persistent",
                "val":       { "type": "u32", "value": 7 },
            })),
        )];

        let writes = extract_contract_data_name_writes(&changes);
        assert!(writes.is_empty());
    }

    // -- Account State Tests --

    #[test]
    fn extract_created_account_state() {
        let changes = vec![make_change(
            "account",
            "created",
            json!({ "account_id": "GABC123" }),
            Some(json!({
                "account_id": "GABC123",
                "balance": 1000000,
                "seq_num": 1,
                "home_domain": "",
                "num_sub_entries": 0,
                "thresholds": "01000000",
                "flags": 0,
            })),
        )];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].account_id, "GABC123");
        assert_eq!(accounts[0].sequence_number, 1);
        assert!(accounts[0].first_seen_ledger.is_some());
        assert!(accounts[0].home_domain.is_none()); // empty string filtered
    }

    #[test]
    fn extract_updated_account_with_home_domain() {
        let changes = vec![make_change(
            "account",
            "updated",
            json!({ "account_id": "GABC123" }),
            Some(json!({
                "account_id": "GABC123",
                "balance": 5000000,
                "seq_num": 42,
                "home_domain": "example.com",
                "num_sub_entries": 2,
                "thresholds": "01000000",
                "flags": 0,
            })),
        )];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        assert!(accounts[0].first_seen_ledger.is_none());
        assert_eq!(accounts[0].home_domain.as_deref(), Some("example.com"));
        assert_eq!(accounts[0].sequence_number, 42);
    }

    #[test]
    fn skip_state_and_removed_accounts() {
        let changes = vec![
            make_change(
                "account",
                "state",
                json!({}),
                Some(json!({"account_id": "G1", "balance": 0, "seq_num": 0})),
            ),
            make_change("account", "removed", json!({}), None),
        ];

        let accounts = extract_account_states(&changes);
        assert!(accounts.is_empty());
    }

    // -- Trustline Balance Tests (0119) --

    #[test]
    fn account_with_two_trustlines() {
        let changes = vec![
            make_change(
                "account",
                "created",
                json!({ "account_id": "GABC" }),
                Some(json!({
                    "account_id": "GABC",
                    "balance": 1000000,
                    "seq_num": 1,
                    "home_domain": "",
                    "num_sub_entries": 2,
                    "thresholds": "01000000",
                    "flags": 0,
                })),
            ),
            make_change(
                "trustline",
                "created",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER1" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER1" },
                    "balance": 5000,
                    "limit": 10000,
                    "flags": 1,
                })),
            ),
            make_change(
                "trustline",
                "created",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum12", "code": "EUROC", "issuer": "GISSUER2" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum12", "code": "EUROC", "issuer": "GISSUER2" },
                    "balance": 3000,
                    "limit": 50000,
                    "flags": 1,
                })),
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let a = &accounts[0];
        assert_eq!(a.account_id, "GABC");
        assert_eq!(a.sequence_number, 1);
        assert!(a.first_seen_ledger.is_some());
        let balances = a.balances.as_array().unwrap();
        assert_eq!(balances.len(), 3);
        assert!(
            balances
                .iter()
                .any(|b| b["asset_type"] == "native" && b["balance"] == "0.1000000")
        );
        assert!(
            balances
                .iter()
                .any(|b| b["asset_code"] == "USDC" && b["balance"] == "0.0005000")
        );
        assert!(
            balances
                .iter()
                .any(|b| b["asset_code"] == "EUROC" && b["balance"] == "0.0003000")
        );
        assert!(a.removed_trustlines.is_empty());
    }

    #[test]
    fn trustline_only_change() {
        let changes = vec![make_change(
            "trustline",
            "updated",
            json!({
                "account_id": "GABC",
                "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER1" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER1" },
                "balance": 9999,
                "limit": 10000,
                "flags": 1,
            })),
        )];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let a = &accounts[0];
        assert_eq!(a.sequence_number, -1); // sentinel
        let balances = a.balances.as_array().unwrap();
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0]["asset_code"], "USDC");
        assert_eq!(balances[0]["balance"], "0.0009999");
    }

    #[test]
    fn trustline_removal() {
        let changes = vec![
            make_change(
                "account",
                "updated",
                json!({ "account_id": "GABC" }),
                Some(json!({
                    "account_id": "GABC",
                    "balance": 500,
                    "seq_num": 10,
                    "home_domain": "",
                    "num_sub_entries": 0,
                    "thresholds": "01000000",
                    "flags": 0,
                })),
            ),
            make_change(
                "trustline",
                "removed",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER1" },
                }),
                None,
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let a = &accounts[0];
        assert_eq!(a.sequence_number, 10);
        let balances = a.balances.as_array().unwrap();
        assert_eq!(balances.len(), 1); // only native remains
        assert_eq!(balances[0]["asset_type"], "native");
        assert_eq!(a.removed_trustlines.len(), 1);
        assert_eq!(a.removed_trustlines[0]["asset_code"], "USDC");
    }

    #[test]
    fn trustline_update_dedup() {
        let changes = vec![
            make_change(
                "trustline",
                "updated",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                    "balance": 100,
                    "limit": 10000,
                    "flags": 1,
                })),
            ),
            make_change(
                "trustline",
                "updated",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                    "balance": 200,
                    "limit": 10000,
                    "flags": 1,
                })),
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let balances = accounts[0].balances.as_array().unwrap();
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0]["balance"], "0.0000200"); // last wins
    }

    #[test]
    fn pool_share_trustline_skipped() {
        let changes = vec![make_change(
            "trustline",
            "created",
            json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
                "balance": 1000,
                "limit": 99999,
                "flags": 0,
            })),
        )];

        let accounts = extract_account_states(&changes);
        assert!(accounts.is_empty());
    }

    // -- Task 0162: extract_lp_positions --

    #[test]
    fn lp_position_extracted_from_created_pool_share_trustline() {
        let changes = vec![make_change(
            "trustline",
            "created",
            json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
                "balance": 420_000_000_i64,  // 42 shares in stroops
                "limit": 99_999_999_999_i64,
                "flags": 0,
            })),
        )];

        let positions = extract_lp_positions(&changes);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].pool_id, "aabb");
        assert_eq!(positions[0].account_id, "GABC");
        assert_eq!(positions[0].shares, "42.0000000");
        assert_eq!(positions[0].first_deposit_ledger, Some(100));
        assert_eq!(positions[0].last_updated_ledger, 100);
    }

    #[test]
    fn lp_position_updated_drops_first_deposit_ledger() {
        let changes = vec![make_change(
            "trustline",
            "updated",
            json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
                "balance": 50_000_000_i64,
                "limit": 99_999_999_999_i64,
                "flags": 0,
            })),
        )];

        let positions = extract_lp_positions(&changes);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].shares, "5.0000000");
        // updated → preserve original first_deposit_ledger via NULL +
        // staging COALESCE, not overwrite from this change.
        assert!(positions[0].first_deposit_ledger.is_none());
    }

    #[test]
    fn lp_position_removed_emits_zero_shares_from_key() {
        // `removed` change has no `data`; account_id + asset come from `key`.
        let changes = vec![make_change(
            "trustline",
            "removed",
            json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
            }),
            None,
        )];

        let positions = extract_lp_positions(&changes);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].shares, "0.0000000");
        assert!(positions[0].first_deposit_ledger.is_none());
        assert_eq!(positions[0].last_updated_ledger, 100);
    }

    #[test]
    fn lp_positions_ignore_credit_trustlines() {
        // Regular credit trustline must not produce an LP position;
        // account-state path handles it instead.
        let changes = vec![make_change(
            "trustline",
            "created",
            json!({
                "account_id": "GABC",
                "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GISSUER" },
                "balance": 5_000_000_i64,
                "limit": 99_999_999_999_i64,
                "flags": 0,
            })),
        )];

        assert!(extract_lp_positions(&changes).is_empty());
        // The same change does still contribute to account state.
        assert_eq!(extract_account_states(&changes).len(), 1);
    }

    #[test]
    fn lp_positions_ignore_state_change_type() {
        // `state` is observation-only (no balance change) — do not emit.
        let changes = vec![make_change(
            "trustline",
            "state",
            json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
            }),
            Some(json!({
                "account_id": "GABC",
                "asset": { "type": "pool_share", "pool_id": "aabb" },
                "balance": 100_000_000_i64,
                "limit": 99_999_999_999_i64,
                "flags": 0,
            })),
        )];

        assert!(extract_lp_positions(&changes).is_empty());
    }

    #[test]
    fn removal_cancels_same_tx_creation() {
        let changes = vec![
            make_change(
                "trustline",
                "created",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                    "balance": 500,
                    "limit": 10000,
                    "flags": 1,
                })),
            ),
            make_change(
                "trustline",
                "removed",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                None,
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let balances = accounts[0].balances.as_array().unwrap();
        assert!(balances.is_empty()); // creation was cancelled by removal
        assert_eq!(accounts[0].removed_trustlines.len(), 1);
    }

    #[test]
    fn recreate_cancels_prior_removal_same_tx() {
        let changes = vec![
            // First: trustline removed
            make_change(
                "trustline",
                "removed",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                None,
            ),
            // Then: trustline re-created
            make_change(
                "trustline",
                "created",
                json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                }),
                Some(json!({
                    "account_id": "GABC",
                    "asset": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G1" },
                    "balance": 700,
                    "limit": 10000,
                    "flags": 1,
                })),
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let balances = accounts[0].balances.as_array().unwrap();
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0]["asset_code"], "USDC");
        assert_eq!(balances[0]["balance"], "0.0000700");
        // Removal should be cancelled — trustline was re-created
        assert!(accounts[0].removed_trustlines.is_empty());
    }

    // -- Liquidity Pool Tests --

    #[test]
    fn extract_pool_produces_state_and_snapshot() {
        let changes = vec![make_change(
            "liquidity_pool",
            "created",
            json!({ "pool_id": "aabb" }),
            Some(json!({
                "pool_id": "aabb",
                "type": "constant_product",
                "params": {
                    "asset_a": "native",
                    "asset_b": { "type": "credit_alphanum4", "code": "USDC", "issuer": "G..." },
                    "fee": 30,
                },
                "reserve_a": 10000,
                "reserve_b": 20000,
                "total_pool_shares": 5000,
                "pool_shares_trust_line_count": 3,
            })),
        )];

        let (pools, snapshots) = extract_liquidity_pools(&changes);
        assert_eq!(pools.len(), 1);
        assert_eq!(snapshots.len(), 1);

        assert_eq!(pools[0].pool_id, "aabb");
        assert_eq!(pools[0].fee_bps, 30);
        assert!(pools[0].created_at_ledger.is_some());
        assert_eq!(pools[0].total_shares, "5000");

        assert_eq!(snapshots[0].pool_id, "aabb");
        assert_eq!(snapshots[0].reserves["a"], 10000);
        assert_eq!(snapshots[0].reserves["b"], 20000);
    }

    // -- Asset Detection Tests --

    use crate::types::ContractFunction;

    fn iface(wasm_hash: &str, fn_names: &[&str]) -> ExtractedContractInterface {
        ExtractedContractInterface {
            wasm_hash: wasm_hash.to_string(),
            functions: fn_names
                .iter()
                .map(|n| ContractFunction {
                    name: (*n).to_string(),
                    doc: String::new(),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                })
                .collect(),
            wasm_byte_len: 0,
        }
    }

    #[test]
    fn sac_credit_deployment_produces_asset_with_real_identity() {
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CSAC456".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            name: None,
            sac_asset: Some(SacAssetIdentity::Credit {
                code: "USDC".into(),
                issuer: "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".into(),
            }),
        }];

        let assets = detect_assets(&deployments, &[]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, TokenAssetType::Sac);
        assert_eq!(assets[0].contract_id.as_deref(), Some("CSAC456"));
        // Task 0160 regression: SAC identity must survive through to the asset row.
        assert_eq!(assets[0].asset_code.as_deref(), Some("USDC"));
        assert_eq!(
            assets[0].issuer_address.as_deref(),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
        );
    }

    #[test]
    fn sac_native_deployment_produces_asset_with_null_code_and_issuer() {
        // SAC deploy wrapping native XLM — typed `Native` variant flows
        // through to the assets row as NULL code / NULL issuer
        // (allowed by ck_assets_identity after the 0160 migration).
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CXLM_SAC".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            name: None,
            sac_asset: Some(SacAssetIdentity::Native),
        }];

        let assets = detect_assets(&deployments, &[]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, TokenAssetType::Sac);
        assert_eq!(assets[0].contract_id.as_deref(), Some("CXLM_SAC"));
        assert!(assets[0].asset_code.is_none());
        assert!(assets[0].issuer_address.is_none());
    }

    #[test]
    fn sac_deployment_without_identity_is_skipped() {
        // SAC deployment whose creating preimage isn't in the current
        // batch (replay from mid-ledger). No asset row produced —
        // better to lose one row than fabricate identity.
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CORPHAN".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            name: None,
            sac_asset: None,
        }];

        let assets = detect_assets(&deployments, &[]);
        assert!(assets.is_empty());
    }

    #[test]
    fn non_sac_without_interface_is_skipped() {
        // No matching interface in this batch → skip; late-WASM bridge
        // in persist layer handles reclassification/backfill.
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CABC123".into(),
            wasm_hash: Some("aa".repeat(32)),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Other,
            is_sac: false,
            name: None,
            sac_asset: None,
        }];

        let assets = detect_assets(&deployments, &[]);
        assert!(assets.is_empty());
    }

    #[test]
    fn fungible_wasm_deployment_produces_soroban_asset() {
        // SEP-0041 surface → ContractClassification::Fungible → Soroban asset row.
        let wasm = "aa".repeat(32);
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CFUN001".into(),
            wasm_hash: Some(wasm.clone()),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Fungible,
            is_sac: false,
            name: None,
            sac_asset: None,
        }];
        let interfaces = vec![iface(
            &wasm,
            &["transfer", "balance", "decimals", "name", "symbol"],
        )];

        let assets = detect_assets(&deployments, &interfaces);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, TokenAssetType::Soroban);
        assert_eq!(assets[0].contract_id.as_deref(), Some("CFUN001"));
        assert!(assets[0].asset_code.is_none());
        assert!(assets[0].issuer_address.is_none());
        assert!(assets[0].name.is_none()); // deferred to 0124 enrichment
    }

    #[test]
    fn nft_wasm_deployment_produces_no_asset() {
        // NFT-classified contracts live in the `nfts` table, not `assets`.
        let wasm = "bb".repeat(32);
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CNFT002".into(),
            wasm_hash: Some(wasm.clone()),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Nft,
            is_sac: false,
            name: None,
            sac_asset: None,
        }];
        let interfaces = vec![iface(&wasm, &["owner_of", "token_uri", "transfer"])];

        let assets = detect_assets(&deployments, &interfaces);
        assert!(assets.is_empty());
    }

    #[test]
    fn other_wasm_deployment_produces_no_asset() {
        // Unknown contract surface — no asset row; a later WASM upload
        // may promote it via reclassify_contracts_from_wasm.
        let wasm = "cc".repeat(32);
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "COTH003".into(),
            wasm_hash: Some(wasm.clone()),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Other,
            is_sac: false,
            name: None,
            sac_asset: None,
        }];
        let interfaces = vec![iface(&wasm, &["execute", "admin", "init"])];

        let assets = detect_assets(&deployments, &interfaces);
        assert!(assets.is_empty());
    }

    #[test]
    fn dual_interface_contract_produces_no_asset_row() {
        // Precedence in classify_contract_from_wasm_spec: NFT wins over
        // Fungible when both discriminators present. Correct downstream
        // behaviour: the contract goes to `nfts` filter — NOT `assets`.
        let wasm = "dd".repeat(32);
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CDUAL04".into(),
            wasm_hash: Some(wasm.clone()),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Nft,
            is_sac: false,
            name: None,
            sac_asset: None,
        }];
        let interfaces = vec![iface(&wasm, &["owner_of", "decimals", "transfer"])];

        let assets = detect_assets(&deployments, &interfaces);
        assert!(assets.is_empty());
    }

    #[test]
    fn sac_and_fungible_in_same_batch_both_produce_assets() {
        let wasm = "ee".repeat(32);
        let deployments = vec![
            ExtractedContractDeployment {
                contract_id: "CSAC005".into(),
                wasm_hash: None,
                deployer_account: None,
                deployed_at_ledger: 100,
                contract_type: ContractType::Token,
                is_sac: true,
                name: None,
                sac_asset: Some(SacAssetIdentity::Credit {
                    code: "USDC".into(),
                    issuer: "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".into(),
                }),
            },
            ExtractedContractDeployment {
                contract_id: "CFUN006".into(),
                wasm_hash: Some(wasm.clone()),
                deployer_account: None,
                deployed_at_ledger: 100,
                contract_type: ContractType::Fungible,
                is_sac: false,
                name: None,
                sac_asset: None,
            },
        ];
        let interfaces = vec![iface(&wasm, &["transfer", "decimals", "allowance"])];

        let assets = detect_assets(&deployments, &interfaces);
        assert_eq!(assets.len(), 2);
        let by_contract: std::collections::HashMap<_, _> = assets
            .iter()
            .map(|t| (t.contract_id.as_deref().unwrap(), t.asset_type))
            .collect();
        assert_eq!(by_contract.get("CSAC005"), Some(&TokenAssetType::Sac));
        assert_eq!(by_contract.get("CFUN006"), Some(&TokenAssetType::Soroban));
    }

    // -- NFT Detection Tests --

    #[test]
    fn nft_mint_event_produces_nft() {
        let events = vec![NftEvent {
            transaction_hash: "abc".into(),
            contract_id: "CNFT789".into(),
            event_kind: "mint".into(),
            token_id: json!({"type": "u32", "value": 42}),
            from: None,
            to: Some("GOWNER".into()),
            ledger_sequence: 100,
            created_at: 1700000000,
        }];

        let nfts = detect_nfts(&events);
        assert_eq!(nfts.len(), 1);
        assert_eq!(nfts[0].contract_id, "CNFT789");
        assert_eq!(nfts[0].token_id, "42");
        assert_eq!(nfts[0].owner_account.as_deref(), Some("GOWNER"));
        assert_eq!(nfts[0].minted_at_ledger, Some(100));
    }

    #[test]
    fn nft_transfer_event() {
        let events = vec![NftEvent {
            transaction_hash: "abc".into(),
            contract_id: "CNFT789".into(),
            event_kind: "transfer".into(),
            token_id: json!({"type": "u32", "value": 42}),
            from: Some("GFROM".into()),
            to: Some("GTO".into()),
            ledger_sequence: 200,
            created_at: 1700001000,
        }];

        let nfts = detect_nfts(&events);
        assert_eq!(nfts.len(), 1);
        assert_eq!(nfts[0].owner_account.as_deref(), Some("GTO"));
        assert!(nfts[0].minted_at_ledger.is_none());
    }

    #[test]
    fn nft_burn_event() {
        let events = vec![NftEvent {
            transaction_hash: "abc".into(),
            contract_id: "CNFT789".into(),
            event_kind: "burn".into(),
            token_id: json!({"type": "string", "value": "unique-nft-id"}),
            from: Some("GFROM".into()),
            to: None,
            ledger_sequence: 300,
            created_at: 1700002000,
        }];

        let nfts = detect_nfts(&events);
        assert_eq!(nfts.len(), 1);
        assert_eq!(nfts[0].token_id, "unique-nft-id");
        assert!(nfts[0].owner_account.is_none());
    }

    #[test]
    fn empty_token_id_skipped() {
        let events = vec![NftEvent {
            transaction_hash: "abc".into(),
            contract_id: "CNFT789".into(),
            event_kind: "mint".into(),
            token_id: json!({"type": "void", "value": null}),
            from: None,
            to: Some("GOWNER".into()),
            ledger_sequence: 100,
            created_at: 1700000000,
        }];

        let nfts = detect_nfts(&events);
        assert!(nfts.is_empty());
    }
}
