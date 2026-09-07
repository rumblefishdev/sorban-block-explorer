//! Pool state read from ledger entries (task 0374, step 7).
//!
//! Two `ContractData` shapes carry a pool's state, and BOTH are recognised by
//! shape, never by address (planes are per-deployment — router B writes to
//! its own):
//!
//! 1. **Plane `PoolData`** — the deployment's scoreboard, one entry per pool,
//!    updated on every pool action. Key `[Symbol("PoolData"), Address(pool)]`,
//!    value a map of `reserves` (vec of u128), `pool_type` (symbol) and
//!    `init_args`. This is THE reserve source for the whole timeline (T4;
//!    event arithmetic failed its oracle 6/49, the plane piloted 80/80).
//! 2. **Pool instance storage** — written in the SAME transaction as
//!    `add_pool`, and on later config changes. Carries `TokenShare` (the
//!    share token, as state — the fundamental source that demoted the
//!    deposit⇄mint rule to a cross-check), `Plane`, `Router`, the token list
//!    and mirror reserves. A concentrated pool has NO `TokenShare` key —
//!    structurally, matching `share_id()` returning the pool itself.
//!
//! Values stay RAW (u128 as strings, verbatim symbols): scaling and
//! vocabulary-folding are read-time concerns. The plane spells the constant
//! product type `standard` while the event spells it `constant` — the third
//! live vocabulary for one concept; nothing here folds them.

use serde_json::Value;

use crate::scval::{address, map_get, symbol, typed, typed_str};
use crate::types::ExtractedLedgerEntryChange;

/// One plane `PoolData` write: a pool's reserves (and registration facts) as
/// the deployment's scoreboard records them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanePoolData {
    /// The plane contract that owns the entry (the emitter of truth).
    pub plane: String,
    /// The pool the entry describes.
    pub pool: String,
    /// Raw u128 reserve amounts, in leg order.
    pub reserves: Vec<String>,
}

/// Decode a plane `PoolData` entry from a `ContractData` change: `owner` is
/// the plane contract, `key`/`val` are house typed-JSON ScVals
/// (`scval_to_typed_json` dialect: `{"type":"sym","value":…}`, map entries
/// `{"key":…,"value":…}`). `None` when the shape is not a plane entry — the
/// normal case for every other `ContractData` write on the chain.
pub fn parse_plane_pool_data(owner: &str, key: &Value, val: &Value) -> Option<PlanePoolData> {
    let parts = typed(key, "vec")?.as_array()?;
    if symbol(parts.first()?) != Some("PoolData") {
        return None;
    }
    let pool = address(parts.get(1)?)?.to_string();
    Some(PlanePoolData {
        plane: owner.to_string(),
        pool,
        reserves: raw_u128_vec(map_get(val, "reserves")?)?,
    })
}

/// The slice of a pool's instance storage this task consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolInstanceState {
    /// The pool contract whose instance this is.
    pub pool: String,
    /// `TokenShare` — the share token as state. `None` is STRUCTURAL for
    /// concentrated pools, which have no such key.
    pub token_share: Option<String>,
    /// `TotalShares` — the outstanding share supply, raw u128, same entry and
    /// same clock as the share token (write half of the "Total shares: —"
    /// gap, ranked item 1). `None` where the key is absent: structural for
    /// concentrated pools (nothing minted) and for the elastic layout.
    pub total_shares: Option<String>,
    /// The deployment's plane this pool reports to.
    pub plane: Option<String>,
    /// The registering router, as the pool itself records it.
    pub router: Option<String>,
    /// Per-operation reserves for a CONCENTRATED pool (`Reserve0`/`Reserve1`
    /// in instance storage). Concentrated pools do NOT update the plane per
    /// operation — measured on a hot ledger: 8 instance writes, zero plane
    /// writes for the busiest concentrated pool — so for them the instance IS
    /// the reserve stream, refining T4's claim (whose pilot predates the
    /// first concentrated pool). Empty for fungible pools: their `ReserveA/B`
    /// mirror is a corroborator only, the plane is their source, and staging
    /// both would duplicate snapshot rows.
    pub reserves: Vec<String>,
}

/// Decode the pool-relevant slice of an instance-storage list (house typed
/// JSON: `[{"key":{"type":"vec","value":[{"type":"sym",…}]},"value":…},…]`).
///
/// Returns `None` unless the instance IS a router-family pool — recognised by
/// shape: it carries a `Plane` key, the deployment scoreboard every pool of
/// this family reports to.
///
/// **`Router` is NOT required, and requiring it was wrong.** The earlier rule
/// ("carries both `Router` and `Plane`") was measured on live creations only
/// and generalised to the whole population. Read from chain 2026-09-01: five
/// of the ten deployments run an older contract whose instance has `Plane`,
/// `TokenShare`, `ReserveA/B` and no `Router` key at all — 23 real pools.
/// Nothing states a guarantee either way: Stellar's docs say nothing about
/// trusting event or entry contents, the vendor's docs describe the roles
/// without promising the key, and the vendor's source is unreachable (404,
/// re-checked 2026-09-01). So a missing `Router` is an observed fact about an
/// older contract version, not evidence of a forgery, and must not be treated
/// as one.
///
/// Relaxing the shape test is safe because this decode is keyed on the entry's
/// OWNER: a foreign contract that writes a `Plane` key describes only itself,
/// and nothing reads an instance row for a pool that never reached the
/// registry.
pub fn parse_pool_instance(pool: &str, storage: &Value) -> Option<PoolInstanceState> {
    let entries = storage.as_array()?;
    let get = |name: &str| -> Option<&Value> {
        entries.iter().find_map(|kv| {
            let k = typed(kv.get("key")?, "vec")?.as_array()?;
            let is = k.len() == 1 && typed(&k[0], "sym").and_then(Value::as_str) == Some(name);
            is.then(|| kv.get("value"))?
        })
    };
    let plane = get("Plane")?;
    let router = get("Router");
    let addr = |v: &Value| address(v).map(str::to_string);
    let u128s = |v: &Value| typed_str(v, "u128").map(str::to_string);
    // Reserve0/Reserve1 is the CONCENTRATED layout; fungible pools use
    // ReserveA/ReserveB, deliberately not read here — the plane is their
    // source, and reading both would double-write snapshots.
    let reserves = match (
        get("Reserve0").and_then(u128s),
        get("Reserve1").and_then(u128s),
    ) {
        (Some(r0), Some(r1)) => vec![r0, r1],
        _ => Vec::new(),
    };
    Some(PoolInstanceState {
        pool: pool.to_string(),
        token_share: get("TokenShare").and_then(&addr),
        total_shares: get("TotalShares").and_then(u128s),
        plane: addr(plane),
        router: router.and_then(&addr),
        reserves,
    })
}

/// One plane `PoolData` write. Per-write coordinates (tx hash, change
/// index) were dropped with the grain collapse: rows are one-per-(pool,
/// ledger) and the last-in-apply-order pick needs only the vector order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPlanePoolData {
    pub data: PlanePoolData,
    pub ledger_sequence: u32,
}

/// Extract plane `PoolData` writes from a transaction's ledger-entry changes
/// (task 0374, step 7 — the reserve source, T4).
///
/// Mirrors `extract_soroban_token_balances`: only `created`/`updated`/
/// `restored` carry a value; the `state` pre-image is skipped (same-ledger
/// clobber) and `removed` has nothing to record.
pub fn extract_plane_pool_data(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedPlanePoolData> {
    let mut out = Vec::new();
    for change in changes {
        if change.entry_type != "contract_data" {
            continue;
        }
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored"
        ) {
            continue;
        }
        let Some(owner) = change.key.get("contract").and_then(Value::as_str) else {
            continue;
        };
        let (Some(key), Some(val)) = (
            change.key.get("key"),
            change.data.as_ref().and_then(|d| d.get("val")),
        ) else {
            continue;
        };
        if let Some(data) = parse_plane_pool_data(owner, key, val) {
            out.push(ExtractedPlanePoolData {
                data,
                ledger_sequence: change.ledger_sequence,
            });
        }
    }
    out
}

/// One pool-instance write (creation or config change) carrying the
/// state-sourced relations: share token, plane, router. Per-write
/// coordinates dropped with the grain collapse (see the plane twin above).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPoolInstance {
    pub state: PoolInstanceState,
    pub ledger_sequence: u32,
    /// The entry was CREATED in this ledger (vs updated/restored). Load-bearing
    /// for the router-less registration arm: a genuine registration deploys
    /// and initialises the pool in ONE transaction (497/497 measured), so its
    /// instance is always a creation — while an attacker inducing a same-ledger
    /// instance write on an existing victim pool can only produce an update.
    pub created: bool,
}

/// Extract router-family pool instances from ledger-entry changes.
///
/// The instance is written in the SAME transaction as `add_pool` (probed on
/// raw meta), so registration-time facts — `TokenShare`, `Plane` — arrive as
/// state, no deposit needed. Later instance rewrites (the 13 measured
/// share-token migrations) flow through the same arm and converge in the RMT
/// side table by ledger version.
pub fn extract_pool_instances(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedPoolInstance> {
    let mut out = Vec::new();
    for change in changes {
        if change.entry_type != "contract_data" {
            continue;
        }
        if !matches!(
            change.change_type.as_str(),
            "created" | "updated" | "restored"
        ) {
            continue;
        }
        // The instance entry's key is the `ledger_key_contract_instance`
        // sentinel; the storage rides inside the value's contract_instance.
        let key_is_instance = change
            .key
            .get("key")
            .and_then(|k| k.get("type"))
            .and_then(Value::as_str)
            == Some("ledger_key_contract_instance");
        if !key_is_instance {
            continue;
        }
        let Some(pool) = change.key.get("contract").and_then(Value::as_str) else {
            continue;
        };
        let Some(storage) = change
            .data
            .as_ref()
            .and_then(|d| d.get("val"))
            .and_then(|v| typed(v, "contract_instance"))
            .and_then(|ci| ci.get("storage"))
        else {
            continue;
        };
        if let Some(state) = parse_pool_instance(pool, storage) {
            out.push(ExtractedPoolInstance {
                state,
                ledger_sequence: change.ledger_sequence,
                created: change.change_type == "created",
            });
        }
    }
    out
}

/// `{"type":"vec","value":[{"type":"u128","value":"…"},…]}` → raw decimal
/// strings, order preserved.
fn raw_u128_vec(v: &Value) -> Option<Vec<String>> {
    typed(v, "vec")?
        .as_array()?
        .iter()
        .map(|e| typed_str(e, "u128").map(str::to_string))
        .collect()
}

// NO parser-side fold for either stream (decision karolkow 2026-09-01):
// plane writes and instance images both collapse in STAGING, one fold per
// destination table (`fold_pool_state_changes` / `fold_pool_instance_state`
// in db-clickhouse), symmetric and in one home. A parser-side pre-fold was
// a second copy with zero effect — staging order preserves ledger apply
// order end-to-end, so the stage folds see the same last-wins sequence.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verbatim from mainnet — plane `CCABO2IQ…`, registration ledger
    /// 63 893 403. Note the plane spells the type `standard`, not `constant`.
    #[test]
    fn decodes_a_real_plane_entry() {
        let key = json!({"type": "vec", "value": [
            {"type": "sym", "value": "PoolData"},
            {"type": "address", "value": "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH"}
        ]});
        let val = json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "init_args"}, "value": {"type": "vec", "value": [{"type": "u128", "value": "10"}]}},
            {"key": {"type": "sym", "value": "pool_type"}, "value": {"type": "sym", "value": "standard"}},
            {"key": {"type": "sym", "value": "reserves"}, "value": {"type": "vec", "value": [{"type": "u128", "value": "100000000000"}, {"type": "u128", "value": "30617317"}]}}
        ]});

        let got = parse_plane_pool_data(
            "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY",
            &key,
            &val,
        )
        .expect("real plane entry decodes");
        assert_eq!(
            got.pool,
            "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH"
        );
        assert_eq!(got.reserves, vec!["100000000000", "30617317"]);
    }

    /// Verbatim from the same ledger — a 5-init-arg stable pool entry still
    /// decodes (the extractor consumes only `reserves`; the other map keys
    /// must not confuse it).
    #[test]
    fn keeps_every_stable_init_arg() {
        let key = json!({"type": "vec", "value": [
            {"type": "sym", "value": "PoolData"},
            {"type": "address", "value": "CCNXGPE4AQCSNEBZO3XJDKKDI3CRLYMVS6UWBBTVDLALLWMJEXBORQ2A"}
        ]});
        let val = json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "init_args"}, "value": {"type": "vec", "value": [
                {"type": "u128", "value": "10"}, {"type": "u128", "value": "1500"}, {"type": "u128", "value": "1764349837"},
                {"type": "u128", "value": "1500"}, {"type": "u128", "value": "1764349837"}
            ]}},
            {"key": {"type": "sym", "value": "pool_type"}, "value": {"type": "sym", "value": "stable"}},
            {"key": {"type": "sym", "value": "reserves"}, "value": {"type": "vec", "value": [{"type": "u128", "value": "7419859054"}, {"type": "u128", "value": "9364494398"}]}}
        ]});
        let got = parse_plane_pool_data(
            "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY",
            &key,
            &val,
        )
        .unwrap();
        assert_eq!(got.reserves, vec!["7419859054", "9364494398"]);
    }

    #[test]
    fn other_contract_data_is_not_a_plane_entry() {
        let key = json!({"type": "vec", "value": [{"type": "sym", "value": "Balance"}, {"type": "address", "value": "GABC"}]});
        assert_eq!(parse_plane_pool_data("C", &key, &json!({})), None);
    }

    /// Verbatim slice of the real constant-pool instance at creation
    /// (ledger 63 893 403): TokenShare, Plane and Router all present.
    #[test]
    fn reads_token_share_from_a_real_instance() {
        let storage = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Plane"}]},
             "value": {"type": "address", "value": "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Router"}]},
             "value": {"type": "address", "value": "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "TokenShare"}]},
             "value": {"type": "address", "value": "CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "FeeFraction"}]}, "value": {"type": "u32", "value": 10}}
        ]);
        let got = parse_pool_instance(
            "CBMWU3574VFWNBNMNYAAH4OBT7DPB27URDW4BWIV7XAPQG6YYMJW2LSH",
            &storage,
        )
        .expect("family pool instance");
        assert_eq!(
            got.token_share.as_deref(),
            Some("CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6"),
            "the share token as STATE, at birth"
        );
        assert!(got.plane.is_some() && got.router.is_some());
    }

    /// A concentrated pool's instance (probed live at ledger 64 134 576) has
    /// no TokenShare key — the absence is structural and must come back as
    /// None, never as a fabricated value.
    #[test]
    fn concentrated_instance_has_no_share_token() {
        let storage = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Plane"}]}, "value": {"type": "address", "value": "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Router"}]}, "value": {"type": "address", "value": "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "TickSpacing"}]}, "value": {"type": "u32", "value": 60}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Slot0"}]}, "value": {"type": "u128", "value": "0"}}
        ]);
        let got = parse_pool_instance(
            "CC642QYWXXR2HUZDNJ6KYN5LV5JFPFPT4Q6YNKLZLYEFWZZZ5SJYLA5G",
            &storage,
        )
        .unwrap();
        assert_eq!(got.token_share, None);
    }

    /// Verbatim shape of an OLDER deployment's pool, read from chain
    /// 2026-09-01 (`CD2X3JY7…`, one of five such deployments): `Plane`,
    /// `TokenShare` and reserves, but NO `Router` key. These are real pools;
    /// requiring `Router` dropped 23 of them.
    #[test]
    fn an_older_pool_without_a_router_key_is_still_a_pool() {
        let storage = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Admin"}]},
             "value": {"type": "address", "value": "GAV5FBMKD2ZF4X2MGWDNQYUP7KFL7MRM6HZBY7HKQLB4BRHSCCX5J6VS"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Plane"}]},
             "value": {"type": "address", "value": "CDYX2OSS4XYZUT2LWWH2NXOQMEFF4JSARGSF3NEB7RM5VOMUHE3X2UN2"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "TokenShare"}]},
             "value": {"type": "address", "value": "CC5PU23MKXHUFJKGG5FAUG7MFZX2KMWXPNZP26DDYW76VCB26UWMPEI6"}},
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "TotalShares"}]}, "value": {"type": "u128", "value": "5"}}
        ]);
        let got = parse_pool_instance(
            "CD2X3JY7PWJBXUU6PB52K3O547L2NX35XUUKKKGYED3UF6FWFTV5NI3N",
            &storage,
        )
        .expect("an older pool is still a pool");
        assert_eq!(
            got.plane.as_deref(),
            Some("CDYX2OSS4XYZUT2LWWH2NXOQMEFF4JSARGSF3NEB7RM5VOMUHE3X2UN2"),
            "the plane is what makes reserve provenance checkable — it must survive"
        );
        assert_eq!(
            got.router, None,
            "absent, and reported as absent — never invented"
        );
        assert!(got.token_share.is_some());
        assert_eq!(
            got.total_shares.as_deref(),
            Some("5"),
            "TotalShares rides the same entry — raw u128, no scaling"
        );
    }

    /// An instance with no `Plane` is not of this family — the plane is the
    /// one key the shape test still rests on.
    #[test]
    fn an_instance_without_a_plane_is_not_a_pool() {
        let storage = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Router"}]},
             "value": {"type": "address", "value": "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK"}}
        ]);
        assert_eq!(parse_pool_instance("CNOPLANE", &storage), None);
    }

    /// A random contract's instance (no `Plane`) is not a pool.
    #[test]
    fn a_foreign_instance_is_rejected_by_shape() {
        let storage = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Admin"}]}, "value": {"type": "address", "value": "GABC"}}
        ]);
        assert_eq!(parse_pool_instance("CFOREIGN", &storage), None);
    }
}
