//! Derived state extraction from raw ledger entry changes.
//!
//! Processes `ExtractedLedgerEntryChange` records to produce higher-level
//! entities: contract deployments, account states, liquidity pools,
//! assets, and NFTs. This is the final parsing stage before DB persistence.

use std::collections::HashMap;

use serde_json::Value;
use tracing::{instrument, warn};

use crate::classification::{ContractClassification, classify_contract_from_wasm_spec};
use crate::types::{
    ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedContractMetadata, ExtractedLedgerEntryChange, ExtractedLiquidityPool,
    ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft, ExtractedNftEvent,
    ExtractedSorobanBalance, NftEvent, SacAssetIdentity,
};
use domain::{AssetFamily, ContractType, NftEventType};

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
///
/// `deployer_by_contract` maps `contract_id` to the per-op effective
/// source for every `CreateContract*` reachable from the batch's
/// envelopes — top-level op effective source (op.source_account override
/// OR tx source) and auth-tree `CreateContractHostFn` signer
/// (`SorobanAuthorizationEntry.credentials`). When the map carries an
/// entry for the deployed `contract_id`, that StrKey wins; otherwise
/// `tx_source_account` is used as fallback. The fallback preserves
/// behaviour for the ~88 % of mainnet deploys where the op inherits the
/// tx source (no per-op override and no auth indirection). Built by
/// `crate::extract_op_source_per_contract` at the indexer call site.
/// Task 0255 Phase 1.
pub fn extract_contract_deployments(
    changes: &[ExtractedLedgerEntryChange],
    tx_source_account: &str,
    sac_identities: &HashMap<String, SacAssetIdentity>,
    deployer_by_contract: &HashMap<String, String>,
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

        let deployer_account = deployer_by_contract
            .get(&contract_id)
            .cloned()
            .or_else(|| Some(tx_source_account.to_string()));

        deployments.push(ExtractedContractDeployment {
            contract_id,
            wasm_hash,
            deployer_account,
            deployed_at_ledger: change.ledger_sequence,
            contract_type,
            is_sac,
            sac_asset,
        });
    }

    deployments
}

/// Extract token-metadata writes from contract-instance `created` / `updated`
/// changes that carry a `Symbol("METADATA")` struct in instance storage.
///
/// Reads the typed `change.token_metadata` (populated in `ledger_entry_changes`,
/// chain-verified location — task 0297) rather than re-decoding. Emits one
/// [`ExtractedContractMetadata`] per qualifying change, for the
/// `soroban_contract_metadata` side table (task 0297).
///
/// - `created` + `updated` + `restored` carry the current value and are kept;
///   `state` (pre-image) and `removed` are ignored. `restored` matters because
///   an instance restored from archival is the first time live ingestion may
///   see a contract's METADATA — dropping it would leave a cold-start hole.
/// - **SACs are skipped at extraction**: `entry_token_metadata` already returns
///   `None` for SAC instances (their name/symbol/decimals derive from the asset
///   identity), so a SAC change simply has no `token_metadata` to emit here.
pub fn extract_contract_metadata_writes(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedContractMetadata> {
    let mut out = Vec::new();
    for change in changes {
        // Cheap structural guards first, so we never clone metadata for a
        // change we then drop.
        if change.entry_type != "contract_data" {
            continue;
        }
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored"
        ) {
            continue;
        }
        if !is_contract_instance_key(&change.key) {
            continue;
        }
        let Some(contract_id) = extract_contract_id_from_key(&change.key) else {
            continue;
        };
        // `None` for SACs (skipped at extraction) and for instances without a
        // METADATA struct — both correctly drop out here.
        let Some(metadata) = change.token_metadata.clone() else {
            continue;
        };
        out.push(ExtractedContractMetadata {
            contract_id,
            metadata,
            ledger: change.ledger_sequence,
        });
    }
    out
}

/// Pull the `contract` StrKey from a ContractData ledger key. Used by
/// `extract_contract_metadata_writes` to dispatch instance-storage METADATA
/// writes to the right contract row.
fn extract_contract_id_from_key(key: &Value) -> Option<String> {
    key.get("contract")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extract per-holder Soroban token balances from `ContractData`
/// `Balance(Address)` ledger-entry changes (task 0331).
///
/// Reads ledger STATE (the current stored balance), not an event-fold —
/// correct-by-construction for vault / rebasing / non-SEP-41-event tokens where
/// a fold under-counts (README DECISION 2026-06-29). Persisted into the unified
/// `balances` table (task 0331 Option C — the per-type `soroban_token_balances`
/// table was dropped on the pivot).
///
/// Recognises the standard `Vec[Symbol("Balance"), Address]` key with EITHER
/// value shape: a bare `i128` (a type-3 Soroban token balance) OR the SAC
/// `BalanceValue` struct (a contract-held classic/native asset, held via the
/// asset's SAC — task 0331). This extractor emits every balance keyed by the
/// STORING contract; the type distinction is resolved downstream in
/// `build_balance_rows`, which keeps type-3 on its own surrogate but re-keys a
/// SAC-held balance onto the wrapped classic/native asset_id via the `asset_sac`
/// map (ADR 0051 — task 0339 retired the standalone type-2 SAC asset, so a SAC
/// balance now folds onto its type-0/1 row). Any other value shape is skipped,
/// never silently mis-summed.
pub fn extract_soroban_token_balances(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedSorobanBalance> {
    let mut out = Vec::new();
    for change in changes {
        if change.entry_type != "contract_data" {
            continue;
        }
        let Some(holder) = balance_key_holder(&change.key) else {
            continue;
        };
        let Some(contract_id) = extract_contract_id_from_key(&change.key) else {
            continue;
        };
        // `closed` carries what the 0 cannot: the ENTRY is gone, as opposed to a
        // holder who spent down to zero but still has one. ADR 0055.
        let closed = change.change_type == "removed";
        let balance = match change.change_type.as_str() {
            // Holder fully spent / entry archived → 0, so the RMT supersedes the
            // stale positive balance (mirrors trustline-removal → 0).
            "removed" => 0,
            // `created` / `updated` / `restored` carry the current value.
            // `state` (pre-image) is ignored — it shares the change's ledger, so
            // emitting it would let the RMT clobber the real value with the old.
            "created" | "updated" | "restored" => {
                let Some(data) = change.data.as_ref() else {
                    continue;
                };
                // Bare `i128` → type-3 token balance. SAC `BalanceValue` struct →
                // contract-held classic/native balance; take `.amount`. (Its
                // `authorized`/`clawback` flags are decodable but not propagated
                // yet — the frozen-balance policy is open, task 0331.) Any other
                // shape is skipped.
                if let Some(b) = decode_scval_i128(data) {
                    b
                } else if let Some(sac) = decode_sac_balance_value(data) {
                    sac.amount
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        out.push(ExtractedSorobanBalance {
            contract_id,
            holder,
            balance,
            ledger: change.ledger_sequence,
            closed,
        });
    }
    out
}

/// `Some(holder_strkey)` when `key.key` is the standard token balance key
/// `Vec[Symbol("Balance"), Address(holder)]`; `None` otherwise. The holder is
/// a `G…` account or `C…` contract — both are valid `ScAddress` holders.
fn balance_key_holder(key: &Value) -> Option<String> {
    // Token / SAC balances are PERSISTENT contract-data entries. Reject temporary
    // (or missing-durability) entries even when the inner shape matches, so a
    // foreign `Balance(Address)`-shaped temp entry is never summed as a balance.
    if key.get("durability")?.as_str()? != "persistent" {
        return None;
    }
    let inner = key.get("key")?;
    if inner.get("type")?.as_str()? != "vec" {
        return None;
    }
    let elems = inner.get("value")?.as_array()?;
    if elems.len() != 2 {
        return None;
    }
    let tag = &elems[0];
    if tag.get("type")?.as_str()? != "sym" || tag.get("value")?.as_str()? != "Balance" {
        return None;
    }
    let holder = &elems[1];
    if holder.get("type")?.as_str()? != "address" {
        return None;
    }
    Some(holder.get("value")?.as_str()?.to_string())
}

/// Decode `data.val` as a bare `i128` (the standard token balance value shape).
fn decode_scval_i128(data: &Value) -> Option<i128> {
    let val = data.get("val")?;
    if val.get("type")?.as_str()? != "i128" {
        return None;
    }
    val.get("value")?.as_str()?.parse::<i128>().ok()
}

/// The SAC `BalanceValue` struct — how a CONTRACT holds a classic/native asset.
///
/// A contract has no trustline; it holds a classic (type-1) or native (type-0) asset
/// as a `Balance(Address)` `ContractData` entry **inside that asset's SAC**, and the
/// value is this struct — NOT the bare `i128` a bespoke Soroban token (type-3) uses.
/// `scval_to_typed_json` serializes it as a `map` of symbol→value entries. Task 0331
/// (contract-held 0/1). `authorized`/`clawback` are carried so a later step can decide
/// whether a deauthorized/frozen balance counts toward supply/holders (open policy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SacBalanceValue {
    pub amount: i128,
    pub authorized: bool,
    pub clawback: bool,
}

/// Decode `data.val` as a SAC `BalanceValue` struct. `None` for any other shape —
/// including the bare-`i128` type-3 balance and a partial/foreign map — so the two
/// value shapes never cross-decode. Does NOT assign an asset: mapping the SAC contract
/// back to its classic/native asset (type-0/1) is the caller's job (task 0331 Problem B).
pub fn decode_sac_balance_value(data: &Value) -> Option<SacBalanceValue> {
    let val = data.get("val")?;
    if val.get("type")?.as_str()? != "map" {
        return None;
    }
    let mut amount = None;
    let mut authorized = None;
    let mut clawback = None;
    for entry in val.get("value")?.as_array()? {
        let key = entry.get("key")?;
        if key.get("type")?.as_str()? != "sym" {
            return None;
        }
        let field = entry.get("value")?;
        match key.get("value")?.as_str()? {
            "amount" => {
                if field.get("type")?.as_str()? != "i128" {
                    return None;
                }
                amount = Some(field.get("value")?.as_str()?.parse::<i128>().ok()?);
            }
            "authorized" => authorized = field.get("value")?.as_bool(),
            "clawback" => clawback = field.get("value")?.as_bool(),
            // Strict: an unknown symbol key means this is NOT the SAC
            // `BalanceValue` struct → reject, never partial-decode a foreign map.
            _ => return None,
        }
    }
    Some(SacBalanceValue {
        amount: amount?,
        authorized: authorized?,
        clawback: clawback?,
    })
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

    #[derive(Default)]
    struct AccountAccum {
        native_balance: Option<i64>,
        sequence_number: Option<i64>,
        home_domain: Option<String>,
        is_creation: bool,
        ledger_sequence: u32,
        created_at: i64,
        trustline_balances: Vec<Value>,
        removed_trustlines: Vec<Value>,
        /// Set by a `removed` account entry, cleared by any later
        /// created/updated/restored one — merge-then-recreate must not leave
        /// the account marked closed. ADR 0055.
        account_removed: bool,
        /// Some = an AccountEntry was observed (full-set semantics — an empty
        /// vec is a real "no signers" state). None = trustline-only accum;
        /// no signers row may be emitted. lore-0463.
        signers: Option<Vec<Value>>,
        thresholds: Option<String>,
        flags: Option<u32>,
    }

    let mut map: HashMap<String, AccountAccum> = HashMap::new();

    // Pass 1: account entries
    for change in changes {
        if change.entry_type != "account" {
            continue;
        }

        // AccountMerge tombstone (task 0295): a `removed` account entry is the
        // only way an account is deleted on Stellar. Emit native balance=0 at
        // the merge ledger so the stale balance row is superseded (the balances
        // table is RMT keyed on the higher ledger). account_id comes from the
        // change key — removed entries carry no data. Identity columns are not
        // set here; the separate RMT whole-row clobber is tracked in lore-0316.
        if change.change_type == "removed" {
            let account_id = change
                .key
                .get("account_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if account_id.is_empty() {
                continue;
            }
            let entry = map.entry(account_id).or_insert_with(|| AccountAccum {
                ledger_sequence: change.ledger_sequence,
                created_at: change.created_at,
                ..Default::default()
            });
            entry.native_balance = Some(0);
            entry.account_removed = true;
            entry.ledger_sequence = change.ledger_sequence;
            entry.created_at = change.created_at;
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
            ledger_sequence: change.ledger_sequence,
            created_at: change.created_at,
            ..Default::default()
        });
        entry.native_balance = Some(balance);
        entry.sequence_number = Some(seq);
        if hd.is_some() {
            entry.home_domain = hd;
        }
        // A live entry supersedes any removal seen earlier in this change set —
        // merge-then-recreate within one ledger must not stay marked closed.
        entry.account_removed = false;
        // Full-set semantics: the entry carries the COMPLETE signer list, so a
        // missing/empty array is a real "no signers" state, not absence of
        // data. Master is not in this list (thresholds byte 0). lore-0463.
        entry.signers = Some(
            data.get("signers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        entry.thresholds = data
            .get("thresholds")
            .and_then(Value::as_str)
            .map(str::to_string);
        entry.flags = data.get("flags").and_then(Value::as_u64).map(|f| f as u32);
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
                    ledger_sequence: change.ledger_sequence,
                    created_at: change.created_at,
                    ..Default::default()
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
                    ledger_sequence: change.ledger_sequence,
                    created_at: change.created_at,
                    ..Default::default()
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
                account_removed: accum.account_removed,
                signers: accum.signers,
                thresholds: accum.thresholds,
                flags: accum.flags,
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
        // Lore-0189: include `state` change_type. Stellar Core writes
        // a read-only `state` snapshot of every LedgerEntry referenced
        // (but not modified) by an operation. Skipping these used to
        // produce orphan `lp_positions` rows when a pool_share trustline
        // was created/updated/removed in a ledger that did not also
        // mutate the pool's reserves — the pool was visible in op_meta
        // only as `state`, the trustline carried the position update,
        // and the FK from `lp_positions.pool_id → liquidity_pools.pool_id`
        // tripped. Including `state` here lets us extract the full pool
        // dimension (asset_a, asset_b, fee, reserves) from the snapshot
        // and satisfy the FK without resorting to sentinel placeholders
        // for the common case. Snapshots emitted alongside are absorbed
        // by `liquidity_pool_snapshots`'s
        // `uq_lp_snapshots_pool_ledger DO NOTHING` (write.rs).
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored" | "state"
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
            created_at_ledger: if is_creation {
                Some(change.ledger_sequence)
            } else {
                None
            },
            last_updated_ledger: change.ledger_sequence,
            created_at: change.created_at,
        };

        pools.push(pool);

        // lore-0356: emit a snapshot for every liquidity-pool change that reaches
        // here (created/updated/restored/state). "One row per (pool, ledger)" is
        // delegated to `dedup_final_pool_snapshots`, which keeps the LAST image in
        // ledger apply order = the end-of-ledger reserves. We deliberately do NOT
        // drop `state` snapshots: for a pool mutated in the ledger the last change
        // is always the `updated` after-image (a `state` before-image is
        // immediately followed by its `updated`, so it never wins), while for a
        // pool referenced but not mutated (the lore-0189 case — e.g. a pool_share
        // trustline change) the lone `state` read IS its correct end-of-ledger
        // value; dropping it would leave that pool with no snapshot and blank
        // reserves in the read path.
        snapshots.push(ExtractedLiquidityPoolSnapshot {
            pool_id,
            ledger_sequence: change.ledger_sequence,
            created_at: change.created_at,
            reserves,
            total_shares,
        });
    }

    (pools, snapshots)
}

/// Collapse pool snapshots to exactly one per `(pool_id, ledger_sequence)`: the
/// LAST one in ledger apply order, i.e. the end-of-ledger (final) reserves.
///
/// Producers push a snapshot for every liquidity-pool change in apply order
/// (transaction order, then operation order), so the last snapshot for a
/// `(pool, ledger)` reflects the pool's committed state at ledger close — the
/// final `updated` after-image for a mutated pool, or the lone read-only `state`
/// value for a pool that was only referenced. Deduping here makes the stored
/// snapshot a deterministic function of the ledger (re-ingesting the same ledger
/// yields the same row) instead of leaving "one row per (pool, ledger)" to CH's
/// version-less `ReplacingMergeTree`, which would otherwise keep an arbitrary
/// intra-ledger image. See lore-0356.
///
/// Call once per ledger, after aggregating every transaction's snapshots
/// (`crate::fold::keep_last_by_key` carries the shared mechanism and the
/// belt-and-braces rationale for the `ledger_sequence` key component).
pub fn dedup_final_pool_snapshots(
    snapshots: Vec<ExtractedLiquidityPoolSnapshot>,
) -> Vec<ExtractedLiquidityPoolSnapshot> {
    crate::fold::keep_last_by_key(snapshots, |s| (s.pool_id.clone(), s.ledger_sequence))
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

        // The pool-share trustline is gone (participant left) versus withdrawn
        // to zero but still open — both write `shares = 0`. ADR 0055.
        let closed = change.change_type == "removed";
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
            closed,
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
/// 1. **SAC deployments** — folded onto the underlying asset as a FACET
///    (ADR 0051): a `classic_credit` (type 1) or `native` (type 0) row with the
///    SAC handle in `sac_contract_id` (the deploy's derived `C…`) + `sac_deployed
///    = true`; the key `contract_id` stays unset (reserved for soroban identity).
///    Identity comes from `deployment.sac_asset` (resolved from
///    `ContractIdPreimage::FromAsset` via `crate::sac::extract_sac_identities`
///    in the indexer). Two shapes:
///    - `Credit { code, issuer }` → the classic_credit row for that pair.
///    - `Native` → the native (type 0) row (NULL code/issuer).
///    - `None` (SAC deployment whose creating preimage is not in this
///      batch) is logged as a warn and skipped — better to lose one row
///      than fabricate identity.
/// 2. **WASM-based deployments classifying as
///    [`ContractClassification::Fungible`]** — [`AssetFamily::Soroban`]
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
/// Soroban rows carry identity only. On-chain name/symbol extraction from
/// ContractData storage entries is tracked as follow-up task 0156; a separate
/// scheduled-Lambda enrichment path for SEP-1 metadata lives under task 0124.
/// Supply/holders are not a parser concern at all — they are aggregated from
/// `balances` into `balance_aggregates` (0293/0331).
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
            // ADR 0051: a SAC is a FACET of its underlying classic_credit /
            // native asset, not a separate `asset_type`. Emit the underlying
            // asset row and record the SAC handle (`deployment.contract_id` is
            // the SAC's derived `C…` StrKey) + deployed=true in the facet
            // columns. Identity from the typed enum produced by the parser:
            //   Native             → the native (type=0) row, no code/issuer.
            //   Credit{code,issuer}→ the classic_credit (type=1) row.
            //   None               → preimage not in this batch; skip with
            //                        a warn rather than fabricate identity.
            let (asset_type, asset_code, issuer_address) = match &deployment.sac_asset {
                Some(SacAssetIdentity::Native) => (AssetFamily::Native, None, None),
                Some(SacAssetIdentity::Credit { code, issuer }) => (
                    AssetFamily::ClassicCredit,
                    Some(code.clone()),
                    Some(issuer.clone()),
                ),
                None => {
                    warn!(
                        contract_id = %deployment.contract_id,
                        "SAC deployment without resolved asset identity; skipping assets row"
                    );
                    continue;
                }
            };
            assets.push(ExtractedAsset {
                asset_type,
                asset_code,
                issuer_address,
                // Key `contract_id` stays reserved for soroban identity — the
                // SAC handle lives in the facet column, keeping this classic /
                // native row on its stable identity key (ORDER BY value 0).
                contract_id: None,
                sac_contract_id: Some(deployment.contract_id.clone()),
                sac_deployed: true,
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
                asset_type: AssetFamily::Soroban,
                asset_code: None,
                issuer_address: None,
                contract_id: Some(deployment.contract_id.clone()),
                // Bespoke Soroban token — no classic backing, so no SAC facet.
                sac_contract_id: None,
                sac_deployed: false,
            });
        }
    }

    assets
}

/// Detect **classic-credit** assets from observed trustline
/// `LedgerEntryChange`s (task 0219 — Karol's pre-audit Bug #1).
///
/// `detect_assets` above only emits SAC + Soroban-fungible rows from
/// observed contract deployments. Classic credits (USDC, AQUA, EURC,
/// every asset issued by a G-account) need their own producer because
/// no deployment-shaped observation carries their identity — the
/// authoritative carrier is the `trustline` `LedgerEntryChange`'s
/// `data.asset` field, which holds `{type, code, issuer}` for every
/// `credit_alphanum4` / `credit_alphanum12` asset.
///
/// Flow:
///
/// 1. Walk `changes` looking for `entry_type == "trustline"`.
/// 2. Read the asset payload from `data.asset` (live changes) or
///    fall back to `key.asset` (removed changes — `data` is `None`,
///    but the change's key still carries `{type, code, issuer}` per
///    `format_trustline_asset_key` on the ingest side). The fallback
///    matters for partial-window backfills whose first observation
///    of a `(code, issuer)` pair is a trustline removal.
/// 3. Skip `asset.type == "pool_share"` — those are LP positions,
///    handled by `extract_lp_positions`.
/// 4. Extract `(code, issuer)`; emit one `ExtractedAsset { asset_type:
///    ClassicCredit, asset_code: code, issuer_address: issuer }` per
///    distinct pair (dedup within this call).
///
/// The row carries identity only. `name` for classic credits lands via
/// Lambda 2's `sep1_assets` enrichment path (task 0195 §2a) — runtime SEP-1
/// stellar.toml fetch keyed on the `(code, issuer)` pair. Supply/holders are
/// aggregated from `balances` into `balance_aggregates` (0293/0331), never
/// written back onto this row.
///
/// The function is pure (no I/O, no DB) and idempotent on replay.
/// Downstream dedup in `Staged::prepare`
/// (`crates/indexer/src/handler/persist/staging.rs`, local
/// `asset_rows` accumulator keyed by the per-`asset_type` fingerprint)
/// already collapses same `(code, issuer)` from multiple sources to
/// one row before the `upsert_assets_classic_like` INSERT fires.
pub fn detect_classic_credit_assets(changes: &[ExtractedLedgerEntryChange]) -> Vec<ExtractedAsset> {
    use std::collections::HashSet;
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut assets: Vec<ExtractedAsset> = Vec::new();

    for change in changes {
        if change.entry_type != "trustline" {
            continue;
        }

        // Live changes (created / updated / restored / state) carry the
        // asset on `data.asset`. Removed changes have `data: None`, but
        // the change's `key.asset` still carries `{type, code, issuer}`
        // — so a partial-window backfill whose first observation is a
        // trustline removal still emits the asset row.
        let asset_source = change
            .data
            .as_ref()
            .and_then(|d| d.get("asset"))
            .or_else(|| change.key.get("asset"));
        let Some(asset) = asset_source.and_then(Value::as_object) else {
            continue;
        };
        let asset_type = asset.get("type").and_then(Value::as_str).unwrap_or("");
        // `pool_share` trustlines are LP positions, not asset balances —
        // handled by `extract_lp_positions`. Skipping here is intentional,
        // not a data drop.
        if asset_type == "pool_share" {
            continue;
        }
        let code = asset.get("code").and_then(Value::as_str).unwrap_or("");
        let issuer = asset.get("issuer").and_then(Value::as_str).unwrap_or("");
        if code.is_empty() || issuer.is_empty() {
            // Malformed trustline (shouldn't happen on mainnet); skip
            // rather than fabricate a partial-identity row.
            continue;
        }

        let key = (code.to_string(), issuer.to_string());
        if !seen.insert(key) {
            continue;
        }
        assets.push(ExtractedAsset {
            asset_type: AssetFamily::ClassicCredit,
            asset_code: Some(code.to_string()),
            issuer_address: Some(issuer.to_string()),
            contract_id: None,
            // A trustline observation carries no SAC signal; if this asset has
            // a SAC, the deploy/override path folds it onto this same row.
            sac_contract_id: None,
            sac_deployed: false,
        });
    }

    assets
}

/// Native XLM singleton bootstrap (task 0219 — Karol's pre-audit Bug #1).
///
/// Returns a single `ExtractedAsset { asset_type: Native }` row. The
/// indexer emits this once per ledger; the persist path
/// (`upsert_assets_native`) inserts via `WHERE NOT EXISTS` against
/// `uidx_assets_native`, so every call after the first is a no-op.
pub fn native_asset_singleton() -> ExtractedAsset {
    ExtractedAsset {
        asset_type: AssetFamily::Native,
        asset_code: None,
        issuer_address: None,
        contract_id: None,
        // XLM's SAC facet is folded on by the deploy/override path when seen.
        sac_contract_id: None,
        sac_deployed: false,
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

// ---------------------------------------------------------------------------
// Step 6b: NFT Ownership Event Extraction (task 0202)
// ---------------------------------------------------------------------------

/// Transform raw parser `NftEvent` records into schema-shaped
/// `ExtractedNftEvent` rows for `nft_ownership`.
///
/// The parser (`detect_nft_events`) emits events with a JSON-typed
/// `token_id`, string `event_kind` ("mint"/"transfer"/"burn"), and split
/// `from`/`to` fields. The persistence layer expects a stringified
/// `token_id`, the `NftEventType` enum, and a unified `owner_account`
/// field (`Some(to)` for mint/transfer, `None` for burn).
///
/// Additionally, this fn computes `event_order` — a per-`(contract, token,
/// ledger)` monotonic ordinal (SMALLINT) required by the schema PK
/// `(nft_id, created_at, ledger_sequence, event_order)` and by the
/// LEAD-window pagination in `17_get_nfts_transfers.sql`.
///
/// Events with empty `token_id` are skipped (matches `detect_nfts`
/// behaviour). Events with `event_kind` not in {"mint","transfer","burn"}
/// are skipped — the parser already restricts emission to these three
/// kinds, so the guard is defensive.
///
/// Pathological-input guard: `event_order` is persisted as SMALLINT so
/// the schema bound is `i16::MAX = 32_767`. Once a single
/// `(contract, token, ledger)` triple has already produced that many
/// rows, further events for the same triple are skipped with a warn
/// instead of overflowing the staging `try_into::<i16>()` and failing
/// the whole ledger. No real NFT contract reaches this bound; the cap
/// exists to keep ingestion robust against a malicious / buggy
/// contract emitting tens of thousands of events for one NFT in a
/// single ledger.
#[instrument(skip(events), fields(event_count = events.len()))]
pub fn extract_nft_ownership_events(events: &[NftEvent]) -> Vec<ExtractedNftEvent> {
    /// SMALLINT max — `nft_ownership.event_order` is stored as i16 in PG.
    const MAX_EVENT_ORDER: u16 = i16::MAX as u16;

    let mut order_counter: HashMap<(String, String, u32), u16> = HashMap::new();
    let mut out: Vec<ExtractedNftEvent> = Vec::with_capacity(events.len());

    for event in events {
        let token_id = token_id_to_string(&event.token_id);
        if token_id.is_empty() {
            continue;
        }

        let event_type = match event.event_kind.parse::<NftEventType>() {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    event_kind = %event.event_kind,
                    error = %e,
                    "unknown NFT event_kind — skipping (parser should not emit this)"
                );
                continue;
            }
        };

        let owner_account = match event_type {
            NftEventType::Mint | NftEventType::Transfer => event.to.clone(),
            NftEventType::Burn => None,
        };

        let key = (
            event.contract_id.clone(),
            token_id.clone(),
            event.ledger_sequence,
        );
        let counter = order_counter.entry(key).or_insert(0);
        if *counter > MAX_EVENT_ORDER {
            warn!(
                contract_id = %event.contract_id,
                token_id = %token_id,
                ledger_sequence = event.ledger_sequence,
                max = MAX_EVENT_ORDER,
                "event_order would exceed SMALLINT range; skipping further events for triple"
            );
            continue;
        }
        let event_order = *counter;
        *counter = counter.saturating_add(1);

        out.push(ExtractedNftEvent {
            transaction_hash: event.transaction_hash.clone(),
            contract_id: event.contract_id.clone(),
            token_id,
            event_type,
            owner_account,
            event_order,
            ledger_sequence: event.ledger_sequence,
            created_at: event.created_at,
        });
    }

    out
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
            token_metadata: None,
        }
    }

    // -- lore-0356: LP snapshot = deterministic end-of-ledger image (keep-last) --

    fn lp_change(
        change_type: &str,
        pool_id: &str,
        reserve_a: i64,
        reserve_b: i64,
        shares: i64,
    ) -> ExtractedLedgerEntryChange {
        make_change(
            "liquidity_pool",
            change_type,
            json!({}),
            Some(json!({
                "pool_id": pool_id,
                "params": { "asset_a": null, "asset_b": null, "fee": 30 },
                "reserve_a": reserve_a,
                "reserve_b": reserve_b,
                "total_pool_shares": shares,
            })),
        )
    }

    #[test]
    fn lp_snapshot_final_wins_over_before_image() {
        // Core writes `state` (before) then `updated` (after) per op. Both become
        // snapshots, but ledger-scope dedup keeps the LAST (the `updated`
        // after-image) — the stale before-image never wins for a mutated pool.
        let (pools, snapshots) = extract_liquidity_pools(&[
            lp_change("state", "POOL1", 100, 200, 50),
            lp_change("updated", "POOL1", 110, 182, 50),
        ]);
        assert_eq!(pools.len(), 2, "dimension extracted from both changes");
        assert_eq!(
            snapshots.len(),
            2,
            "a snapshot per change (state + updated)"
        );

        let deduped = dedup_final_pool_snapshots(snapshots);
        assert_eq!(deduped.len(), 1, "one snapshot per (pool, ledger)");
        assert_eq!(
            deduped[0].reserves,
            json!({ "a": 110, "b": 182 }),
            "final (after) image, not the stale before-image"
        );
    }

    #[test]
    fn lp_snapshot_kept_for_state_only_pool() {
        // lore-0356 regression guard: a pool referenced only as read-only `state`
        // (the lore-0189 dormant case) must still get exactly one snapshot carrying
        // its correct, unchanged reserves — otherwise the read path shows blank
        // reserves for a real pool.
        let (pools, snapshots) =
            extract_liquidity_pools(&[lp_change("state", "POOL1", 100, 200, 50)]);
        assert_eq!(pools.len(), 1);

        let deduped = dedup_final_pool_snapshots(snapshots);
        assert_eq!(deduped.len(), 1, "state-only pool keeps its snapshot");
        assert_eq!(deduped[0].reserves, json!({ "a": 100, "b": 200 }));
    }

    #[test]
    fn dedup_keeps_last_image_per_pool_ledger() {
        // Multi-op pool: keep the last (end-of-ledger) image, not before/intermediate.
        let (_pools, snapshots) = extract_liquidity_pools(&[
            lp_change("state", "POOL1", 100, 200, 50),
            lp_change("updated", "POOL1", 110, 190, 50),
            lp_change("state", "POOL1", 110, 190, 50),
            lp_change("updated", "POOL1", 121, 181, 50), // final
            lp_change("updated", "POOL2", 7, 8, 3),
        ]);
        assert_eq!(snapshots.len(), 5, "a snapshot per change (incl. state)");

        let deduped = dedup_final_pool_snapshots(snapshots);
        assert_eq!(deduped.len(), 2, "one snapshot per (pool, ledger)");
        let p1 = deduped.iter().find(|s| s.pool_id == "POOL1").unwrap();
        assert_eq!(
            p1.reserves,
            json!({ "a": 121, "b": 181 }),
            "final image, not before/intermediate"
        );
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

        let deployments =
            extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new(), &HashMap::new());
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

        let deployments =
            extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new(), &HashMap::new());
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

        let deployments =
            extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new(), &HashMap::new());
        assert!(deployments.is_empty());
    }

    // -- Soroban token balance tests (task 0331) --

    #[test]
    fn extract_balance_entry_bare_i128() {
        let key = json!({
            "contract": "CTOKEN1",
            "key": { "type": "vec", "value": [
                { "type": "sym", "value": "Balance" },
                { "type": "address", "value": "GHOLDER1" }
            ]},
            "durability": "persistent",
        });
        let mut data = key.clone();
        data["val"] = json!({ "type": "i128", "value": "800009446178" });
        let changes = vec![make_change("contract_data", "updated", key, Some(data))];

        let balances = extract_soroban_token_balances(&changes);
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].contract_id, "CTOKEN1");
        assert_eq!(balances[0].holder, "GHOLDER1");
        assert_eq!(balances[0].balance, 800_009_446_178_i128);
        assert_eq!(balances[0].ledger, 100);
    }

    /// Real mainnet end-to-end (RPC `getLedgerEntries`, 2026-07-01): decodes the
    /// ACTUAL `Balance(GAWOKP6N…)` ContractData entry for token `CCSNFZ5R…` at
    /// ledger 63268948 from on-chain XDR, converts its real key + val `ScVal`s
    /// via the REAL `scval_to_typed_json` (the exact JSON the ingestion emits),
    /// and asserts the live parser recovers contract + holder + balance. The
    /// i128 comes from the LEDGER BYTES (not a test constant) and equals the
    /// independent `stellar contract invoke … balance` read (10000040000000) —
    /// NON-circular.
    #[test]
    fn extract_balance_real_mainnet_entry() {
        use base64::Engine;
        use stellar_xdr::{LedgerEntryData, Limits, ReadXdr};

        let entry_b64 = "AAAABgAAAAAAAAABpNLnsQaIecmK0DuR3iIEA4DUoHpK2z+hSQS0L4ntArUAAAAQAAAAAQAAAAIAAAAPAAAAB0JhbGFuY2UAAAAAEgAAAAAAAAAALOU/zUgs2L4DJx225wMqTkYuiH78AX+HaE65g2akcB4AAAABAAAACgAAAAAAAAAAAAAJGFDU+gA=";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(entry_b64)
            .unwrap();
        let LedgerEntryData::ContractData(entry) =
            LedgerEntryData::from_xdr(&bytes, Limits::none()).unwrap()
        else {
            panic!("expected ContractData");
        };

        let key = json!({
            "contract": entry.contract.to_string(),
            "key": crate::scval::scval_to_typed_json(&entry.key),
            "durability": "persistent",
        });
        let mut data = key.clone();
        data["val"] = crate::scval::scval_to_typed_json(&entry.val);

        let balances = extract_soroban_token_balances(&[make_change(
            "contract_data",
            "updated",
            key,
            Some(data),
        )]);

        assert_eq!(balances.len(), 1, "one balance from the real entry");
        assert_eq!(
            balances[0].contract_id,
            "CCSNFZ5RA2EHTSMK2A5ZDXRCAQBYBVFAPJFNWP5BJECLIL4J5UBLLUQG"
        );
        assert_eq!(
            balances[0].holder,
            "GAWOKP6NJAWNRPQDE4O3NZYDFJHEMLUIP36AC74HNBHLTA3GURYB4PYJ"
        );
        assert_eq!(
            balances[0].balance, 10_000_040_000_000,
            "parser must decode the exact on-chain i128 from the real entry"
        );
    }

    /// Real mainnet (RPC `getLedgerEntries`, 2026-07-01): the ACTUAL SAC `BalanceValue`
    /// struct entries for the AMM pool `CATUJXDU…` holding native XLM and EURC (each held
    /// via the asset's SAC — the contract-held classic/native case the type-3 bare-`i128`
    /// path drops today). Decodes on-chain XDR → real `scval_to_typed_json` → the new
    /// `decode_sac_balance_value`, and asserts the amount equals the INDEPENDENT
    /// `stellar contract invoke … balance` read (native 11_635_129_310_963, EURC
    /// 2_026_487_623_620) — the i128 comes from the ledger bytes, NON-circular. Also
    /// asserts the bare-`i128` decoder rejects the struct (the two shapes never cross-decode).
    #[test]
    fn decode_sac_balance_value_real_mainnet() {
        use base64::Engine;
        use stellar_xdr::{LedgerEntryData, Limits, ReadXdr};

        for (entry_b64, expected_amount) in [
            (
                "AAAABgAAAAAAAAABJbT82FmuwvpjSEOMSJs8PBDJi20hvk/TyzDLaJU++XcAAAAQAAAAAQAAAAIAAAAPAAAAB0JhbGFuY2UAAAAAEgAAAAEnRNx0d+UpTAqVK9xT0ZTDwPQVQeA669CYruDz0/SWywAAAAEAAAARAAAAAQAAAAMAAAAPAAAABmFtb3VudAAAAAAACgAAAAAAAAAAAAAKlQO/3vMAAAAPAAAACmF1dGhvcml6ZWQAAAAAAAAAAAABAAAADwAAAAhjbGF3YmFjawAAAAAAAAAA",
                11_635_129_310_963_i128,
            ),
            (
                "AAAABgAAAAAAAAAB5qfZ63UjAGpGmqdIOtEQckdEPA2C5idj3mcISMTpfJAAAAAQAAAAAQAAAAIAAAAPAAAAB0JhbGFuY2UAAAAAEgAAAAEnRNx0d+UpTAqVK9xT0ZTDwPQVQeA669CYruDz0/SWywAAAAEAAAARAAAAAQAAAAMAAAAPAAAABmFtb3VudAAAAAAACgAAAAAAAAAAAAAB19QTL8QAAAAPAAAACmF1dGhvcml6ZWQAAAAAAAAAAAABAAAADwAAAAhjbGF3YmFjawAAAAAAAAAA",
                2_026_487_623_620_i128,
            ),
        ] {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(entry_b64)
                .unwrap();
            let LedgerEntryData::ContractData(entry) =
                LedgerEntryData::from_xdr(&bytes, Limits::none()).unwrap()
            else {
                panic!("expected ContractData");
            };
            let data = json!({ "val": crate::scval::scval_to_typed_json(&entry.val) });

            let decoded = decode_sac_balance_value(&data).expect("SAC BalanceValue decodes");
            assert_eq!(
                decoded.amount, expected_amount,
                "amount must equal the independent on-chain balance() read"
            );
            assert!(decoded.authorized, "pool balances are authorized");
            assert!(!decoded.clawback, "no clawback on these balances");

            // The two value shapes must never cross-decode.
            assert!(
                decode_scval_i128(&data).is_none(),
                "bare-i128 decoder must reject the SAC struct"
            );
        }
    }

    #[test]
    fn decode_sac_balance_value_rejects_foreign_maps() {
        // Missing a required field (no `clawback`) → None.
        let missing = json!({ "val": { "type": "map", "value": [
            { "key": { "type": "sym", "value": "amount" }, "value": { "type": "i128", "value": "5" } },
            { "key": { "type": "sym", "value": "authorized" }, "value": { "type": "bool", "value": true } },
        ]}});
        assert!(decode_sac_balance_value(&missing).is_none());

        // An unknown extra symbol key → None (not the SAC struct, don't partial-decode).
        let foreign = json!({ "val": { "type": "map", "value": [
            { "key": { "type": "sym", "value": "amount" }, "value": { "type": "i128", "value": "5" } },
            { "key": { "type": "sym", "value": "authorized" }, "value": { "type": "bool", "value": true } },
            { "key": { "type": "sym", "value": "clawback" }, "value": { "type": "bool", "value": false } },
            { "key": { "type": "sym", "value": "extra" }, "value": { "type": "i128", "value": "9" } },
        ]}});
        assert!(decode_sac_balance_value(&foreign).is_none());

        // A bare i128 (type-3 shape) is not a map → None.
        let bare = json!({ "val": { "type": "i128", "value": "5" } });
        assert!(decode_sac_balance_value(&bare).is_none());
    }

    /// Contract-held SAC balance end-to-end (real mainnet entry): `extract_soroban_token_balances`
    /// now emits the contract-held balance for a SAC `BalanceValue` struct — the
    /// pool `CATUJXDU…` holding native XLM in the XLM SAC. Same real entry as
    /// `decode_sac_balance_value_real_mainnet`, but asserts the WHOLE extractor
    /// (not just the value decoder) surfaces it: contract = the SAC, holder = the
    /// pool, balance = the independent on-chain read. Keyed downstream by the SAC
    /// surrogate → lands on the SAC's type-2 asset row (task 0339 folds to type-0).
    #[test]
    fn extract_sac_struct_balance_real_mainnet() {
        use base64::Engine;
        use stellar_xdr::{LedgerEntryData, Limits, ReadXdr};

        let entry_b64 = "AAAABgAAAAAAAAABJbT82FmuwvpjSEOMSJs8PBDJi20hvk/TyzDLaJU++XcAAAAQAAAAAQAAAAIAAAAPAAAAB0JhbGFuY2UAAAAAEgAAAAEnRNx0d+UpTAqVK9xT0ZTDwPQVQeA669CYruDz0/SWywAAAAEAAAARAAAAAQAAAAMAAAAPAAAABmFtb3VudAAAAAAACgAAAAAAAAAAAAAKlQO/3vMAAAAPAAAACmF1dGhvcml6ZWQAAAAAAAAAAAABAAAADwAAAAhjbGF3YmFjawAAAAAAAAAA";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(entry_b64)
            .unwrap();
        let LedgerEntryData::ContractData(entry) =
            LedgerEntryData::from_xdr(&bytes, Limits::none()).unwrap()
        else {
            panic!("expected ContractData");
        };
        let key = json!({
            "contract": entry.contract.to_string(),
            "key": crate::scval::scval_to_typed_json(&entry.key),
            "durability": "persistent",
        });
        let mut data = key.clone();
        data["val"] = crate::scval::scval_to_typed_json(&entry.val);

        let balances = extract_soroban_token_balances(&[make_change(
            "contract_data",
            "updated",
            key,
            Some(data),
        )]);

        assert_eq!(
            balances.len(),
            1,
            "extractor surfaces the SAC struct balance"
        );
        assert_eq!(
            balances[0].contract_id, "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA",
            "stored in the native XLM SAC"
        );
        assert_eq!(
            balances[0].holder, "CATUJXDUO7SSSTAKSUV5YU6RSTB4B5AVIHQDV26QTCXOB46T6SLMWNMY",
            "held by the pool contract"
        );
        assert_eq!(
            balances[0].balance, 11_635_129_310_963,
            "the exact on-chain SAC balance"
        );
    }

    #[test]
    fn removed_balance_entry_emits_zero() {
        // Holder fully spent → entry removed (data is None). Must emit balance 0
        // so the RMT supersedes the stale positive balance (else over-count).
        let changes = vec![make_change(
            "contract_data",
            "removed",
            json!({
                "contract": "CTOKEN1",
                "key": { "type": "vec", "value": [
                    { "type": "sym", "value": "Balance" },
                    { "type": "address", "value": "GHOLDER1" }
                ]},
                "durability": "persistent",
            }),
            None,
        )];

        let balances = extract_soroban_token_balances(&changes);
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].holder, "GHOLDER1");
        assert_eq!(balances[0].balance, 0);
    }

    #[test]
    fn skip_state_preimage_balance_entry() {
        // `state` is the pre-image (shares the change's ledger). Emitting it would
        // let the RMT clobber the real value with the old balance.
        let key = json!({
            "contract": "CTOKEN1",
            "key": { "type": "vec", "value": [
                { "type": "sym", "value": "Balance" },
                { "type": "address", "value": "GHOLDER1" }
            ]},
            "durability": "persistent",
        });
        let mut data = key.clone();
        data["val"] = json!({ "type": "i128", "value": "999" });
        let changes = vec![make_change("contract_data", "state", key, Some(data))];
        assert!(extract_soroban_token_balances(&changes).is_empty());
    }

    #[test]
    fn skip_temporary_durability_balance_entry() {
        // Exact `Balance(Address)` shape but TEMPORARY durability — a real token /
        // SAC balance is PERSISTENT, so this foreign temp entry must be skipped.
        let key = json!({
            "contract": "CTOKEN1",
            "key": { "type": "vec", "value": [
                { "type": "sym", "value": "Balance" },
                { "type": "address", "value": "GHOLDER1" }
            ]},
            "durability": "temporary",
        });
        let mut data = key.clone();
        data["val"] = json!({ "type": "i128", "value": "999" });
        let changes = vec![make_change("contract_data", "updated", key, Some(data))];
        assert!(extract_soroban_token_balances(&changes).is_empty());
    }

    #[test]
    fn skip_non_balance_contract_data_for_balances() {
        // Symbol("name") and instance keys must not be mistaken for a balance.
        let changes = vec![make_change(
            "contract_data",
            "updated",
            json!({
                "contract": "CTOKEN1",
                "key": { "type": "sym", "value": "name" },
                "durability": "persistent",
            }),
            Some(json!({
                "contract": "CTOKEN1",
                "key": { "type": "sym", "value": "name" },
                "durability": "persistent",
                "val": { "type": "string", "value": "MERU" },
            })),
        )];
        assert!(extract_soroban_token_balances(&changes).is_empty());
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

        let deployments =
            extract_contract_deployments(&changes, "GDEPLOYER", &HashMap::new(), &HashMap::new());
        assert!(deployments.is_empty());
    }

    // -- extract_contract_metadata_writes (task 0297 side-table source) --

    /// Helper: a contract-instance change with a given executable type and a
    /// preset `token_metadata` (mirrors what `ledger_entry_changes` produces).
    fn make_instance_meta_change(
        contract_id: &str,
        change_type: &str,
        executable_type: &str,
        meta: crate::token_metadata::TokenMetadata,
    ) -> ExtractedLedgerEntryChange {
        let exec = if executable_type == "wasm" {
            json!({ "type": "wasm", "hash": "aa".repeat(32) })
        } else {
            json!({ "type": "stellar_asset" })
        };
        let mut c = make_change(
            "contract_data",
            change_type,
            json!({
                "contract": contract_id,
                "key": { "type": "ledger_key_contract_instance", "value": null },
                "durability": "persistent",
            }),
            Some(
                json!({ "val": { "type": "contract_instance", "value": { "executable": exec } } }),
            ),
        );
        // Mirror `entry_token_metadata`: a SAC instance yields no metadata.
        c.token_metadata = if executable_type == "stellar_asset" {
            None
        } else {
            Some(meta)
        };
        c
    }

    #[test]
    fn extract_metadata_writes_from_wasm_instance() {
        let c = make_instance_meta_change(
            "CWASMTOKEN",
            "created",
            "wasm",
            crate::token_metadata::TokenMetadata {
                name: Some("liquidFi bridge token".into()),
                symbol: Some("lUSDC".into()),
                decimals: Some(7),
            },
        );
        let writes = extract_contract_metadata_writes(&[c]);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].contract_id, "CWASMTOKEN");
        assert_eq!(
            writes[0].metadata.name.as_deref(),
            Some("liquidFi bridge token")
        );
        assert_eq!(writes[0].metadata.symbol.as_deref(), Some("lUSDC"));
        assert_eq!(writes[0].metadata.decimals, Some(7));
        assert_eq!(writes[0].ledger, 100);
    }

    #[test]
    fn extract_metadata_writes_skips_sac() {
        // SAC METADATA (name = CODE:ISSUER) is redundant with SAC identity → skip.
        let c = make_instance_meta_change(
            "CSAC",
            "created",
            "stellar_asset",
            crate::token_metadata::TokenMetadata {
                name: Some("USDC:GISSUER".into()),
                symbol: Some("USDC".into()),
                decimals: Some(7),
            },
        );
        assert!(extract_contract_metadata_writes(&[c]).is_empty());
    }

    #[test]
    fn extract_metadata_writes_includes_restored() {
        // A contract instance restored from archival re-materializes the
        // current value — its METADATA must be (re)written (closes the
        // cold-start-after-eviction hole; task 0297 review).
        let c = make_instance_meta_change(
            "CRESTORED",
            "restored",
            "wasm",
            crate::token_metadata::TokenMetadata {
                name: Some("Restored Token".into()),
                symbol: Some("RST".into()),
                decimals: Some(7),
            },
        );
        let w = extract_contract_metadata_writes(&[c]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].metadata.name.as_deref(), Some("Restored Token"));
    }

    #[test]
    fn extract_metadata_writes_skips_state_preimage() {
        let c = make_instance_meta_change(
            "CWASM2",
            "state",
            "wasm",
            crate::token_metadata::TokenMetadata {
                name: Some("old".into()),
                symbol: None,
                decimals: None,
            },
        );
        assert!(extract_contract_metadata_writes(&[c]).is_empty());
    }

    #[test]
    fn extract_metadata_writes_includes_updated_late_init() {
        let c = make_instance_meta_change(
            "CLATE",
            "updated",
            "wasm",
            crate::token_metadata::TokenMetadata {
                name: Some("Late Init Token".into()),
                symbol: Some("LI".into()),
                decimals: Some(6),
            },
        );
        let w = extract_contract_metadata_writes(&[c]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].metadata.name.as_deref(), Some("Late Init Token"));
        assert_eq!(w[0].metadata.decimals, Some(6));
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
    fn skip_state_only_account() {
        // `state` is a read-only pre-image snapshot; account state is derived
        // only from created/updated/restored (plus the removed tombstone
        // below). A lone `state` change yields nothing.
        let changes = vec![make_change(
            "account",
            "state",
            json!({}),
            Some(json!({"account_id": "G1", "balance": 0, "seq_num": 0})),
        )];

        assert!(extract_account_states(&changes).is_empty());
    }

    #[test]
    fn removed_account_emits_zero_native_tombstone() {
        // AccountMerge: a `removed` account ⇒ native balance=0 tombstone at the
        // merge ledger, account_id from the key (removed carries no data).
        let changes = vec![make_change(
            "account",
            "removed",
            json!({ "account_id": "GMERGED" }),
            None,
        )];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        let a = &accounts[0];
        assert_eq!(a.account_id, "GMERGED");
        assert_eq!(a.last_seen_ledger, 100); // merge ledger
        assert!(a.first_seen_ledger.is_none()); // not a creation
        assert_eq!(a.sequence_number, -1); // no seq on removal — must not clobber
        assert_eq!(a.balances[0]["asset_type"], "native");
        assert_eq!(a.balances[0]["balance"], "0.0000000");
        // The 0 alone cannot say "this account is gone" — an account holding no
        // XLM writes the same value and is very much alive (CAP-0033 sponsored
        // reserves). ADR 0055.
        assert!(a.account_removed, "the merge must be marked as a closure");
    }

    #[test]
    fn live_account_holding_no_xlm_is_not_marked_removed() {
        // The counter-case that makes the flag worth having: balance 0, account
        // alive. Measured at 4.27M zero-native rows in production, of which
        // 239,087 sit alongside a positive non-native balance.
        let changes = vec![make_change(
            "account",
            "updated",
            json!({ "account_id": "GPOOR" }),
            Some(json!({
                "account_id": "GPOOR",
                "balance": 0,
                "seq_num": 7,
                "home_domain": "",
            })),
        )];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts[0].balances[0]["balance"], "0.0000000");
        assert!(
            !accounts[0].account_removed,
            "a live account at zero XLM must not read as merged"
        );
    }

    /// lore-0463: signers/thresholds/flags flow through the accumulator with
    /// full-set semantics — Some(empty) is a real state, None means the entry
    /// was never observed.
    #[test]
    fn signers_flow_through_with_full_set_semantics() {
        let changes = vec![make_change(
            "account",
            "updated",
            json!({ "account_id": "GMULTI" }),
            Some(json!({
                "account_id": "GMULTI",
                "balance": 100,
                "seq_num": 5,
                "home_domain": "",
                "thresholds": "01030303",
                "flags": 4,
                "signers": [
                    {"key": "GS1", "weight": 1, "type": "ed25519"},
                    {"key": "TS2", "weight": 2, "type": "preauth_tx"},
                ],
            })),
        )];
        let a = &extract_account_states(&changes)[0];
        assert_eq!(a.thresholds.as_deref(), Some("01030303"));
        assert_eq!(a.flags, Some(4));
        let sg = a.signers.as_ref().expect("entry observed => Some");
        assert_eq!(sg.len(), 2);
        assert_eq!(sg[1]["type"], "preauth_tx");
    }

    #[test]
    fn entry_without_signers_field_yields_some_empty_not_none() {
        // Removing the last signer emits an entry whose set is empty — that
        // MUST surface as Some(empty), or persist would skip the write and
        // the stale set would survive in the RMT forever.
        let changes = vec![make_change(
            "account",
            "updated",
            json!({ "account_id": "GBARE" }),
            Some(json!({
                "account_id": "GBARE",
                "balance": 1,
                "seq_num": 1,
                "home_domain": "",
                "thresholds": "01000000",
                "flags": 0,
            })),
        )];
        let a = &extract_account_states(&changes)[0];
        assert_eq!(a.signers.as_deref(), Some(&[][..]));
        assert_eq!(a.thresholds.as_deref(), Some("01000000"));
    }

    #[test]
    fn trustline_only_change_never_observes_signers() {
        let changes = vec![make_change(
            "trustline",
            "created",
            json!({ "account_id": "GTL" }),
            Some(json!({
                "account_id": "GTL",
                "asset": {"type": "credit_alphanum4", "code": "AQUA", "issuer": "GISS"},
                "balance": 5,
                "limit": 100,
                "flags": 1,
            })),
        )];
        let a = &extract_account_states(&changes)[0];
        assert!(
            a.signers.is_none() && a.thresholds.is_none(),
            "a trustline-only accum must not fabricate an observed entry"
        );
    }

    #[test]
    fn merge_then_recreate_in_one_change_set_is_not_closed() {
        // Order matters: the removal is seen first, a live entry follows. The
        // account exists at the end of the ledger, so the closure must be
        // cancelled — otherwise the read path would hide a live account.
        let changes = vec![
            make_change(
                "account",
                "removed",
                json!({ "account_id": "GPHOENIX" }),
                None,
            ),
            make_change(
                "account",
                "created",
                json!({ "account_id": "GPHOENIX" }),
                Some(json!({
                    "account_id": "GPHOENIX",
                    "balance": 50_000_000,
                    "seq_num": 1,
                    "home_domain": "",
                })),
            ),
        ];

        let accounts = extract_account_states(&changes);
        assert_eq!(accounts.len(), 1);
        assert!(
            !accounts[0].account_removed,
            "recreated in the same change set — must not stay marked closed"
        );
        assert_eq!(accounts[0].balances[0]["balance"], "5.0000000");
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

    // -- Lore-0189: pool extraction includes `state` change_type --

    #[test]
    fn pool_extracted_from_state_change_type() {
        // Lore-0189 reproducer: ledger 62148003 contained pool d63184... only
        // as a `state` snapshot (no reserves change), but a pool_share trustline
        // for that pool was simultaneously `removed` — producing an `lp_positions`
        // emit with a pool_id that, pre-fix, was not present in `pool_rows`.
        // Post-fix (lore-0189), `state` is included so the pool dimension is
        // captured. lore-0356: `state` also emits a snapshot — for this
        // referenced-but-not-mutated pool it is the pool's correct end-of-ledger
        // value, so the pool keeps exactly one snapshot instead of blank reserves.
        let changes = vec![make_change(
            "liquidity_pool",
            "state",
            json!({
                "pool_id": "d63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561",
            }),
            Some(json!({
                "pool_id": "d63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561",
                "params": {
                    "asset_a": { "type": "credit_alphanum4", "code": "Lira", "issuer": "GBU3EGQO" },
                    "asset_b": { "type": "credit_alphanum12", "code": "liragold", "issuer": "GAIHDHWF" },
                    "fee": 30,
                },
                "reserve_a": 0,
                "reserve_b": 0,
                "total_pool_shares": 0,
                "type": "constant_product",
            })),
        )];

        let (pools, snapshots) = extract_liquidity_pools(&changes);
        assert_eq!(pools.len(), 1, "state change_type must produce 1 pool row");
        assert_eq!(
            snapshots.len(),
            1,
            "lore-0356: state-only pool keeps one snapshot (its correct reserves)"
        );
        assert_eq!(snapshots[0].reserves, json!({ "a": 0, "b": 0 }));

        let pool = &pools[0];
        assert_eq!(
            pool.pool_id,
            "d63184d4e5601fad174d9d5fa8e79f2366f6818892e43867a952e8adb13fa561"
        );
        assert_eq!(pool.fee_bps, 30);
        // `state` is not creation — created_at_ledger must remain None.
        assert!(
            pool.created_at_ledger.is_none(),
            "state must NOT mark as creation"
        );
        assert_eq!(pool.last_updated_ledger, 100);
    }

    #[test]
    fn pool_state_only_does_not_promote_to_creation() {
        // Reinforces the contract: `state` is observed-not-created. If we
        // ever start treating it as a creation, downstream logic that
        // depends on `created_at_ledger` (e.g. earliest-observation
        // analytics) would silently regress.
        let changes = vec![make_change(
            "liquidity_pool",
            "state",
            json!({"pool_id": "aabb"}),
            Some(json!({
                "pool_id": "aabb",
                "params": {
                    "asset_a": { "type": "native" },
                    "asset_b": { "type": "credit_alphanum4", "code": "USDC", "issuer": "GIS" },
                    "fee": 30,
                },
                "reserve_a": 100_i64,
                "reserve_b": 50_i64,
                "total_pool_shares": 70_i64,
                "type": "constant_product",
            })),
        )];

        let (pools, _) = extract_liquidity_pools(&changes);
        assert_eq!(pools.len(), 1);
        assert!(pools[0].created_at_ledger.is_none());
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
            upgradeable: false,
        }
    }

    #[test]
    fn sac_credit_deployment_produces_classic_credit_with_sac_facet() {
        // ADR 0051: a SAC credit deploy folds onto the classic_credit row —
        // the SAC handle rides in `sac_contract_id` (+ `sac_deployed = true`),
        // NOT a separate `asset_type`; the key `contract_id` stays unset.
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CSAC456".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            sac_asset: Some(SacAssetIdentity::Credit {
                code: "USDC".into(),
                issuer: "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN".into(),
            }),
        }];

        let assets = detect_assets(&deployments, &[]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetFamily::ClassicCredit);
        assert_eq!(assets[0].contract_id, None);
        assert_eq!(assets[0].sac_contract_id.as_deref(), Some("CSAC456"));
        assert!(assets[0].sac_deployed);
        // Task 0160 regression: SAC identity must survive through to the asset row.
        assert_eq!(assets[0].asset_code.as_deref(), Some("USDC"));
        assert_eq!(
            assets[0].issuer_address.as_deref(),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
        );
    }

    #[test]
    fn sac_native_deployment_produces_native_with_sac_facet() {
        // ADR 0051: a SAC deploy wrapping native XLM folds onto the native
        // (type=0) row — NULL code/issuer, SAC handle in `sac_contract_id`.
        let deployments = vec![ExtractedContractDeployment {
            contract_id: "CXLM_SAC".into(),
            wasm_hash: None,
            deployer_account: None,
            deployed_at_ledger: 100,
            contract_type: ContractType::Token,
            is_sac: true,
            sac_asset: Some(SacAssetIdentity::Native),
        }];

        let assets = detect_assets(&deployments, &[]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetFamily::Native);
        assert_eq!(assets[0].contract_id, None);
        assert_eq!(assets[0].sac_contract_id.as_deref(), Some("CXLM_SAC"));
        assert!(assets[0].sac_deployed);
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
            sac_asset: None,
        }];
        let interfaces = vec![iface(
            &wasm,
            &["transfer", "balance", "decimals", "name", "symbol"],
        )];

        let assets = detect_assets(&deployments, &interfaces);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetFamily::Soroban);
        assert_eq!(assets[0].contract_id.as_deref(), Some("CFUN001"));
        assert!(assets[0].asset_code.is_none());
        assert!(assets[0].issuer_address.is_none());
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
                sac_asset: None,
            },
        ];
        let interfaces = vec![iface(&wasm, &["transfer", "decimals", "allowance"])];

        let assets = detect_assets(&deployments, &interfaces);
        assert_eq!(assets.len(), 2);
        // The SAC folds onto a classic_credit carrier (handle in `sac_contract_id`,
        // key `contract_id` unset); the bespoke fungible is a soroban row keyed by
        // its own `contract_id`.
        let sac = assets
            .iter()
            .find(|a| a.sac_contract_id.as_deref() == Some("CSAC005"))
            .expect("SAC carrier present");
        assert_eq!(sac.asset_type, AssetFamily::ClassicCredit);
        assert_eq!(sac.contract_id, None);
        assert!(sac.sac_deployed);
        let fungible = assets
            .iter()
            .find(|a| a.contract_id.as_deref() == Some("CFUN006"))
            .expect("soroban fungible present");
        assert_eq!(fungible.asset_type, AssetFamily::Soroban);
        assert_eq!(fungible.sac_contract_id, None);
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

    // -- NFT Ownership Event Extraction Tests (task 0202) --

    fn make_nft_event(
        contract: &str,
        kind: &str,
        token: i64,
        from: Option<&str>,
        to: Option<&str>,
        ledger: u32,
    ) -> NftEvent {
        NftEvent {
            transaction_hash: format!("tx{}", ledger),
            contract_id: contract.into(),
            event_kind: kind.into(),
            token_id: json!({"type": "u32", "value": token}),
            from: from.map(Into::into),
            to: to.map(Into::into),
            ledger_sequence: ledger,
            created_at: 1700000000 + ledger as i64,
        }
    }

    #[test]
    fn mint_event_yields_owner_to() {
        let events = vec![make_nft_event(
            "CNFT1",
            "mint",
            42,
            None,
            Some("GRECIPIENT"),
            100,
        )];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].contract_id, "CNFT1");
        assert_eq!(out[0].token_id, "42");
        assert_eq!(out[0].event_type, NftEventType::Mint);
        assert_eq!(out[0].owner_account.as_deref(), Some("GRECIPIENT"));
        assert_eq!(out[0].event_order, 0);
        assert_eq!(out[0].ledger_sequence, 100);
    }

    #[test]
    fn transfer_event_yields_owner_to() {
        let events = vec![make_nft_event(
            "CNFT1",
            "transfer",
            42,
            Some("GFROM"),
            Some("GTO"),
            100,
        )];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event_type, NftEventType::Transfer);
        assert_eq!(out[0].owner_account.as_deref(), Some("GTO"));
    }

    #[test]
    fn burn_event_yields_owner_none() {
        let events = vec![make_nft_event(
            "CNFT1",
            "burn",
            42,
            Some("GBURNER"),
            None,
            100,
        )];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event_type, NftEventType::Burn);
        assert!(out[0].owner_account.is_none());
    }

    #[test]
    fn event_order_monotonic_per_triple() {
        let events = vec![
            make_nft_event("CNFT1", "mint", 42, None, Some("GA"), 100),
            make_nft_event("CNFT1", "transfer", 42, Some("GA"), Some("GB"), 100),
            make_nft_event("CNFT1", "transfer", 42, Some("GB"), Some("GC"), 100),
        ];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].event_order, 0);
        assert_eq!(out[1].event_order, 1);
        assert_eq!(out[2].event_order, 2);
    }

    #[test]
    fn event_order_resets_per_token() {
        let events = vec![
            // Same contract, same ledger, different tokens.
            make_nft_event("CNFT1", "mint", 42, None, Some("GA"), 100),
            make_nft_event("CNFT1", "mint", 43, None, Some("GB"), 100),
            // Different contract, same ledger.
            make_nft_event("CNFT2", "mint", 42, None, Some("GC"), 100),
        ];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 3);
        // Each (contract, token, ledger) triple starts its own counter.
        assert_eq!(out[0].event_order, 0);
        assert_eq!(out[1].event_order, 0);
        assert_eq!(out[2].event_order, 0);
    }

    #[test]
    fn token_id_jsonvalue_stringified() {
        // Numeric token_id → "42".
        let numeric = NftEvent {
            transaction_hash: "tx1".into(),
            contract_id: "CNFT1".into(),
            event_kind: "mint".into(),
            token_id: json!({"type": "u64", "value": 42}),
            from: None,
            to: Some("GA".into()),
            ledger_sequence: 100,
            created_at: 1700000000,
        };
        // String token_id → "uuid-abc".
        let string = NftEvent {
            transaction_hash: "tx2".into(),
            contract_id: "CNFT1".into(),
            event_kind: "mint".into(),
            token_id: json!({"type": "string", "value": "uuid-abc"}),
            from: None,
            to: Some("GB".into()),
            ledger_sequence: 100,
            created_at: 1700000000,
        };

        let out = extract_nft_ownership_events(&[numeric, string]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].token_id, "42");
        assert_eq!(out[1].token_id, "uuid-abc");
    }

    #[test]
    fn empty_token_id_event_skipped() {
        // `token_id_to_string` returns "" when the JSON value is null
        // (e.g. type=void). Such events must be skipped so they never
        // reach staging — matches `detect_nfts` behaviour.
        let events = vec![NftEvent {
            transaction_hash: "tx1".into(),
            contract_id: "CNFT1".into(),
            event_kind: "mint".into(),
            token_id: json!({"type": "void", "value": null}),
            from: None,
            to: Some("GA".into()),
            ledger_sequence: 100,
            created_at: 1700000000,
        }];

        let out = extract_nft_ownership_events(&events);
        assert!(out.is_empty(), "event with empty token_id must be skipped");
    }

    #[test]
    fn unknown_event_kind_skipped() {
        // Parser is supposed to emit only mint/transfer/burn; anything
        // else is a defence-in-depth skip path. Mixed batch must keep
        // the recognised events and drop the unknown one.
        let events = vec![
            make_nft_event("CNFT1", "approve", 42, Some("GA"), Some("GB"), 100),
            make_nft_event("CNFT1", "mint", 43, None, Some("GA"), 100),
        ];
        let out = extract_nft_ownership_events(&events);

        assert_eq!(out.len(), 1, "unknown event_kind must be skipped");
        assert_eq!(out[0].event_type, NftEventType::Mint);
        assert_eq!(out[0].token_id, "43");
    }

    #[test]
    fn event_order_overflow_skips_excess_events() {
        // Pathological-input guard: once a (contract, token, ledger)
        // triple has emitted i16::MAX events, further events for that
        // triple are skipped with a warn rather than overflowing the
        // SMALLINT column at staging.
        const OVERFLOW_AT: u16 = i16::MAX as u16;

        let mut events = Vec::with_capacity((OVERFLOW_AT as usize) + 5);
        for _ in 0..(OVERFLOW_AT as usize + 5) {
            events.push(make_nft_event(
                "CNFT1",
                "transfer",
                42,
                Some("GA"),
                Some("GB"),
                100,
            ));
        }
        let out = extract_nft_ownership_events(&events);

        // Emits exactly i16::MAX + 1 rows (event_order 0..=32_767),
        // then refuses to write more — five excess events dropped.
        assert_eq!(
            out.len(),
            OVERFLOW_AT as usize + 1,
            "should emit one row per slot 0..=i16::MAX, no overflow"
        );
        assert_eq!(out.first().unwrap().event_order, 0);
        assert_eq!(out.last().unwrap().event_order, i16::MAX as u16);
    }

    // ----------------------------------------------------------------------
    // Task 0219 — detect_classic_credit_assets + native_asset_singleton
    // ----------------------------------------------------------------------

    fn trustline_change(change_type: &str, code: &str, issuer: &str) -> ExtractedLedgerEntryChange {
        make_change(
            "trustline",
            change_type,
            json!({
                "account_id": "GHOLDER",
                "asset": { "type": "credit_alphanum4", "code": code, "issuer": issuer },
            }),
            Some(json!({
                "account_id": "GHOLDER",
                "asset": { "type": "credit_alphanum4", "code": code, "issuer": issuer },
                "balance": 10_000_000,
                "limit": 1_000_000_000,
            })),
        )
    }

    #[test]
    fn classic_credit_assets_emitted_from_trustline_created() {
        let changes = vec![trustline_change(
            "created",
            "USDC",
            "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
        )];
        let assets = detect_classic_credit_assets(&changes);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetFamily::ClassicCredit);
        assert_eq!(assets[0].asset_code.as_deref(), Some("USDC"));
        assert_eq!(
            assets[0].issuer_address.as_deref(),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
        );
        assert!(assets[0].contract_id.is_none());
    }

    #[test]
    fn classic_credit_emitted_for_updated_restored_state_changes() {
        let issuer = "GISSUER";
        let changes = vec![
            trustline_change("created", "AQUA", issuer),
            trustline_change("updated", "AQUA", issuer),
            trustline_change("restored", "AQUA", issuer),
            trustline_change("state", "AQUA", issuer),
        ];
        let assets = detect_classic_credit_assets(&changes);
        // Same (code, issuer) across 4 change types → 1 row after dedup.
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_code.as_deref(), Some("AQUA"));
    }

    #[test]
    fn classic_credit_removed_trustlines_use_key_asset_fallback() {
        // Removed trustlines carry `data: None`, but the change's
        // `key.asset` still holds `{type, code, issuer}`. A
        // partial-window backfill whose first observation of a
        // `(code, issuer)` pair is the trustline removal should still
        // emit the asset row — falling back to `key` covers that case.
        let mut change = trustline_change(
            "removed",
            "USDC",
            "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
        );
        change.data = None;
        let assets = detect_classic_credit_assets(&[change]);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_code.as_deref(), Some("USDC"));
        assert_eq!(
            assets[0].issuer_address.as_deref(),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
        );
    }

    #[test]
    fn classic_credit_removed_trustline_without_key_asset_is_skipped() {
        // Truly key-less removed change (key carries account_id only) —
        // we have no asset identity to emit; skip safely.
        let mut change = trustline_change("removed", "USDC", "GISSUER");
        change.data = None;
        change.key = json!({"account_id": "GHOLDER"}); // no `asset` field
        let assets = detect_classic_credit_assets(&[change]);
        assert!(assets.is_empty());
    }

    #[test]
    fn classic_credit_dedups_same_code_issuer_across_changes() {
        let issuer = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
        let changes = vec![
            trustline_change("created", "USDC", issuer),
            trustline_change("updated", "USDC", issuer),
            trustline_change("created", "EURC", issuer),
        ];
        let assets = detect_classic_credit_assets(&changes);
        // USDC dedups across two changes; EURC stands alone → 2 rows.
        assert_eq!(assets.len(), 2);
        let mut codes: Vec<_> = assets
            .iter()
            .map(|a| a.asset_code.as_deref().unwrap_or(""))
            .collect();
        codes.sort();
        assert_eq!(codes, vec!["EURC", "USDC"]);
    }

    #[test]
    fn classic_credit_skips_pool_share_trustlines() {
        let change = make_change(
            "trustline",
            "created",
            json!({"account_id": "GHOLDER", "asset": {"type": "pool_share"}}),
            Some(json!({
                "account_id": "GHOLDER",
                "asset": { "type": "pool_share" },
                "balance": 1_000,
            })),
        );
        let assets = detect_classic_credit_assets(&[change]);
        assert!(
            assets.is_empty(),
            "pool_share trustlines belong to extract_lp_positions, not asset rows"
        );
    }

    #[test]
    fn classic_credit_skips_non_trustline_entries() {
        let change = make_change(
            "account",
            "created",
            json!({"account_id": "GACCOUNT"}),
            Some(json!({"account_id": "GACCOUNT", "balance": 10_000_000})),
        );
        let assets = detect_classic_credit_assets(&[change]);
        assert!(assets.is_empty());
    }

    #[test]
    fn classic_credit_skips_malformed_asset_object() {
        // Asset object missing `code` or `issuer` should be skipped rather
        // than producing a partial-identity row.
        let no_code = make_change(
            "trustline",
            "created",
            json!({"account_id": "GHOLDER", "asset": {"type": "credit_alphanum4"}}),
            Some(json!({
                "account_id": "GHOLDER",
                "asset": { "type": "credit_alphanum4", "issuer": "GISSUER" },
                "balance": 0,
            })),
        );
        let no_issuer = make_change(
            "trustline",
            "created",
            json!({"account_id": "GHOLDER", "asset": {"type": "credit_alphanum4"}}),
            Some(json!({
                "account_id": "GHOLDER",
                "asset": { "type": "credit_alphanum4", "code": "USDC" },
                "balance": 0,
            })),
        );
        let assets = detect_classic_credit_assets(&[no_code, no_issuer]);
        assert!(assets.is_empty());
    }

    #[test]
    fn native_singleton_returns_native_asset_no_identity() {
        let asset = native_asset_singleton();
        assert_eq!(asset.asset_type, AssetFamily::Native);
        assert!(asset.asset_code.is_none());
        assert!(asset.issuer_address.is_none());
        assert!(asset.contract_id.is_none());
    }
}
