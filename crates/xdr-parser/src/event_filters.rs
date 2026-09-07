//! SEP-41 / CAP-67 token-event decoding for Soroban contract events.
//!
//! [`parse_token_event`] classifies a contract event's topics as a fungible
//! token movement (transfer / mint / burn / clawback) and extracts the account
//! operands + the moved asset. Centralising the shape rules here keeps ingest
//! (`db_clickhouse::persist::stage`) and the `soroban-token-flow` backfill on a
//! single decode.

use serde_json::Value;

/// The asset a SEP-41 / CAP-67 token event names — the EVENT domain's asset
/// vocabulary (cf. `AssetRef` for op-declared assets, `LedgerAsset` for
/// ledger-read balances; each domain owns its own small asset enum, resolved to a
/// DB surrogate by the persistence layer).
///
/// CAP-67 "unified" SAC events carry the classic asset as a trailing SEP-11 string
/// topic (`"native"` or `"CODE:ISSUER"`); bespoke non-SAC tokens omit it, so their
/// identity IS the emitting contract (`Bespoke` — the caller supplies the emitter
/// surrogate it already holds; the id is not in the topics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAsset {
    /// Native XLM (`"native"` asset string).
    Native,
    /// A classic issued asset (`"CODE:ISSUER"` asset string).
    Credit { code: String, issuer: String },
    /// A bespoke non-SAC token: no asset string in the event, so the asset is the
    /// emitting contract; resolved from the emitting contract id by the caller.
    Bespoke,
}

/// The SEP-41 / CAP-67 token-event verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenEventKind {
    Transfer,
    Mint,
    Burn,
    Clawback,
}

/// A decoded SEP-41 / CAP-67 token event (transfer / mint / burn / clawback).
///
/// `from` is `None` for mint; `to` is `None` for burn and clawback. No amount:
/// the presence indexes never store it, and the tx-detail page decodes amounts
/// from archive XDR at read time (E3, ADR 0029) — so it is not needed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEvent {
    pub kind: TokenEventKind,
    pub from: Option<String>,
    pub to: Option<String>,
    pub asset: EventAsset,
}

/// The verb in a token event's first topic, if it is one of the four SEP-41 /
/// CAP-67 verbs — regardless of whether the remaining topics decode. Lets a
/// caller tell "not a token event" from "a token verb in a shape we do not
/// decode": the second is counted and raised, never dropped (task 0540).
pub fn token_verb(topics: &Value) -> Option<TokenEventKind> {
    let verb = topics.as_array()?.first()?;
    if verb.get("type").and_then(Value::as_str)? != "sym" {
        return None;
    }
    let sym = verb.get("value").and_then(Value::as_str)?;
    if sym.eq_ignore_ascii_case("transfer") {
        Some(TokenEventKind::Transfer)
    } else if sym.eq_ignore_ascii_case("mint") {
        Some(TokenEventKind::Mint)
    } else if sym.eq_ignore_ascii_case("burn") {
        Some(TokenEventKind::Burn)
    } else if sym.eq_ignore_ascii_case("clawback") {
        Some(TokenEventKind::Clawback)
    } else {
        None
    }
}

/// Decode any SEP-41 / CAP-67 token event from its topics. Returns `None` when
/// topics do not match a known token-event shape.
///
/// Shapes — verified against prod (task 0383), extended in task 0540 when a
/// review measured the admin shape on mainnet (3 169 `mint` events with an
/// address as the last topic in ledgers 64 000 000–64 100 000, 54 emitters):
/// - transfer `[sym, addr(from), addr(to), string(asset)?]`
/// - mint     `[sym, addr(to), string(asset)?]`              — CAP-67 SAC
/// - mint     `[sym, addr(admin), addr(to), string(asset)?]` — SEP-41 / pre-CAP-67 SAC
/// - burn     `[sym, addr(from), string(asset)?]`
/// - clawback `[sym, addr(from), string(asset)?]`            — CAP-67 SAC
/// - clawback `[sym, addr(admin), addr(from), string(asset)?]` — SEP-41 / pre-CAP-67 SAC
///
/// In the admin shapes the party to the movement is the SECOND address; the
/// admin authorised it and is not credited or debited. CAP-67 removed the
/// admin topic from the SAC; SEP-41 tokens built on `soroban-token-sdk` still
/// emit it. Decoded by shape (a second address topic selects the admin
/// shape), never by fixed position.
///
/// The trailing SEP-11 asset string is present on SAC events and absent on
/// bespoke tokens (→ `EventAsset::Bespoke`).
pub fn parse_token_event(topics: &Value) -> Option<TokenEvent> {
    let arr = topics.as_array()?;
    let kind = token_verb(topics)?;

    // (from, to, asset_idx) — the asset string, if any, sits right after the
    // address operand(s).
    let (from, to, asset_idx) = match kind {
        TokenEventKind::Transfer => (
            Some(address_topic(arr.get(1)?)?),
            Some(address_topic(arr.get(2)?)?),
            3,
        ),
        TokenEventKind::Mint => {
            let (to, asset_idx) = operand_after_optional_admin(arr)?;
            (None, Some(to), asset_idx)
        }
        TokenEventKind::Burn => (Some(address_topic(arr.get(1)?)?), None, 2),
        TokenEventKind::Clawback => {
            let (from, asset_idx) = operand_after_optional_admin(arr)?;
            (Some(from), None, asset_idx)
        }
    };

    Some(TokenEvent {
        kind,
        from,
        to,
        asset: event_asset(arr.get(asset_idx)),
    })
}

/// The one address operand of `mint` / `clawback`, and the index the asset
/// string (if any) follows it at. CAP-67 puts the operand first
/// (`[verb, addr, asset?]`); SEP-41 and the pre-CAP-67 SAC put the admin
/// first and the operand second (`[verb, admin, addr, asset?]`). A second
/// address topic therefore selects the admin shape.
fn operand_after_optional_admin(arr: &[Value]) -> Option<(String, usize)> {
    let first = address_topic(arr.get(1)?)?;
    match arr.get(2).and_then(address_topic) {
        Some(operand) => Some((operand, 3)),
        None => Some((first, 2)),
    }
}

/// Resolve the asset from a trailing SEP-11 string topic. Absent, empty, or
/// malformed → `Bespoke` (bespoke token; identity is the emitting contract).
fn event_asset(topic: Option<&Value>) -> EventAsset {
    let Some(s) = topic.and_then(string_topic) else {
        return EventAsset::Bespoke;
    };
    if s == "native" {
        return EventAsset::Native;
    }
    match s.split_once(':') {
        Some((code, issuer)) if !code.is_empty() && !issuer.is_empty() => EventAsset::Credit {
            code: code.to_string(),
            issuer: issuer.to_string(),
        },
        _ => EventAsset::Bespoke,
    }
}

fn string_topic(topic: &Value) -> Option<String> {
    crate::scval::typed_str(topic, "string").map(str::to_string)
}

fn address_topic(topic: &Value) -> Option<String> {
    crate::scval::address(topic).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sym(value: &str) -> Value {
        json!({ "type": "sym", "value": value })
    }

    fn addr(value: &str) -> Value {
        json!({ "type": "address", "value": value })
    }

    // ---- parse_token_event (0383) ----------------------------------------

    fn string_topic(value: &str) -> Value {
        json!({ "type": "string", "value": value })
    }

    const ISSUER: &str = "GB5WIXCUO5DWAJSVLVIJH5SBWGIRKGD27YYHLPOISGBO7MW2UH3EJXLM";

    #[test]
    fn token_event_transfer_sac_credit() {
        let ev = parse_token_event(&json!([
            sym("transfer"),
            addr("GBFROM"),
            addr("GBTO"),
            string_topic(&format!("USDC:{ISSUER}"))
        ]))
        .unwrap();
        assert_eq!(ev.kind, TokenEventKind::Transfer);
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
        assert_eq!(
            ev.asset,
            EventAsset::Credit {
                code: "USDC".to_string(),
                issuer: ISSUER.to_string()
            }
        );
    }

    #[test]
    fn token_event_transfer_native() {
        let ev = parse_token_event(&json!([
            sym("transfer"),
            addr("GBFROM"),
            addr("GBTO"),
            string_topic("native")
        ]))
        .unwrap();
        assert_eq!(ev.asset, EventAsset::Native);
    }

    #[test]
    fn token_event_transfer_bespoke_no_asset_string_is_contract() {
        let ev =
            parse_token_event(&json!([sym("transfer"), addr("GBFROM"), addr("GBTO")])).unwrap();
        assert_eq!(ev.kind, TokenEventKind::Transfer);
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
        assert_eq!(ev.asset, EventAsset::Bespoke);
    }

    #[test]
    fn token_event_mint_has_to_no_from() {
        let ev = parse_token_event(&json!([
            sym("mint"),
            addr("GBTO"),
            string_topic(&format!("BISMUTH:{ISSUER}"))
        ]))
        .unwrap();
        assert_eq!(ev.kind, TokenEventKind::Mint);
        assert_eq!(ev.from, None);
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
    }

    #[test]
    fn token_event_burn_has_from_no_to() {
        let ev = parse_token_event(&json!([
            sym("burn"),
            addr("GBFROM"),
            string_topic(&format!("GOLD:{ISSUER}"))
        ]))
        .unwrap();
        assert_eq!(ev.kind, TokenEventKind::Burn);
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.to, None);
    }

    #[test]
    fn token_event_clawback_has_from_no_to() {
        let ev = parse_token_event(&json!([
            sym("clawback"),
            addr("GBFROM"),
            string_topic(&format!("VELO:{ISSUER}"))
        ]))
        .unwrap();
        assert_eq!(ev.kind, TokenEventKind::Clawback);
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.to, None);
    }

    #[test]
    fn token_event_mint_bespoke_two_topics_is_contract() {
        let ev = parse_token_event(&json!([sym("mint"), addr("GBTO")])).unwrap();
        assert_eq!(ev.kind, TokenEventKind::Mint);
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
        assert_eq!(ev.asset, EventAsset::Bespoke);
    }

    #[test]
    fn token_event_is_case_insensitive_on_verb() {
        let ev =
            parse_token_event(&json!([sym("MINT"), addr("GBTO"), string_topic("native")])).unwrap();
        assert_eq!(ev.kind, TokenEventKind::Mint);
    }

    #[test]
    fn token_event_rejects_unknown_symbol() {
        assert!(parse_token_event(&json!([sym("swap"), addr("GBA"), addr("GBB")])).is_none());
    }

    #[test]
    fn token_event_rejects_mint_without_address() {
        assert!(parse_token_event(&json!([sym("mint"), sym("not_an_address")])).is_none());
    }

    // ---- SEP-41 admin shapes (0540 review, measured on mainnet) ----------

    /// The `soroban-token-sdk` shape `("mint", admin, to)`: the emitter is
    /// its own admin, the recipient is the SECOND address. Real instance:
    /// tx 7850558829833248568, ledger 64 000 048, event 3.
    #[test]
    fn token_event_sep41_mint_credits_the_second_address_not_the_admin() {
        let ev = parse_token_event(&json!([sym("mint"), addr("CADMIN"), addr("GBTO")])).unwrap();
        assert_eq!(ev.kind, TokenEventKind::Mint);
        assert_eq!(ev.from, None);
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
        assert_eq!(ev.asset, EventAsset::Bespoke);
    }

    /// The pre-CAP-67 SAC shape kept the admin AND the asset string.
    #[test]
    fn token_event_pre_cap67_sac_mint_keeps_the_asset_after_the_admin() {
        let ev = parse_token_event(&json!([
            sym("mint"),
            addr("GADMIN"),
            addr("GBTO"),
            string_topic(&format!("KALE:{ISSUER}"))
        ]))
        .unwrap();
        assert_eq!(ev.to.as_deref(), Some("GBTO"));
        assert_eq!(
            ev.asset,
            EventAsset::Credit {
                code: "KALE".to_string(),
                issuer: ISSUER.to_string()
            }
        );
    }

    #[test]
    fn token_event_sep41_clawback_debits_the_second_address_not_the_admin() {
        let ev =
            parse_token_event(&json!([sym("clawback"), addr("CADMIN"), addr("GBFROM")])).unwrap();
        assert_eq!(ev.kind, TokenEventKind::Clawback);
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.to, None);
    }

    /// `burn` has no admin shape in SEP-41; a second address is not decoded
    /// as anything.
    #[test]
    fn token_event_burn_keeps_the_single_operand_shape() {
        let ev = parse_token_event(&json!([sym("burn"), addr("GBFROM"), addr("GBX")])).unwrap();
        assert_eq!(ev.from.as_deref(), Some("GBFROM"));
        assert_eq!(ev.asset, EventAsset::Bespoke);
    }

    #[test]
    fn token_verb_is_known_even_when_the_shape_is_not() {
        // The 1-topic `mint` that concentrated-liquidity position contracts
        // emit (measured: 123 in 100 000 ledgers): a verb, no decodable shape.
        let topics = json!([sym("mint")]);
        assert_eq!(token_verb(&topics), Some(TokenEventKind::Mint));
        assert!(parse_token_event(&topics).is_none());
        assert_eq!(token_verb(&json!([sym("swap"), addr("GBA")])), None);
        assert_eq!(token_verb(&json!([addr("GBA")])), None);
    }
}
