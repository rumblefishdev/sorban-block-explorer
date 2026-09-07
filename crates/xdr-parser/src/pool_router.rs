//! Router-registry pool discovery — decoding `add_pool` events (task 0374).
//!
//! # Scope: one discovery mechanism, not every AMM
//!
//! This covers the **router-registry** family only. A router contract announces
//! each new pool with `add_pool`, and emitting one **is** what makes a contract
//! a router — 10 contracts on mainnet do, all of one protocol family, together
//! registering 497 pools (count taken 2026-08-27; the registry is live and this
//! number moves).
//!
//! It does **not** cover AMMs that have no router registry. Soroswap pairs, for
//! instance, announce themselves — `("SoroswapPair", "deposit")` — with no
//! registry event anywhere, so they need their own discovery entirely
//! (task 0518, and blocked on 0517 because their event name sits in topic 1).
//!
//! Per-protocol decoding is not a design preference: Soroban defines no AMM
//! standard, so each protocol names and lays out its events differently and
//! there is nothing universal to decode. What *is* shared — the pool model,
//! the leg list, reserve storage, the position shape — is shared (task 0516);
//! reading events is the only place the chain forces a difference.
//! Classic protocol pools are not contracts at all and never reach here.
//!
//! **Shape-driven, never address-driven.** Nothing here matches a contract
//! address: the vendor documents one router, and keying on that address
//! silently drops ~6 % of live pools — see the verification pass in task 0374.
//!
//! # Payload — from the vendor's own source, not inferred
//!
//! `liquidity_pool_router/src/events.rs` publishes:
//!
//! ```rust,ignore
//! self.env().events().publish(
//!     (Symbol::new(self.env(), "add_pool"), tokens),       // topics
//!     (pool_address, pool_type, subpool_salt, init_args),  // data
//! );
//! ```
//!
//! which reaches us as ScVal JSON:
//!
//! ```text
//! topics = [ {sym "add_pool"}, {vec [ {address token}, … ]} ]
//! data   = {vec [ {address pool_address},
//!                 {sym pool_type},
//!                 {bytes subpool_salt},
//!                 {vec [ {u32 fee}, … ]} ]}
//! ```
//!
//! Corroboration, 2026-08-27: the vendor repository is unreachable (404 on
//! every branch and through the API), so the snippet above comes from an
//! archived capture (research 0008, fetched 2026-03-27) and **cannot be
//! re-confirmed at source today**. Two independent checks stand in for it: the
//! deployed contract's own spec, pulled from chain, agrees on every part it
//! covers — `pool_type` really is a `Symbol`, and the `BytesN<32>` really is
//! the router's pool lookup key. Note the spec carries **no event definitions
//! at all**, so an event's shape can only ever be confirmed by source or by
//! observation; there is no third route.
//!
//! Structural conformance measured 2026-08-27: **497 of 497** `add_pool`
//! events in all history match this shape, across all ten emitting contracts,
//! with pool types drawn from one vocabulary. No false positives exist today —
//! but the shape is the only guard, so a contract using the name for something
//! else with the same layout would be accepted. Behaviour confirms what the
//! registry only claims: 23 registrations from five dead deployments have
//! never emitted a single pool event, so a caller should treat a registration
//! as a candidate and let activity confirm it.
//!
//! **The leg list is a list.** Measured across all registrations: 488 pools
//! carry two legs, 7 carry three, and **2 carry four** — all `stable`. Treating
//! legs as a pair is the classic-AMM assumption that does not survive here, and
//! an upper bound of three would not survive either. No bound is assumed.

use serde_json::Value;

use crate::scval::{address, raw_scalar, symbol, vec_elements};
use crate::types::{EventSource, ExtractedEvent};

/// One pool registration, tied to the router that emitted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolRegistration {
    /// Emitting router contract StrKey (`C…`).
    pub router: String,
    pub event: AddPoolEvent,
}

/// Scan a ledger's events for `add_pool` registrations — the router-registry
/// discovery detector, shaped like [`crate::detect_nft_events`]: semantic
/// decoding lives here, staging only maps the result to rows.
///
/// An event that CLAIMS to be a registration and cannot be read is a pool
/// going missing: it is warned about here (with the reject reason) and
/// dropped from the result — never silently.
pub fn detect_pool_registrations(
    events: &[(String, Vec<ExtractedEvent>)],
) -> Vec<PoolRegistration> {
    let mut out = Vec::new();
    for (_tx, evs) in events {
        for ev in evs {
            // The diagnostic container carries byte-identical copies of
            // consensus events AND events from FAILED transactions (task
            // 0182). Indexing it would register pools whose registration
            // never applied. Every sibling detector filters it; this one is
            // the guard review #438 found missing.
            if matches!(ev.source, EventSource::Diagnostic) {
                continue;
            }
            // `as_deref`, not `clone`: this runs for EVERY event on the
            // chain, and the overwhelming majority are not registrations —
            // the allocation belongs in the match arm that keeps one.
            let Some(router) = ev.contract_id.as_deref() else {
                continue;
            };
            match parse_add_pool(&ev.topics, &ev.data) {
                Ok(event) => out.push(PoolRegistration {
                    router: router.to_string(),
                    event,
                }),
                Err(AddPoolReject::NotAddPool) => {}
                Err(reason) => tracing::warn!(
                    ledger_sequence = ev.ledger_sequence,
                    router = %router,
                    ?reason,
                    "add_pool claimed to be a registration and could not be read — a pool is missing"
                ),
            }
        }
    }
    out
}

// The deposit⇄mint share-token detector (T6 rule) is NOT part of the live
// pipeline: `TokenShare` in pool instance storage is the fundamental source
// (see `pool_state`), and the event-correlation rule is a demoted
// CROSS-CHECK. It lives with its corpus oracle in
// `tests/share_token_real_corpus.rs`, out of the production module.

/// One decoded `add_pool` event: a pool joining a router's registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddPoolEvent {
    /// Pool contract StrKey (`C…`).
    pub pool: String,
    /// Pool shape, exactly as the router wrote it.
    ///
    /// Deliberately not normalised here. Three vocabularies for this concept
    /// are live at once — `constant` in this event, `standard` in pool state,
    /// `ConstantProduct` in the contract's own enum — and a decoder that
    /// folded them would be asserting they mean the same thing, which is an
    /// interpretation, not a decoding. Whoever needs one vocabulary maps it
    /// where that need lives, and keeps this value to fall back on.
    pub pool_type: String,
    /// Pool legs, in the order the router registered them. This order is the
    /// one the pool's own `get_tokens()` reports, so reserve vectors line up
    /// with it index-for-index and need no reordering.
    ///
    /// The `subpool_salt` at data position 2 (the router's slot key — NOT a
    /// pool identity and NOT a WASM hash; see task 0374 step 1b) is
    /// deliberately not decoded: nothing consumes it, and the schema rule for
    /// registration provenance is "extract on demand, never copy" — the event
    /// sits complete in `soroban_events` forever.
    pub tokens: Vec<String>,
    /// `init_args` — the construction arguments, as raw decoded strings in
    /// emitted order.
    ///
    /// Kept as text, unparsed, because the list is **three different
    /// vocabularies wearing one shape** (measured across all history):
    ///
    /// | Pool type | `init_args` | Meaning |
    /// |---|---|---|
    /// | `constant`, `elastic` | `[u32]` | fee |
    /// | `concentrated` | `[u32, i32]` | fee, tick spacing (signed) |
    /// | `stable` | `[u32, u128]` or `[u32, u128, u32]` | fee, amplification, admin fee |
    ///
    /// So position 1 is a tick spacing in one type and an amplification factor
    /// in another. Parsing to a numeric list here would also invite dropping
    /// an element that does not fit and **shifting the rest left**, which
    /// turns a three-argument stable pool into something shaped exactly like a
    /// two-argument concentrated one. `u128` cannot be assumed to fit `i64`
    /// either. Whoever needs a number reads it against a known pool type.
    pub init_args: Vec<String>,
}

/// Why an event did not yield a registration.
///
/// Separated from success so a caller can tell the two apart, because they
/// mean opposite things. [`NotAddPool`](Self::NotAddPool) is the overwhelming
/// normal case — every other event on the chain — and counting it is noise.
/// Every other variant means an event *claimed* to be a registration and could
/// not be read, so a pool is missing and somebody should be told. Collapsing
/// both into a bare `None` makes that alarm impossible to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddPoolReject {
    /// Not a registration at all. Expected, and not worth counting.
    NotAddPool,
    /// Claims to be a registration and could not be read — a pool is
    /// missing. The message names which part was wrong; one variant
    /// suffices because the only consumer is the warn log.
    Malformed(&'static str),
}

/// Decode an `add_pool` event from its topics and data payloads.
///
/// Every error other than [`AddPoolReject::NotAddPool`] is a pool that would
/// otherwise go missing — count those, and alert on them.
pub fn parse_add_pool(topics: &Value, data: &Value) -> Result<AddPoolEvent, AddPoolReject> {
    use AddPoolReject as R;

    let topics = topics.as_array().ok_or(R::NotAddPool)?;
    if symbol(topics.first().ok_or(R::NotAddPool)?) != Some("add_pool") {
        return Err(R::NotAddPool);
    }

    // Legs ride in topic 1 as a vec of addresses. From here on the event has
    // claimed to be a registration, so nothing may fail quietly.
    let tokens: Vec<String> = topics
        .get(1)
        .and_then(vec_elements)
        .ok_or(R::Malformed("legs missing or not a vec of addresses"))?
        .iter()
        .map(|v| address(v).map(str::to_string))
        .collect::<Option<_>>()
        .ok_or(R::Malformed("legs missing or not a vec of addresses"))?;
    if tokens.is_empty() {
        return Err(R::Malformed("empty leg list"));
    }

    let fields = vec_elements(data).ok_or(R::Malformed("data is not the expected tuple"))?;
    let pool = fields
        .first()
        .and_then(|v| address(v).map(str::to_string))
        .ok_or(R::Malformed("pool address missing or not an address"))?;
    let pool_type = fields
        .get(1)
        .and_then(symbol)
        .ok_or(R::Malformed("pool type missing or not a symbol"))?
        .to_string();

    // Remaining fields (the subpool salt at position 2, init_args at 3) are
    // positional and have been stable across all history, but a shorter
    // payload is a decodable registration all the same: the pool and its legs
    // are what the registry is for. The salt is not decoded — see `tokens`.
    // Every init_args element is kept. A `filter_map` here would silently
    // shorten the list and change what position 1 means.
    let init_args = fields
        .get(3)
        .and_then(vec_elements)
        .map(|items| items.iter().map(raw_scalar).collect())
        .unwrap_or_default();

    Ok(AddPoolEvent {
        pool,
        pool_type,
        tokens,
        init_args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verbatim from mainnet — router `CBQDHNBF…6QUK`, a two-token constant
    /// pool. Kept exact so a payload-shape change fails here rather than in
    /// production.
    fn constant_pool_event() -> (Value, Value) {
        let topics = json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CDLWTKL7XIALOQPTV7R2KKTXTA6OPKT4T354Y7RG7S6TERQ7KI2VPXIW"}
            ]}
        ]);
        let data = json!({"type": "vec", "value": [
            {"type": "address", "value": "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O"},
            {"type": "sym", "value": "constant"},
            {"type": "bytes", "value": "suAvz8pslvitXL2E53hKd3s22clqJFlALE9FhGKqt/A="},
            {"type": "vec", "value": [{"type": "u32", "value": 10}]}
        ]});
        (topics, data)
    }

    #[test]
    fn decodes_a_mainnet_constant_pool() {
        let (topics, data) = constant_pool_event();
        let ev = parse_add_pool(&topics, &data).expect("well-formed add_pool");

        assert_eq!(
            ev.pool,
            "CDTSSTLKVVPWJZXVCGJJNGWKH5MY7OMINVXTB7DGFMDJTCCDBCSRG52O"
        );
        assert_eq!(ev.pool_type, "constant");
        assert_eq!(ev.tokens.len(), 2);
        assert_eq!(
            ev.tokens[0],
            "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
        );
        assert_eq!(ev.init_args, vec!["10"]);
    }

    /// Verbatim from mainnet — a `stable` pool, three `init_args`
    /// (`u32` fee, `u128` amplification, `u32` admin fee). Pinned exactly so a
    /// change in the widest real argument list fails here.
    #[test]
    fn decodes_a_mainnet_stable_pool_with_three_init_args() {
        let topics = json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CBZVSNVB55ANF24QVJL2K5QCLOAB6XITGTGXYEAF6NPTXYKEJUYQOHFC"}
            ]}
        ]);
        let data = json!({"type": "vec", "value": [
            {"type": "address", "value": "CDO2P6WRYF752RA2G23BQFEGKQ4Z4V53O7V43D3KDKNIB3TT24MPU7QI"},
            {"type": "sym", "value": "stable"},
            {"type": "bytes", "value": "1IpnTyaMNDXHXJjpAMaBFGtq1aRpqbXw4/uwrovz85s="},
            {"type": "vec", "value": [
                {"type": "u32", "value": 4},
                {"type": "u128", "value": "85"},
                {"type": "u32", "value": 0}
            ]}
        ]});

        let ev = parse_add_pool(&topics, &data).expect("real stable pool");
        assert_eq!(ev.pool_type, "stable");
        assert_eq!(
            ev.init_args,
            vec!["4", "85", "0"],
            "all three arguments survive, in order and unparsed"
        );
    }

    /// Four-leg pools are real — two of them, both `stable`. A decoder capped
    /// at three legs would drop them, and an earlier version of this module's
    /// documentation claimed three was the maximum.
    #[test]
    fn keeps_all_four_legs_of_a_stable_pool() {
        let leg = |c: &str| json!({"type": "address", "value": c});
        let topics = json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                leg("CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"),
                leg("CBZVSNVB55ANF24QVJL2K5QCLOAB6XITGTGXYEAF6NPTXYKEJUYQOHFC"),
                leg("CB7OOP3VSAWBZOOTOG2YEFANVU45GVWYUUM5HI32DKLHVKUDOFVQ37XP"),
                leg("CDLWTKL7XIALOQPTV7R2KKTXTA6OPKT4T354Y7RG7S6TERQ7KI2VPXIW")
            ]}
        ]);
        let data = json!({"type": "vec", "value": [
            {"type": "address", "value": "CDO2P6WRYF752RA2G23BQFEGKQ4Z4V53O7V43D3KDKNIB3TT24MPU7QI"},
            {"type": "sym", "value": "stable"},
            {"type": "bytes", "value": "1IpnTyaMNDXHXJjpAMaBFGtq1aRpqbXw4/uwrovz85s="},
            {"type": "vec", "value": [{"type": "u32", "value": 4}]}
        ]});

        let ev = parse_add_pool(&topics, &data).expect("four-leg stable pool");
        assert_eq!(ev.tokens.len(), 4, "no arity cap may exist");
    }

    /// A pool type nobody has catalogued must arrive as itself. `elastic` is
    /// real — one router registers it — and was absent from the first pass of
    /// this research; the next unknown type must not fare worse.
    #[test]
    fn passes_through_an_unknown_pool_type() {
        let (topics, mut data) = constant_pool_event();
        data["value"][1] = json!({"type": "sym", "value": "elastic"});

        let ev = parse_add_pool(&topics, &data).expect("unknown type still decodes");
        assert_eq!(
            ev.pool_type, "elastic",
            "a shape only one deployment registers must survive verbatim"
        );
    }

    /// Verbatim from mainnet — a `concentrated` pool. Its tick spacing is a
    /// **signed** `i32`, not the `u32` an earlier fixture assumed.
    #[test]
    fn decodes_a_mainnet_concentrated_pool() {
        let topics = json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": [
                {"type": "address", "value": "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"},
                {"type": "address", "value": "CB7OOP3VSAWBZOOTOG2YEFANVU45GVWYUUM5HI32DKLHVKUDOFVQ37XP"}
            ]}
        ]);
        let data = json!({"type": "vec", "value": [
            {"type": "address", "value": "CCKQASCNLYGCUYRKDFYHW5SRVUQNRPRX27YAK62CBNLEVSKIXE3CO4LI"},
            {"type": "sym", "value": "concentrated"},
            {"type": "bytes", "value": "7UVCkVX4dkddPHrBjTQnPWONalsvzHNts21xpXiIUsU="},
            {"type": "vec", "value": [
                {"type": "u32", "value": 30},
                {"type": "i32", "value": 60}
            ]}
        ]});

        let ev = parse_add_pool(&topics, &data).expect("real concentrated pool");
        assert_eq!(ev.pool_type, "concentrated");
        assert_eq!(ev.init_args, vec!["30", "60"], "tick spacing survives");
    }

    /// A wide `u128` must not be dropped or narrowed. Nothing on chain
    /// currently exceeds `i64`, but the emitted type permits it, and losing an
    /// element would shift every later argument into the wrong meaning.
    #[test]
    fn keeps_an_argument_too_wide_for_i64() {
        let (topics, mut data) = constant_pool_event();
        data["value"][1] = json!({"type": "sym", "value": "stable"});
        data["value"][3] = json!({"type": "vec", "value": [
            {"type": "u32", "value": 4},
            {"type": "u128", "value": "340282366920938463463374607431768211455"},
            {"type": "u32", "value": 0}
        ]});

        let ev = parse_add_pool(&topics, &data).expect("stable pool");
        assert_eq!(ev.init_args.len(), 3, "nothing may be dropped");
        assert_eq!(ev.init_args[1], "340282366920938463463374607431768211455");
        assert_eq!(ev.init_args[2], "0", "the admin fee must not shift left");
    }

    /// A shape nobody has catalogued arrives as itself and still registers the
    /// pool. The decoder has no catalogue to check against, which is the point:
    /// nothing here can turn an unknown shape into a known-looking one.
    #[test]
    fn an_uncatalogued_shape_arrives_intact() {
        let (topics, mut data) = constant_pool_event();
        data["value"][1] = json!({"type": "sym", "value": "hyperbolic"});

        let ev = parse_add_pool(&topics, &data).expect("pool is still real");
        assert_eq!(ev.pool_type, "hyperbolic", "no mapping may rewrite it");
        assert!(!ev.pool.is_empty(), "the pool itself must not be dropped");
    }

    #[test]
    fn rejects_other_events() {
        let (_, data) = constant_pool_event();
        let topics = json!([{"type": "sym", "value": "trade"}]);
        assert_eq!(
            parse_add_pool(&topics, &data),
            Err(AddPoolReject::NotAddPool)
        );
    }

    /// Soroswap-shaped events put a protocol label in topic 0 and the name in
    /// topic 1. This decoder must not claim them — that convention is task
    /// 0517's, and a false positive here would register junk pools.
    #[test]
    fn rejects_a_protocol_labelled_topic_zero() {
        let (_, data) = constant_pool_event();
        let topics = json!([
            {"type": "string", "value": "SoroswapPair"},
            {"type": "sym", "value": "add_pool"}
        ]);
        assert_eq!(
            parse_add_pool(&topics, &data),
            Err(AddPoolReject::NotAddPool)
        );
    }

    /// An empty leg list is a *malformed registration*, not some other event.
    /// The distinction is the point of the reject enum: this one means a pool
    /// went missing and should raise an alarm.
    #[test]
    fn rejects_a_registration_with_no_legs() {
        let (_, data) = constant_pool_event();
        let topics = json!([
            {"type": "sym", "value": "add_pool"},
            {"type": "vec", "value": []}
        ]);
        assert_eq!(
            parse_add_pool(&topics, &data),
            Err(AddPoolReject::Malformed("empty leg list"))
        );
    }
}
