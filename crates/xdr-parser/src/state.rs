//! Derived state extraction from raw ledger entry changes.
//!
//! Processes `ExtractedLedgerEntryChange` records to produce higher-level
//! entities: contract deployments, account states, liquidity pools,
//! assets, and NFTs. This is the final parsing stage before DB persistence.

use serde_json::Value;

use crate::classification::{ContractClassification, classify_contract_from_wasm_spec};
use crate::types::{
    ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedLedgerEntryChange, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedNft, ExtractedOperation, NftEvent,
};
use domain::{ContractType, OperationType, TokenAssetType};

// ---------------------------------------------------------------------------
// Step 1 + Step 7: Contract Deployment + SAC Detection
// ---------------------------------------------------------------------------

/// Extract contract deployments from ledger entry changes.
///
/// Identifies new contract instances by looking for `contract_data` entries
/// with the contract instance key. Detects SACs from the executable type.
pub fn extract_contract_deployments(
    changes: &[ExtractedLedgerEntryChange],
    tx_source_account: &str,
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

        deployments.push(ExtractedContractDeployment {
            contract_id,
            wasm_hash,
            deployer_account: Some(tx_source_account.to_string()),
            deployed_at_ledger: change.ledger_sequence,
            contract_type,
            is_sac,
            metadata: serde_json::json!({}),
        });
    }

    deployments
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
                        // Skip pool_share trustlines — LP positions, not asset balances
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
// Step 5: Asset Detection
// ---------------------------------------------------------------------------

/// Detect assets from contract deployments.
///
/// Two paths produce an [`ExtractedAsset`]:
///
/// 1. **SAC deployments** — [`TokenAssetType::Sac`] row; identity is
///    `contract_id`, plus the underlying classic asset_code/issuer if
///    available (not yet populated here).
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
            assets.push(ExtractedAsset {
                asset_type: TokenAssetType::Sac,
                asset_code: None,
                issuer_address: None,
                contract_id: Some(deployment.contract_id.clone()),
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
                name: None,
                total_supply: None,
                holder_count: None,
            });
        }
    }

    assets
}

// ---------------------------------------------------------------------------
// Task 0160: SAC underlying asset identity from CreateContract preimage
// ---------------------------------------------------------------------------

/// Underlying classic asset identity carried by a SAC deployment.
///
/// Sourced from `CreateContractArgs.contract_id_preimage` with variant
/// `FromAsset(Asset)`. The ContractInstance XDR entry for a SAC is a
/// marker-only `{"type": "stellar_asset"}` and carries no asset data,
/// so this is the sole path for populating `assets.asset_code` /
/// `.issuer_id` for SAC rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SacAssetIdentity {
    /// XLM-SAC — wraps the native Stellar asset. No code / issuer.
    /// Downstream (`detect_assets`) applies the XLM-SAC sentinel.
    Native,
    /// Classic-credit SAC — wraps a `credit_alphanum4` or
    /// `credit_alphanum12` asset with a real issuer.
    Credit {
        /// Asset code (trailing NULs already stripped by the parser).
        code: String,
        /// Issuer `G...` StrKey.
        issuer: String,
    },
}

/// Pull the underlying classic asset out of a CreateContract / CreateContractV2
/// operation whose preimage is `FromAsset`.
///
/// Returns `None` when the operation is not a `CreateContract*` op, when the
/// preimage is not `FromAsset` (i.e. regular `FromAddress` Soroban deploy),
/// or when the JSON shape is malformed.
///
/// Reads `ExtractedOperation.details["contractIdPreimage"]` populated by
/// `format_contract_id_preimage` in `operation.rs` — both paths must stay
/// in sync.
pub fn extract_sac_asset_from_create_contract(op: &ExtractedOperation) -> Option<SacAssetIdentity> {
    if op.op_type != OperationType::InvokeHostFunction {
        return None;
    }
    let host_fn_type = op.details.get("hostFunctionType").and_then(Value::as_str)?;
    if host_fn_type != "createContract" && host_fn_type != "createContractV2" {
        return None;
    }
    let preimage = op.details.get("contractIdPreimage")?;
    if preimage.get("type").and_then(Value::as_str) != Some("from_asset") {
        return None;
    }
    let asset = preimage.get("asset")?;
    match asset.get("type").and_then(Value::as_str)? {
        "native" => Some(SacAssetIdentity::Native),
        "credit_alphanum4" | "credit_alphanum12" => {
            let code = asset.get("asset_code").and_then(Value::as_str)?.to_string();
            let issuer = asset.get("issuer").and_then(Value::as_str)?.to_string();
            Some(SacAssetIdentity::Credit { code, issuer })
        }
        _ => None,
    }
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

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER");
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

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER");
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

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER");
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

        let deployments = extract_contract_deployments(&changes, "GDEPLOYER");
        assert!(deployments.is_empty());
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
    fn sac_deployment_produces_asset() {
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CSAC456".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            metadata: json!({}),
        }];

        let assets = detect_assets(&deployments, &[]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, TokenAssetType::Sac);
        assert_eq!(assets[0].contract_id.as_deref(), Some("CSAC456"));
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
            metadata: json!({}),
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
            metadata: json!({}),
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
            metadata: json!({}),
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
            metadata: json!({}),
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
            metadata: json!({}),
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
                metadata: json!({}),
            },
            ExtractedContractDeployment {
                contract_id: "CFUN006".into(),
                wasm_hash: Some(wasm.clone()),
                deployer_account: None,
                deployed_at_ledger: 100,
                contract_type: ContractType::Fungible,
                is_sac: false,
                metadata: json!({}),
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

    // -- Task 0160: SAC asset identity extraction tests --

    fn make_create_contract_op(details: Value) -> ExtractedOperation {
        ExtractedOperation {
            transaction_hash: "txhash".into(),
            operation_index: 0,
            op_type: OperationType::InvokeHostFunction,
            source_account: None,
            details,
        }
    }

    #[test]
    fn sac_asset_credit4_identity_extracted() {
        let op = make_create_contract_op(json!({
            "hostFunctionType": "createContract",
            "executable": { "type": "stellar_asset" },
            "contractIdPreimage": {
                "type": "from_asset",
                "asset": {
                    "type": "credit_alphanum4",
                    "asset_code": "USDC",
                    "issuer": "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
                }
            }
        }));
        let identity = extract_sac_asset_from_create_contract(&op);
        assert_eq!(
            identity,
            Some(SacAssetIdentity::Credit {
                code: "USDC".into(),
                issuer: "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".into(),
            })
        );
    }

    #[test]
    fn sac_asset_credit12_identity_extracted() {
        let op = make_create_contract_op(json!({
            "hostFunctionType": "createContractV2",
            "executable": { "type": "stellar_asset" },
            "contractIdPreimage": {
                "type": "from_asset",
                "asset": {
                    "type": "credit_alphanum12",
                    "asset_code": "YXLM",
                    "issuer": "GARDNV3Q7YGT4AKSDF25LT32YSCCW4EV22Y2TV3I2PU2MMXJTEDL5T55",
                }
            }
        }));
        let identity = extract_sac_asset_from_create_contract(&op);
        assert_eq!(
            identity,
            Some(SacAssetIdentity::Credit {
                code: "YXLM".into(),
                issuer: "GARDNV3Q7YGT4AKSDF25LT32YSCCW4EV22Y2TV3I2PU2MMXJTEDL5T55".into(),
            })
        );
    }

    #[test]
    fn sac_asset_native_identity_extracted() {
        let op = make_create_contract_op(json!({
            "hostFunctionType": "createContract",
            "executable": { "type": "stellar_asset" },
            "contractIdPreimage": {
                "type": "from_asset",
                "asset": { "type": "native" }
            }
        }));
        let identity = extract_sac_asset_from_create_contract(&op);
        assert_eq!(identity, Some(SacAssetIdentity::Native));
    }

    #[test]
    fn from_address_preimage_returns_none() {
        let op = make_create_contract_op(json!({
            "hostFunctionType": "createContract",
            "executable": { "type": "wasm", "hash": "de".repeat(32) },
            "contractIdPreimage": {
                "type": "from_address",
                "address": "GDEPLOYER...",
                "salt": "ab".repeat(32),
            }
        }));
        assert_eq!(extract_sac_asset_from_create_contract(&op), None);
    }

    #[test]
    fn non_create_contract_op_returns_none() {
        let op = make_create_contract_op(json!({
            "hostFunctionType": "invokeContract",
            "contractId": "CABC123",
            "functionName": "transfer",
        }));
        assert_eq!(extract_sac_asset_from_create_contract(&op), None);
    }

    #[test]
    fn non_invoke_host_function_op_returns_none() {
        let op = ExtractedOperation {
            transaction_hash: "txhash".into(),
            operation_index: 0,
            op_type: OperationType::Payment,
            source_account: None,
            details: json!({ "asset": "USDC:GA5Z..." }),
        };
        assert_eq!(extract_sac_asset_from_create_contract(&op), None);
    }

    #[test]
    fn malformed_preimage_returns_none() {
        // Missing `asset` subtree entirely.
        let op = make_create_contract_op(json!({
            "hostFunctionType": "createContract",
            "executable": { "type": "stellar_asset" },
            "contractIdPreimage": { "type": "from_asset" }
        }));
        assert_eq!(extract_sac_asset_from_create_contract(&op), None);
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
