//! Soroswap pair discovery and state (task 0518, second adapter under 0516).
//!
//! Soroswap is a Uniswap-V2 clone: one fixed constant-product mode, a
//! FACTORY per deployment that deploys one PAIR contract per token pair, and
//! the pair contract doubling as its own SEP-41 LP token. Four factory
//! deployments share the `SoroswapFactory` label on mainnet (measured
//! 2026-09-02): the documented one with 214 pairs and three dead early ones
//! with 11/6/4 — every counter gapless from 1, every first pair after the
//! ingest floor, zero orphan emitters. Discovery is therefore the factory's
//! own `new_pair` event, shape-driven at the FACTORY level: any contract
//! emitting the full registration shape IS a soroswap-family factory — no
//! hardcoded addresses, the same rule that surfaced Aquarius's ten routers.
//!
//! Two payload conventions, both anchored in vendor source
//! (`soroswap/core` `factory/src/event.rs`, fetched 2026-09-02; archived
//! captures in lore 0008):
//!
//! - **`new_pair`** — topics `[String("SoroswapFactory"),
//!   Symbol("new_pair")]` (the 0517 label convention), data a MAP of
//!   `{token_0, token_1, pair, new_pairs_length}`. `new_pairs_length` is the
//!   factory's own monotone counter — the free closure check for every
//!   backfill (`max == count` per factory).
//! - **Pair instance storage** — Soroswap's `DataKey` is a plain enum, so
//!   instance keys are bare u32 DISCRIMINANTS (read verbatim from mainnet,
//!   pair `CAM7DY53…`): `0` = token0 address, `1` = token1 address,
//!   `2`/`3` = Reserve0/Reserve1 as i128, `4` = the deploying factory's
//!   address; plus the SEP-41 token half under bare symbol keys
//!   (`TotalSupply`, `METADATA`, `name`, `symbol`) — the pair IS the LP
//!   token. The factory pointer at `4` is what corroborates a registration
//!   (the `Router`-class check from ADR 0058), and the reserves make the
//!   pair's OWN instance the reserve source — self-authenticated ledger
//!   state, with the `sync` event demoted to a monitored cross-check
//!   (exactly `update_reserves`' fate in 0374).
//!
//! Values stay RAW (i128 as strings); scaling is a read-time concern.

use serde_json::Value;

use crate::scval::{address, map_get, symbol, typed, typed_str};
use crate::types::{EventSource, ExtractedEvent, ExtractedLedgerEntryChange};

/// One pair registration, tied to the factory that emitted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRegistration {
    /// Emitting factory contract StrKey (`C…`).
    pub factory: String,
    pub event: NewPairEvent,
}

/// A decoded `new_pair` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPairEvent {
    /// The deployed pair contract — the pool's identity AND its LP token.
    pub pair: String,
    pub token_0: String,
    pub token_1: String,
    /// The factory's own pair counter AFTER this registration. Monotone from
    /// 1 per factory; `max == count` is the backfill closure check.
    pub new_pairs_length: u32,
}

/// Why a claimed `new_pair` was not decodable. Mirrors `AddPoolReject`:
/// `NotNewPair` is the silent everyday case, `Malformed` is a registration
/// that would otherwise go missing and must be warned about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewPairReject {
    /// Not a `new_pair` event at all — the normal case for every other
    /// event on the chain.
    NotNewPair,
    /// Claimed the shape and could not be read. The `&'static str` names
    /// what was wrong; it suffices because the only consumer is the warn.
    Malformed(&'static str),
}

/// Decode a `new_pair` event from its topics and data payloads.
///
/// The shape is BOTH topics plus the data-map keys — event names collide
/// across protocols (0516: the name is never the identifier), so the
/// `SoroswapFactory` label is part of the sieve, not decoration.
pub fn parse_new_pair(topics: &Value, data: &Value) -> Result<NewPairEvent, NewPairReject> {
    use NewPairReject as R;

    let topics = topics.as_array().ok_or(R::NotNewPair)?;
    if typed_str(topics.first().ok_or(R::NotNewPair)?, "string") != Some("SoroswapFactory") {
        return Err(R::NotNewPair);
    }
    if symbol(topics.get(1).ok_or(R::NotNewPair)?) != Some("new_pair") {
        return Err(R::NotNewPair);
    }

    // From here the event has claimed to be a registration; nothing may
    // fail quietly.
    let field = |name: &'static str| map_get(data, name);
    let addr_field = |name: &'static str| field(name).and_then(address).map(str::to_string);

    let pair = addr_field("pair").ok_or(R::Malformed("pair missing or not an address"))?;
    let token_0 = addr_field("token_0").ok_or(R::Malformed("token_0 missing or not an address"))?;
    let token_1 = addr_field("token_1").ok_or(R::Malformed("token_1 missing or not an address"))?;
    let new_pairs_length = field("new_pairs_length")
        .and_then(|v| typed(v, "u32"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(R::Malformed("new_pairs_length missing or not a u32"))?;

    Ok(NewPairEvent {
        pair,
        token_0,
        token_1,
        new_pairs_length,
    })
}

/// Scan a ledger's events for `new_pair` registrations — shaped like
/// `detect_pool_registrations`: semantic decoding lives here, staging only
/// maps results to rows.
pub fn detect_pair_registrations(
    events: &[(String, Vec<ExtractedEvent>)],
) -> Vec<PairRegistration> {
    let mut out = Vec::new();
    for (_tx, evs) in events {
        for ev in evs {
            // The diagnostic container carries copies of consensus events
            // AND events from FAILED transactions (task 0182); indexing it
            // would register pairs whose registration never applied.
            if matches!(ev.source, EventSource::Diagnostic) {
                continue;
            }
            let Some(factory) = ev.contract_id.as_deref() else {
                continue;
            };
            match parse_new_pair(&ev.topics, &ev.data) {
                Ok(event) => out.push(PairRegistration {
                    factory: factory.to_string(),
                    event,
                }),
                Err(NewPairReject::NotNewPair) => {}
                Err(reason) => tracing::warn!(
                    ledger_sequence = ev.ledger_sequence,
                    factory = %factory,
                    ?reason,
                    "new_pair claimed to be a registration and could not be read — a pair is missing"
                ),
            }
        }
    }
    out
}

/// The slice of a Soroswap pair's instance storage this task consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryPairState {
    /// The pair contract whose instance this is — pool identity and LP
    /// token in one.
    pub pair: String,
    pub token_0: String,
    pub token_1: String,
    /// The deploying factory, as the pair itself records it (`DataKey` 4) —
    /// the registration corroboration authority: the ledger-authenticated
    /// owner of this entry is the pair, so a third party cannot point a
    /// pair at a foreign factory.
    pub factory: String,
    /// Reserve0/Reserve1 (`DataKey` 2/3), raw i128 — BOTH or neither: a
    /// half-pair of reserves is malformed, not partial. `None` at creation
    /// time, before the first deposit writes them.
    pub reserves: Option<(String, String)>,
    /// SEP-41 `TotalSupply` of the LP token — the outstanding shares, raw
    /// i128. `None` before the first mint.
    pub total_supply: Option<String>,
}

/// Decode the pool-relevant slice of a pair's instance storage.
///
/// Recognition rests on the IDENTITY TRIPLE: bare-u32 keys `0` and `1`
/// holding addresses plus `4` holding an address — the vendor's `DataKey`
/// discriminants, not symbols, so this reader is deliberately separate from
/// the symbol-keyed Aquarius one. The composite is the sieve; its
/// false-positive rate against every instance write in the raw corpus is
/// measured by the corpus test, per 0516's shape-not-name rule.
pub fn parse_factory_pair(pair: &str, storage: &Value) -> Option<FactoryPairState> {
    let entries = storage.as_array()?;
    let u32_key = |n: u64| -> Option<&Value> {
        entries.iter().find_map(|kv| {
            let is = typed(kv.get("key")?, "u32").and_then(Value::as_u64) == Some(n);
            is.then(|| kv.get("value"))?
        })
    };
    // The SEP-41 half mixes TWO key spellings in ONE instance (read from a
    // real 55.4M-era ledger): `METADATA` is a BARE sym, while `TotalSupply`
    // is a VEC-WRAPPED sym — the token-SDK enum-variant encoding, the same
    // wrap Aquarius uses for every key. Accept both.
    let sym_key = |name: &str| -> Option<&Value> {
        entries.iter().find_map(|kv| {
            let k = kv.get("key")?;
            let is = symbol(k) == Some(name)
                || typed(k, "vec")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.len() == 1 && symbol(&a[0]) == Some(name));
            is.then(|| kv.get("value"))?
        })
    };
    let addr = |v: &Value| address(v).map(str::to_string);
    let i128s = |v: &Value| typed_str(v, "i128").map(str::to_string);

    let token_0 = u32_key(0).and_then(addr)?;
    let token_1 = u32_key(1).and_then(addr)?;
    let factory = u32_key(4).and_then(addr)?;
    // Both or neither: one reserve without the other is not a smaller
    // answer, it is a malformed one.
    let reserves = match (u32_key(2).and_then(i128s), u32_key(3).and_then(i128s)) {
        (Some(r0), Some(r1)) => Some((r0, r1)),
        (None, None) => None,
        _ => return None,
    };

    Some(FactoryPairState {
        pair: pair.to_string(),
        token_0,
        token_1,
        factory,
        reserves,
        total_supply: sym_key("TotalSupply").and_then(i128s),
    })
}

/// One pair-instance write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFactoryPair {
    pub state: FactoryPairState,
    pub ledger_sequence: u32,
    /// The entry was CREATED in this ledger. Load-bearing for registration
    /// corroboration, same reasoning as the router family's created gate:
    /// the factory deploys and initialises the pair in the registering
    /// transaction, so a genuine registration's instance is always a
    /// creation, while an attacker touching an existing pair can only
    /// produce an update.
    pub created: bool,
}

/// Extract Soroswap pair instances from ledger-entry changes. Mirrors
/// `extract_pool_instances`: only `created`/`updated`/`restored` carry a
/// value; the `state` pre-image and `removed` record nothing.
pub fn extract_factory_pairs(changes: &[ExtractedLedgerEntryChange]) -> Vec<ExtractedFactoryPair> {
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
        let key_is_instance = change
            .key
            .get("key")
            .and_then(|k| k.get("type"))
            .and_then(Value::as_str)
            == Some("ledger_key_contract_instance");
        if !key_is_instance {
            continue;
        }
        let Some(pair) = change.key.get("contract").and_then(Value::as_str) else {
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
        if let Some(state) = parse_factory_pair(pair, storage) {
            out.push(ExtractedFactoryPair {
                state,
                ledger_sequence: change.ledger_sequence,
                created: change.change_type == "created",
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verbatim from mainnet — the FIRST `new_pair` ever (ledger
    /// 50,688,706, dead early factory `CDBRTEJM…`). Kept exact so a payload
    /// change fails here rather than in production.
    fn real_new_pair() -> (Value, Value) {
        let topics = json!([
            {"type": "string", "value": "SoroswapFactory"},
            {"type": "sym", "value": "new_pair"}
        ]);
        let data = json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "new_pairs_length"},
             "value": {"type": "u32", "value": 1}},
            {"key": {"type": "sym", "value": "pair"},
             "value": {"type": "address", "value": "CDMC44BMEGF5GMJHNP6NQA3LLBMWLONQFV37E2J5NWYYBBEXNMYMKRBO"}},
            {"key": {"type": "sym", "value": "token_0"},
             "value": {"type": "address", "value": "CAINX4EAMVB5DJLM3TP7Z5AZIKEYBA6LKURSBF75C6MS35NDY3FYLV6Y"}},
            {"key": {"type": "sym", "value": "token_1"},
             "value": {"type": "address", "value": "CAVXDPJ2M6BWRVTJ3VOVSE3U7QISFS4ET3XA3ONS3UD47X6TA54PIXFJ"}}
        ]});
        (topics, data)
    }

    #[test]
    fn decodes_a_real_new_pair() {
        let (topics, data) = real_new_pair();
        let got = parse_new_pair(&topics, &data).expect("a real registration decodes");
        assert_eq!(
            got.pair,
            "CDMC44BMEGF5GMJHNP6NQA3LLBMWLONQFV37E2J5NWYYBBEXNMYMKRBO"
        );
        assert_eq!(
            got.token_0,
            "CAINX4EAMVB5DJLM3TP7Z5AZIKEYBA6LKURSBF75C6MS35NDY3FYLV6Y"
        );
        assert_eq!(got.new_pairs_length, 1);
    }

    #[test]
    fn the_label_is_part_of_the_sieve() {
        // Same name, different label: another protocol's `new_pair` must not
        // be claimed (0516: the name is never the identifier).
        let (_, data) = real_new_pair();
        let foreign = json!([
            {"type": "string", "value": "SomeOtherFactory"},
            {"type": "sym", "value": "new_pair"}
        ]);
        assert_eq!(
            parse_new_pair(&foreign, &data),
            Err(NewPairReject::NotNewPair)
        );
        // A sym label is the Aquarius convention, not this one.
        let sym_label = json!([
            {"type": "sym", "value": "SoroswapFactory"},
            {"type": "sym", "value": "new_pair"}
        ]);
        assert_eq!(
            parse_new_pair(&sym_label, &data),
            Err(NewPairReject::NotNewPair)
        );
    }

    #[test]
    fn a_claimed_registration_with_a_broken_field_is_malformed_not_silent() {
        let (topics, _) = real_new_pair();
        let data = json!({"type": "map", "value": [
            {"key": {"type": "sym", "value": "pair"},
             "value": {"type": "address", "value": "CDMC44BMEGF5GMJHNP6NQA3LLBMWLONQFV37E2J5NWYYBBEXNMYMKRBO"}}
        ]});
        assert!(matches!(
            parse_new_pair(&topics, &data),
            Err(NewPairReject::Malformed(_))
        ));
    }

    /// Verbatim shape of a live pair instance (mainnet `CAM7DY53…`,
    /// native-USDC, read 2026-09-02): bare-u32 DataKey discriminants plus
    /// the SEP-41 token half under bare symbols.
    fn real_pair_storage() -> Value {
        json!([
            {"key": {"type": "u32", "value": 0},
             "value": {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"}},
            {"key": {"type": "u32", "value": 1},
             "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}},
            {"key": {"type": "u32", "value": 2}, "value": {"type": "i128", "value": "3362421101426"}},
            {"key": {"type": "u32", "value": 3}, "value": {"type": "i128", "value": "585980063616"}},
            {"key": {"type": "u32", "value": 4},
             "value": {"type": "address", "value": "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2"}},
            {"key": {"type": "u32", "value": 7}, "value": {"type": "u32", "value": 0}},
            // VEC-WRAPPED, verbatim from ledger 55,423,809 — the token-SDK
            // enum encoding; a bare sym here was the fixture bug the local
            // e2e caught (total_shares = 0 on every active pair).
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "TotalSupply"}]},
             "value": {"type": "i128", "value": "1387420389"}},
            {"key": {"type": "sym", "value": "name"}, "value": {"type": "string", "value": "native-USDC Soroswap LP Token"}}
        ])
    }

    #[test]
    fn decodes_a_real_pair_instance() {
        let got = parse_factory_pair(
            "CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP",
            &real_pair_storage(),
        )
        .expect("a live pair decodes");
        assert_eq!(
            got.factory, "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2",
            "the factory pointer is the corroboration authority"
        );
        assert_eq!(
            got.reserves,
            Some(("3362421101426".into(), "585980063616".into()))
        );
        assert_eq!(got.total_supply.as_deref(), Some("1387420389"));
    }

    #[test]
    fn a_newborn_pair_without_reserves_still_decodes() {
        // At creation the identity triple exists before any deposit writes
        // reserves or supply — the registration corroboration must not
        // depend on them.
        let storage = json!([
            {"key": {"type": "u32", "value": 0},
             "value": {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"}},
            {"key": {"type": "u32", "value": 1},
             "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}},
            {"key": {"type": "u32", "value": 4},
             "value": {"type": "address", "value": "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2"}}
        ]);
        let got = parse_factory_pair(
            "CAM7DY53G63XA4AJRS24Z6VFYAFSSF76C3RZ45BE5YU3FQS5255OOABP",
            &storage,
        )
        .expect("identity triple suffices");
        assert_eq!(got.reserves, None);
        assert_eq!(got.total_supply, None);
    }

    #[test]
    fn half_a_reserve_pair_is_refused_and_foreign_instances_are_not_claimed() {
        // One reserve without the other = malformed, never a partial answer.
        let half = json!([
            {"key": {"type": "u32", "value": 0},
             "value": {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"}},
            {"key": {"type": "u32", "value": 1},
             "value": {"type": "address", "value": "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"}},
            {"key": {"type": "u32", "value": 2}, "value": {"type": "i128", "value": "1"}},
            {"key": {"type": "u32", "value": 4},
             "value": {"type": "address", "value": "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2"}}
        ]);
        assert_eq!(parse_factory_pair("CPAIR", &half), None);
        // An Aquarius pool instance (vec-wrapped symbol keys) is a different
        // dialect and must not match the u32 sieve.
        let aquarius = json!([
            {"key": {"type": "vec", "value": [{"type": "sym", "value": "Plane"}]},
             "value": {"type": "address", "value": "CCABO2IQYDWRGGQ4DYQ73CV3ZFDBRZTEQNDDJMFT7JZO54CLS4RYJROY"}}
        ]);
        assert_eq!(parse_factory_pair("CPOOL", &aquarius), None);
        // A random u32-keyed enum contract missing the triple is not a pair.
        let other = json!([
            {"key": {"type": "u32", "value": 0}, "value": {"type": "u64", "value": "9"}},
            {"key": {"type": "u32", "value": 4},
             "value": {"type": "address", "value": "CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2"}}
        ]);
        assert_eq!(parse_factory_pair("COTHER", &other), None);
    }
}
