//! Derived state extraction from raw ledger entry changes.
//!
//! Processes `ExtractedLedgerEntryChange` records to produce higher-level
//! entities: contract deployments, account states, liquidity pools,
//! tokens, and NFTs. This is the final parsing stage before DB persistence.

use serde_json::Value;

use crate::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedLedgerEntryChange,
    ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedNft, ExtractedToken, NftEvent,
};

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
        let contract_type = if is_sac {
            "token".to_string()
        } else {
            "other".to_string()
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

/// Extract account states from ledger entry changes.
///
/// Filters for account entries with "created" or "updated" change types.
pub fn extract_account_states(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedAccountState> {
    let mut accounts = Vec::new();

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

        let sequence_number = data.get("seq_num").and_then(|v| v.as_i64()).unwrap_or(0);

        let balance = data.get("balance").and_then(|v| v.as_i64()).unwrap_or(0);
        let balances = serde_json::json!([{ "asset_type": "native", "balance": balance }]);

        let home_domain = data
            .get("home_domain")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let is_creation = matches!(change.change_type.as_str(), "created" | "restored");
        accounts.push(ExtractedAccountState {
            account_id,
            first_seen_ledger: if is_creation {
                Some(change.ledger_sequence)
            } else {
                None
            },
            last_seen_ledger: change.ledger_sequence,
            sequence_number,
            balances,
            home_domain,
            created_at: change.created_at,
        });
    }

    accounts
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
// Step 5: Token Detection
// ---------------------------------------------------------------------------

/// Detect tokens from contract deployments.
///
/// SAC deployments produce "sac" tokens. Other deployments with token-like
/// interfaces could produce "soroban" tokens (heuristic-based).
pub fn detect_tokens(deployments: &[ExtractedContractDeployment]) -> Vec<ExtractedToken> {
    let mut tokens = Vec::new();

    for deployment in deployments {
        if deployment.is_sac {
            tokens.push(ExtractedToken {
                asset_type: "sac".to_string(),
                asset_code: None,
                issuer_address: None,
                contract_id: Some(deployment.contract_id.clone()),
                name: None,
                total_supply: None,
                holder_count: None,
            });
        }
    }

    tokens
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
        assert_eq!(deployments[0].contract_type, "other");
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
        assert_eq!(deployments[0].contract_type, "token");
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

    // -- Token Detection Tests --

    #[test]
    fn sac_deployment_produces_token() {
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CSAC456".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: "token".into(),
            is_sac: true,
            metadata: json!({}),
        }];

        let tokens = detect_tokens(&deployments);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].asset_type, "sac");
        assert_eq!(tokens[0].contract_id.as_deref(), Some("CSAC456"));
    }

    #[test]
    fn non_sac_deployment_no_token() {
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CABC123".into(),
            wasm_hash: Some("aa".repeat(32)),
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: "other".into(),
            is_sac: false,
            metadata: json!({}),
        }];

        let tokens = detect_tokens(&deployments);
        assert!(tokens.is_empty());
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
