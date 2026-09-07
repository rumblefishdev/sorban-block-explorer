//! Config-factory pool discovery and state (task 0518, third adapter under
//! 0516 — the Phoenix-family shape).
//!
//! The family's mechanism: a FACTORY announces each pool with a
//! `("create", "liquidity_pool")` event whose data is the bare pool address
//! (nothing else — no tokens, no counter), and the POOL keeps its state in
//! per-key PERSISTENT entries it owns (read verbatim from mainnet, creation
//! ledger 64,030,567):
//!
//! - `Symbol("CONFIG")` → a map `{token_a, token_b, share_token,
//!   stake_contract, pool_type (u32), total_fee_bps (i64), …}` — written at
//!   creation and on admin config changes, NOT per operation.
//! - Bare-u32 `DataKey` discriminants: `0` = TotalShares (i128), `1`/`2` =
//!   ReserveA/ReserveB (i128), `3` = Admin. Reserves are rewritten per
//!   swap/provide/withdraw; TotalShares per provide/withdraw.
//! - The pool's contract instance itself carries NO storage — everything
//!   lives in the keyed persistent entries. SIX factory deployments share
//!   the event shape on mainnet (five dead early ones with 6 pools, then
//!   the documented factory with 14 — 20 pools total, all XYK,
//!   `pool_type` 0); discovery is shape-driven at the emitter level, the
//!   same rule that surfaced the pair family's dead factories. The stable
//!   variant shares the key names, so a future stable pool flows through
//!   unchanged with `pool_type` ≠ 0.
//!
//! Unlike the pair-factory family there is NO back-pointer: the pool does
//! not record its deploying factory anywhere. Registration corroboration is
//! therefore the created gate + the pool's own full CONFIG in the same
//! ledger — the entries are ledger-authenticated to the pool, so a forged
//! event can neither hijack an existing pool (its entries are not CREATED)
//! nor invent one without deploying a contract that genuinely has this
//! shape (which IS a member of the family, per 0516's shape-not-brand
//! rule). The third forgery shape — a second emitter co-claiming a GENUINE
//! pool inside its creation ledger, which no pool-side check can arbitrate
//! without a back-pointer — meets a TWO-STAGE gate at staging (review
//! #447): the emitter must also have recorded the pool in an address list
//! in storage the EMITTER owns (see [`extract_address_list_writes`] — an
//! event-only claim dies pointwise and the genuine registration survives),
//! and if more than one emitter corroborates, every registration for that
//! pool refuses loudly.
//!
//! Recognition of per-operation state (no CONFIG in a swap tx) rests on the
//! RESERVE PAIR: the same contract writing both `u32(1)` and `u32(2)` as
//! i128 in one transaction. One reserve without the other is refused as
//! malformed, mirroring the pair-factory rule; the corpus test measures the
//! sieve's false-positive rate against raw history.
//!
//! Values stay RAW (i128 as strings); scaling is a read-time concern.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::scval::{address, map_get, symbol, typed, typed_str};
use crate::types::{EventSource, ExtractedEvent, ExtractedLedgerEntryChange};

/// One pool registration, tied to the factory that emitted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPoolRegistration {
    /// Emitting factory contract StrKey (`C…`).
    pub factory: String,
    /// The announced pool contract StrKey (`C…`) — the event's whole
    /// payload. Everything else (legs, fee, share token) comes from the
    /// pool's own CONFIG, never from the event.
    pub pool: String,
    pub ledger_sequence: u32,
}

/// Why a claimed registration was not decodable. `NotRegistration` is the
/// silent everyday case; `Malformed` is a registration that would otherwise
/// go missing and must be warned about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPoolReject {
    NotRegistration,
    Malformed(&'static str),
}

/// Decode a `("create", "liquidity_pool")` registration from its topics and
/// data payloads. Both topics are plain Strings (the Phoenix-family
/// convention — 0517 arm 3) and the data is a bare address.
pub fn parse_pool_created(topics: &Value, data: &Value) -> Result<String, ConfigPoolReject> {
    use ConfigPoolReject as R;

    let topics = topics.as_array().ok_or(R::NotRegistration)?;
    if typed_str(topics.first().ok_or(R::NotRegistration)?, "string") != Some("create") {
        return Err(R::NotRegistration);
    }
    if typed_str(topics.get(1).ok_or(R::NotRegistration)?, "string") != Some("liquidity_pool") {
        return Err(R::NotRegistration);
    }
    // From here the event has claimed to be a registration; nothing may
    // fail quietly.
    address(data)
        .map(str::to_string)
        .ok_or(R::Malformed("data is not an address"))
}

/// Scan a ledger's events for config-factory registrations — shaped like
/// `detect_pair_registrations`: semantic decoding lives here, staging only
/// corroborates and maps to rows.
pub fn detect_config_pool_registrations(
    events: &[(String, Vec<ExtractedEvent>)],
) -> Vec<ConfigPoolRegistration> {
    let mut out = Vec::new();
    for (_tx, evs) in events {
        for ev in evs {
            // The diagnostic container carries copies of consensus events
            // AND events from FAILED transactions (task 0182); indexing it
            // would register pools whose registration never applied.
            if matches!(ev.source, EventSource::Diagnostic) {
                continue;
            }
            let Some(factory) = ev.contract_id.as_deref() else {
                continue;
            };
            match parse_pool_created(&ev.topics, &ev.data) {
                Ok(pool) => out.push(ConfigPoolRegistration {
                    factory: factory.to_string(),
                    pool,
                    ledger_sequence: ev.ledger_sequence,
                }),
                Err(ConfigPoolReject::NotRegistration) => {}
                Err(reason) => tracing::warn!(
                    ledger_sequence = ev.ledger_sequence,
                    factory = %factory,
                    ?reason,
                    "create/liquidity_pool claimed to be a registration and could not \
                     be read — a pool is missing"
                ),
            }
        }
    }
    out
}

/// The slice of a pool's `CONFIG` map this task consumes. `stake_contract`
/// and the slippage/spread knobs stay verbatim in the raw entry — extract
/// on demand, never copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    pub token_a: String,
    pub token_b: String,
    /// The LP token — a SEPARATE contract the pool deploys at construction
    /// (like the router family's TokenShare, unlike the pair-factory's
    /// self-token).
    pub share_token: String,
    /// The vendor's `PairType` discriminant, verbatim (0 = XYK on every
    /// mainnet pool to date; a stable pool would carry ≠ 0).
    pub pool_type: u32,
    /// Per-pool fee in bps (i64 on chain; mutable via `update_config`, so a
    /// registry row is a creation-time snapshot — same caveat as every
    /// family).
    pub total_fee_bps: i64,
}

/// Decode the pool-relevant slice of a `CONFIG` map value. `None` when the
/// map does not carry the family's field set — that is the sieve refusing a
/// foreign `CONFIG`, not an error.
pub fn parse_pool_config(val: &Value) -> Option<PoolConfig> {
    let addr_field = |name: &str| map_get(val, name).and_then(address).map(str::to_string);
    Some(PoolConfig {
        token_a: addr_field("token_a")?,
        token_b: addr_field("token_b")?,
        share_token: addr_field("share_token")?,
        pool_type: map_get(val, "pool_type")
            .and_then(|v| typed(v, "u32"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())?,
        total_fee_bps: map_get(val, "total_fee_bps")
            .and_then(|v| typed(v, "i64"))
            .and_then(Value::as_i64)?,
    })
}

/// What one transaction wrote to one config-factory pool's keyed entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPoolState {
    /// The pool contract whose entries these are — the authenticated owner.
    pub pool: String,
    /// The `CONFIG` map, when this transaction (re)wrote it — creation and
    /// admin config changes only, never per-operation.
    pub config: Option<PoolConfig>,
    /// ReserveA/ReserveB (`DataKey` 1/2), raw i128 — BOTH or neither, same
    /// rule as the pair-factory family.
    pub reserves: Option<(String, String)>,
    /// TotalShares (`DataKey` 0), raw i128, when written. The pool mirrors
    /// its share token's supply here on every provide/withdraw.
    pub total_shares: Option<String>,
}

/// One pool's keyed-entry writes from one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedConfigPool {
    pub state: ConfigPoolState,
    pub ledger_sequence: u32,
    /// The pool's contract INSTANCE was created in this transaction. The
    /// created gate for registrations: a genuine registration deploys and
    /// initialises the pool in one transaction, so its instance is always a
    /// creation, while an attacker touching an existing pool can only
    /// produce updates.
    pub created: bool,
}

/// Extract config-factory pool writes from one transaction's ledger-entry
/// changes: group the per-key persistent entries by owning contract, then
/// keep owners that wrote a decodable `CONFIG` and/or the full reserve
/// pair. Mirrors its siblings: only `created`/`updated`/`restored` carry a
/// value; the `state` pre-image and `removed` record nothing.
pub fn extract_config_pools(changes: &[ExtractedLedgerEntryChange]) -> Vec<ExtractedConfigPool> {
    #[derive(Default)]
    struct Acc {
        config: Option<PoolConfig>,
        reserve_a: Option<String>,
        reserve_b: Option<String>,
        total_shares: Option<String>,
        instance_created: bool,
        ledger_sequence: u32,
    }
    // BTreeMap for deterministic output order across re-parses.
    let mut by_pool: BTreeMap<&str, Acc> = BTreeMap::new();

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
        let Some(key) = change.key.get("key") else {
            continue;
        };
        // The created gate reads the instance entry (which this family
        // leaves storage-less) — everything else needs the value payload.
        if key
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t == "ledger_key_contract_instance")
        {
            if change.change_type == "created" {
                let acc = by_pool.entry(owner).or_default();
                acc.instance_created = true;
                acc.ledger_sequence = change.ledger_sequence;
            }
            continue;
        }
        let Some(val) = change.data.as_ref().and_then(|d| d.get("val")) else {
            continue;
        };
        let i128s = |v: &Value| typed_str(v, "i128").map(str::to_string);
        if symbol(key) == Some("CONFIG") {
            if let Some(config) = parse_pool_config(val) {
                let acc = by_pool.entry(owner).or_default();
                acc.config = Some(config);
                acc.ledger_sequence = change.ledger_sequence;
            }
        } else if let Some(n) = typed(key, "u32").and_then(Value::as_u64) {
            let Some(v) = i128s(val) else { continue };
            let acc = by_pool.entry(owner).or_default();
            match n {
                0 => acc.total_shares = Some(v),
                1 => acc.reserve_a = Some(v),
                2 => acc.reserve_b = Some(v),
                _ => continue,
            }
            acc.ledger_sequence = change.ledger_sequence;
        }
    }

    let mut out = Vec::new();
    for (pool, acc) in by_pool {
        // Both or neither: one reserve without the other is not a smaller
        // answer, it is a malformed one — but ONLY once the owner already
        // qualified as family (via CONFIG); a lone stray u32 entry on some
        // unrelated contract is just not ours.
        let reserves = match (acc.reserve_a, acc.reserve_b) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None) => None,
            _ if acc.config.is_some() => {
                tracing::error!(
                    pool = %pool,
                    ledger_sequence = acc.ledger_sequence,
                    "config-factory pool wrote HALF a reserve pair — refusing \
                     the write; a snapshot is missing"
                );
                continue;
            }
            _ => continue,
        };
        // The sieve: a decodable CONFIG proves the shape; without one, only
        // the full reserve pair does. Anything else is not family output.
        if acc.config.is_none() && reserves.is_none() {
            continue;
        }
        out.push(ExtractedConfigPool {
            state: ConfigPoolState {
                pool: pool.to_string(),
                config: acc.config,
                reserves,
                total_shares: acc.total_shares,
            },
            ledger_sequence: acc.ledger_sequence,
            created: acc.instance_created,
        });
    }
    out
}

/// One contract's write of an ADDRESS LIST into its own storage — the
/// membership-list side of the config-factory registration gate.
///
/// The genuine factory appends each new pool to a pools vector in its OWN
/// persistent storage in the creation transaction (read verbatim from
/// mainnet, ledger 64,030,567: key `u32(2)`, value a vec of pool
/// addresses). The sieve is deliberately KEY-AGNOSTIC — any
/// created/updated persistent entry whose value is a non-empty vec of
/// addresses — so a factory-wasm generation that moves the key still
/// corroborates; the stage e2e proves the shape on the family's whole
/// registration population.
///
/// What this anchors and what it does not: a registration event is
/// believed only when its EMITTER also recorded the named pool in storage
/// the emitter owns — an event-only forgery dies right there, pointwise.
/// It is NOT a discriminator against a determined attacker who writes the
/// pool's address into his own list too (addresses are just data): that
/// case leaves two corroborated claimants and falls to the staging
/// conflict rule, which refuses both loudly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAddressListWrite {
    /// The contract that owns the entry — ledger-authenticated.
    pub owner: String,
    /// The list's members (C…/G… strkeys), verbatim.
    pub members: Vec<String>,
    pub ledger_sequence: u32,
}

/// Extract address-list writes from one transaction's ledger-entry
/// changes. Same change-type filter as every sibling: only
/// `created`/`updated`/`restored` carry a post-image.
pub fn extract_address_list_writes(
    changes: &[ExtractedLedgerEntryChange],
) -> Vec<ExtractedAddressListWrite> {
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
        let Some(val) = change.data.as_ref().and_then(|d| d.get("val")) else {
            continue;
        };
        let Some(elems) = typed(val, "vec").and_then(Value::as_array) else {
            continue;
        };
        if elems.is_empty() {
            continue;
        }
        let members: Option<Vec<String>> = elems
            .iter()
            .map(|e| address(e).map(str::to_string))
            .collect();
        // ALL elements must be addresses — a mixed vec is some other
        // structure, not a membership list.
        let Some(members) = members else { continue };
        out.push(ExtractedAddressListWrite {
            owner: owner.to_string(),
            members,
            ledger_sequence: change.ledger_sequence,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verbatim from mainnet — the creation-ledger CONFIG of pool
    /// `CCPPPTDW…` (ledger 64,030,567). Kept exact so a payload change
    /// fails here rather than in production.
    fn real_config() -> Value {
        json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "fee_recipient"},
             "value": {"type": "address", "value": "GA2N5NZL2IGVH7FWGBHOZNXRZL2XFHLYIFXC5XGU2COUMQCK7MAJUXXV"}},
            {"key": {"type": "sym", "value": "max_allowed_slippage_bps"},
             "value": {"type": "i64", "value": 10000}},
            {"key": {"type": "sym", "value": "max_allowed_spread_bps"},
             "value": {"type": "i64", "value": 10000}},
            {"key": {"type": "sym", "value": "max_referral_bps"},
             "value": {"type": "i64", "value": 5000}},
            {"key": {"type": "sym", "value": "pool_type"},
             "value": {"type": "u32", "value": 0}},
            {"key": {"type": "sym", "value": "share_token"},
             "value": {"type": "address", "value": "CA3KLIRAM6BKPN6BPPKTDX3CSY2DSM4YZAX54KZLER25X2QRK3FGDXR6"}},
            {"key": {"type": "sym", "value": "stake_contract"},
             "value": {"type": "address", "value": "CBUS3GWDBJYOLYC7PA3JCFQPPHXCX7U3TBHQSVCSTSNG6GH6WU4L74UE"}},
            {"key": {"type": "sym", "value": "token_a"},
             "value": {"type": "address", "value": "CBZ7M5B3Y4WWBZ5XK5UZCAFOEZ23KSSZXYECYX3IXM6E2JOLQC52DK32"}},
            {"key": {"type": "sym", "value": "token_b"},
             "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}},
            {"key": {"type": "sym", "value": "total_fee_bps"},
             "value": {"type": "i64", "value": 50}}
        ]})
    }

    #[test]
    fn decodes_a_real_config() {
        let got = parse_pool_config(&real_config()).expect("a real CONFIG decodes");
        assert_eq!(
            got.token_a,
            "CBZ7M5B3Y4WWBZ5XK5UZCAFOEZ23KSSZXYECYX3IXM6E2JOLQC52DK32"
        );
        assert_eq!(
            got.token_b,
            "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"
        );
        assert_eq!(
            got.share_token,
            "CA3KLIRAM6BKPN6BPPKTDX3CSY2DSM4YZAX54KZLER25X2QRK3FGDXR6"
        );
        assert_eq!(got.pool_type, 0);
        assert_eq!(got.total_fee_bps, 50);
    }

    #[test]
    fn foreign_config_map_is_not_family() {
        // A different protocol's CONFIG-named map without the field set.
        let val = json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "owner"},
             "value": {"type": "address", "value": "GA2N5NZL2IGVH7FWGBHOZNXRZL2XFHLYIFXC5XGU2COUMQCK7MAJUXXV"}}
        ]});
        assert_eq!(parse_pool_config(&val), None);
    }

    #[test]
    fn decodes_a_real_registration_event() {
        // Verbatim from prod `soroban_events` (ledger 64,030,567).
        let topics = json!([
            {"type": "string", "value": "create"},
            {"type": "string", "value": "liquidity_pool"}
        ]);
        let data = json!({"type": "address",
            "value": "CCPPPTDWJIWXQUQ2CN64S5JYQ7GYWVZIT7YWUUTH75HKIZX53Z2CE3XI"});
        assert_eq!(
            parse_pool_created(&topics, &data).expect("a real registration decodes"),
            "CCPPPTDWJIWXQUQ2CN64S5JYQ7GYWVZIT7YWUUTH75HKIZX53Z2CE3XI"
        );
    }

    #[test]
    fn registration_with_non_address_data_is_malformed_not_silent() {
        let topics = json!([
            {"type": "string", "value": "create"},
            {"type": "string", "value": "liquidity_pool"}
        ]);
        let data = json!({"type": "u32", "value": 7});
        assert_eq!(
            parse_pool_created(&topics, &data),
            Err(ConfigPoolReject::Malformed("data is not an address"))
        );
    }

    fn change(
        owner: &str,
        key: Value,
        val: Value,
        change_type: &str,
    ) -> ExtractedLedgerEntryChange {
        ExtractedLedgerEntryChange {
            transaction_hash: "00".repeat(32),
            change_type: change_type.into(),
            entry_type: "contract_data".into(),
            key: json!({"contract": owner, "key": key}),
            data: Some(json!({"val": val})),
            change_index: 0,
            operation_index: Some(0),
            ledger_sequence: 64_030_567,
            created_at: 0,
            token_metadata: None,
        }
    }

    const POOL: &str = "CCPPPTDWJIWXQUQ2CN64S5JYQ7GYWVZIT7YWUUTH75HKIZX53Z2CE3XI";

    #[test]
    fn creation_tx_groups_config_reserves_shares_and_created_gate() {
        let changes = vec![
            change(
                POOL,
                json!({"type": "ledger_key_contract_instance"}),
                json!({"type": "contract_instance", "value": {"storage": null}}),
                "created",
            ),
            change(
                POOL,
                json!({"type": "sym", "value": "CONFIG"}),
                real_config(),
                "created",
            ),
            change(
                POOL,
                json!({"type": "u32", "value": 0}),
                json!({"type": "i128", "value": "0"}),
                "created",
            ),
            change(
                POOL,
                json!({"type": "u32", "value": 1}),
                json!({"type": "i128", "value": "0"}),
                "created",
            ),
            change(
                POOL,
                json!({"type": "u32", "value": 2}),
                json!({"type": "i128", "value": "0"}),
                "created",
            ),
        ];
        let got = extract_config_pools(&changes);
        assert_eq!(got.len(), 1);
        let p = &got[0];
        assert_eq!(p.state.pool, POOL);
        assert!(p.created, "instance creation must arm the created gate");
        assert!(p.state.config.is_some());
        assert_eq!(
            p.state.reserves,
            Some(("0".to_string(), "0".to_string())),
            "creation writes a TRUE zero reserve pair"
        );
        assert_eq!(p.state.total_shares.as_deref(), Some("0"));
    }

    #[test]
    fn swap_tx_reserve_pair_without_config() {
        let changes = vec![
            change(
                POOL,
                json!({"type": "u32", "value": 1}),
                json!({"type": "i128", "value": "123456"}),
                "updated",
            ),
            change(
                POOL,
                json!({"type": "u32", "value": 2}),
                json!({"type": "i128", "value": "654321"}),
                "updated",
            ),
        ];
        let got = extract_config_pools(&changes);
        assert_eq!(got.len(), 1);
        assert!(!got[0].created);
        assert_eq!(got[0].state.config, None);
        assert_eq!(
            got[0].state.reserves,
            Some(("123456".to_string(), "654321".to_string()))
        );
    }

    #[test]
    fn lone_u32_write_on_a_foreign_contract_is_not_family() {
        // Some unrelated contract keeping an i128 under u32(1) — half a
        // reserve pair and no CONFIG: not ours, silently.
        let changes = vec![change(
            "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX",
            json!({"type": "u32", "value": 1}),
            json!({"type": "i128", "value": "42"}),
            "updated",
        )];
        assert_eq!(extract_config_pools(&changes), vec![]);
    }

    #[test]
    fn admin_key_and_markers_do_not_qualify_a_pool() {
        // u32(3) = Admin (an address, not i128) plus the XYK marker — no
        // CONFIG, no reserves: nothing extracted.
        let changes = vec![
            change(
                POOL,
                json!({"type": "u32", "value": 3}),
                json!({"type": "address",
                    "value": "GAPRPZYCIV3QPMCTWSRDNY64EJMZNCJFUCTJHQDQNW6RJ66TEVEH5UDU"}),
                "updated",
            ),
            change(
                POOL,
                json!({"type": "sym", "value": "XYK_POOL"}),
                json!({"type": "bool", "value": true}),
                "updated",
            ),
        ];
        assert_eq!(extract_config_pools(&changes), vec![]);
    }

    #[test]
    fn extracts_the_factorys_own_pools_vector() {
        // Real shape from the creation ledger 64,030,567: the factory's
        // u32(2) entry is a vec of pool addresses ending with the newborn.
        const FACTORY: &str = "CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI";
        let changes = vec![change(
            FACTORY,
            json!({"type": "u32", "value": 2}),
            json!({"type": "vec", "value": [
                {"type": "address",
                 "value": "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX"},
                {"type": "address", "value": POOL}
            ]}),
            "updated",
        )];
        let got = extract_address_list_writes(&changes);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].owner, FACTORY);
        assert_eq!(got[0].members.len(), 2);
        assert_eq!(got[0].members[1], POOL);
    }

    #[test]
    fn mixed_or_empty_vecs_are_not_membership_lists() {
        let mixed = vec![change(
            POOL,
            json!({"type": "u32", "value": 7}),
            json!({"type": "vec", "value": [
                {"type": "address", "value": POOL},
                {"type": "u32", "value": 1}
            ]}),
            "updated",
        )];
        assert_eq!(extract_address_list_writes(&mixed), vec![]);
        let empty = vec![change(
            POOL,
            json!({"type": "u32", "value": 7}),
            json!({"type": "vec", "value": []}),
            "updated",
        )];
        assert_eq!(extract_address_list_writes(&empty), vec![]);
    }

    #[test]
    fn half_a_reserve_pair_with_config_is_refused_loudly() {
        // CONFIG proves the owner IS family — then a half reserve pair is
        // malformed, and the whole write is refused (dropping to a partial
        // row would be the misleading-fallback class).
        let changes = vec![
            change(
                POOL,
                json!({"type": "sym", "value": "CONFIG"}),
                real_config(),
                "updated",
            ),
            change(
                POOL,
                json!({"type": "u32", "value": 1}),
                json!({"type": "i128", "value": "42"}),
                "updated",
            ),
        ];
        assert_eq!(extract_config_pools(&changes), vec![]);
    }
}
