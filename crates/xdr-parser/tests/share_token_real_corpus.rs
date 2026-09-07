//! Every deposit transaction on mainnet, through the real share-token
//! detector (task 0374, step 15).
//!
//! The inline tests pin four hand-copied payloads; this runs the compiled
//! `detect_share_tokens` over ALL 85k+ deposit transactions of every
//! registered pool and checks the result against ground truth that was
//! verified ON CHAIN (T6: 16 ambiguous pools confirmed 16/16 against
//! `share_id()`, plus 12 clean-pool spot checks). The T6 research measured
//! its rule with SQL imitating the logic; this test runs the logic.
//!
//! Skipped (passes trivially) when `SHARE_TOKEN_CORPUS` is unset. Harvest:
//! the deposit-tx list plus `deposit_liquidity`/`mint` events grouped per
//! transaction — the exact recipe lives in task 0374 (step 15 notes).
//!
//! ```sh
//! SHARE_TOKEN_CORPUS=/path/share_token_corpus.jsonl \
//!   cargo test -p xdr-parser --test share_token_real_corpus -- --nocapture
//! ```

use std::collections::BTreeMap;

use domain::ContractEventType;
use xdr_parser::types::{EventSource, ExtractedEvent};

#[derive(serde::Deserialize)]
struct CorpusEvent {
    contract: String,
    idx: u32,
    topics: String,
    data: String,
}

#[derive(serde::Deserialize)]
struct CorpusTx {
    ledger: i64,
    tx: i64,
    events: Vec<CorpusEvent>,
}

/// (pool, share token) pairs confirmed against `share_id()` by read-only
/// simulation on mainnet — the T6 resolution table, verbatim.
const ON_CHAIN_TRUTH: &[(&str, &str)] = &[
    (
        "CA6GAFOJCW4MGQQBUCQUSA3CLIH25G4SNKB2JHYKZCVWZTNW5VXMSC4O",
        "CDOY7ILRR7PDGLBXZUPSENB6XOET77PR2JY3HXDGQS3TS4T764OYBUGO",
    ),
    (
        "CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE",
        "CAVKLYY4RWFQBRA2YI5GTGGXKUKJQI3JLAHDGXMS7L5RDH6X6A47NMOZ",
    ),
    (
        "CAB6MICC2WKRT372U3FRPKGGVB5R3FDJSMWSLPF2UJNJPYMBZ76RQVYE",
        "CA2KG5ZCUZSAHC2MMDSNPRLTFPRAS7OUR5EYNGJW3FHJPLYB3WLO3WQK",
    ),
    (
        "CAMXZXXBD7DFBLYLHUW24U4MY37X7SU5XXT5ZVVUBXRXWLAIM7INI7G2",
        "CDMRHKJCYYHZTRQVR7NY43PR7ISMRBYC2O57IMVAQ7B7P2I2XGIZLI5E",
    ),
    (
        "CB7FKGSTHP75ORTIZGGMVUTQLEMVTSEOI4QORQPCABJSGTAATDFCE2YV",
        "CDXFLNBIBD75DKS2HLNMMWMBMTTYWAJLH7OUUH3S5NWZ3FD7LERNOE4W",
    ),
    (
        "CBL7MWLEZ4SU6YC5XL4T3WXKNKNO2UQVDVONOQSW5VVCYFWORROHY4AM",
        "CDA7DVSWI2VKGK72ZZW4C2ASBOJEVFNA72FOXXR7BYE6QEBYQUTKMIXU",
    ),
    (
        "CBP67KMDEOZHRJ5YM7DS4MOVYJH66YCEFCVBDBQVNTI46X37DU6KOZMB",
        "CDWSTBWL2355YNAE3FCV3WWSUDEDURHAHWFMO5UZ4AZIN7V7O6Q7LW2A",
    ),
    (
        "CCFGZJTHQZGDZP5PK6WMLKHKJ72ACSVMJGCI2NFR7Q6EAVSKWLJB3ZH3",
        "CAFEXSVTYRXJSAQWI6JZE5VL3US5XMDKDNQFACPYTKJNSHCHCU2ER4YE",
    ),
    (
        "CCLZQDL5LY2DBPNNFBRKPSROGFGTT7Y7AI2SM6QUI3SUTTKA672X4PDF",
        "CA73MQDXDHT7Z37KIWP5BCRGAKOXDK2FLR3OPVKNUJUEHK5SWL74SE4K",
    ),
    (
        "CCPR2Q3F2TPOVKPKTAHGLMMWPL5RBNBYDWGGUNBYP7KV7W245IAJXRI7",
        "CDRYIHHD3V6E4L3445V62A77WYWQYAJWLHTRBRITBTCDIOZ2D6UJ7ICJ",
    ),
    (
        "CCSY43EHJAHT3NQDYKAMJXRFBEEH7OXDL3J3VNGO33UUSEXWNN27GBIZ",
        "CC4BPROIXISEFC7UKTB2HYBLNSNP27WNCR7YNZOHXLTPTGDKFMKYQ2YN",
    ),
    (
        "CCY2PXGMKNQHO7WNYXEWX76L2C5BH3JUW3RCATGUYKY7QQTRILBZIFWV",
        "CBOHAVUYKQD4C7FIVXEDJCVLUZYUO6RN3VIKEDOTIJGDDV3QN33Y4T4D",
    ),
    (
        "CD4ASKG2XVZRAUXSXPCGUSBIX4JOC2TNA2FDBAPUNJB7RSUG5YGRQRSF",
        "CBJQXWMAXJPBWQ75JW26HAI72I5D5GRHMKQNAZ2YHEDGFORXCVA3NFGJ",
    ),
    (
        "CDE57N6XTUPBKYYDGQMXX7E7SLNOLFY3JEQB4MULSMR2AKTSAENGX2HC",
        "CBWYOO6AFZ6RNOBDVDB4BMPJDUTN3L45SEKMMDOFNY4PWJNMELYJRUMC",
    ),
    (
        "CDRRLXLBXYVENKIUCCTFUT5WW6BW43NJMAPEMGU6VIU74LV6526OLS35",
        "CCKFOOBRTACG72QJ5FUAW72KB2PYYHYYB7QXBQSFHRS7F2YKF6JWIMKX",
    ),
    (
        "CDUX476HQ4JZBPLLVEVRIVXDRKAGYDNDL7OLBE5DD37JXOUFI2NACXJ5",
        "CB6DMPDOCSSIW4M5WX77FN5AR77Z765UBSMNJPJ4KDHVJ636OEE4JAN4",
    ),
];

#[test]
fn every_mainnet_deposit_resolves_like_share_id() {
    let Ok(path) = std::env::var("SHARE_TOKEN_CORPUS") else {
        eprintln!("SHARE_TOKEN_CORPUS unset — skipping (see module docs)");
        return;
    };
    let raw = std::fs::read_to_string(&path).expect("corpus readable");

    // Final winner per pool = the sighting from the NEWEST ledger, mirroring
    // how the RMT side table converges (version = sighting ledger).
    let mut final_token: BTreeMap<String, (i64, String)> = BTreeMap::new();
    let mut txs = 0usize;
    let mut sightings = 0usize;

    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let tx: CorpusTx = serde_json::from_str(line).expect("corpus row");
        txs += 1;
        let events: Vec<ExtractedEvent> = tx
            .events
            .iter()
            .map(|e| ExtractedEvent {
                transaction_hash: tx.tx.to_string(),
                event_type: ContractEventType::Contract,
                source: EventSource::TxLevel,
                contract_id: Some(e.contract.clone()),
                topics: serde_json::from_str(&e.topics).expect("topics json"),
                data: serde_json::from_str(&e.data).expect("data json"),
                event_index: e.idx,
                op_index: None,
                event_pos_in_op: None,
                stage: None,
                ledger_sequence: tx.ledger as u32,
                created_at: 0,
            })
            .collect();

        for s in detect_share_tokens(&[(tx.tx.to_string(), events)]) {
            sightings += 1;
            let e = final_token
                .entry(s.pool)
                .or_insert((tx.ledger, s.token.clone()));
            if tx.ledger >= e.0 {
                *e = (tx.ledger, s.token);
            }
        }
    }

    eprintln!(
        "\n--- share-token corpus: {txs} deposit txs, {sightings} sightings, {} pools resolved ---",
        final_token.len()
    );

    // Ground truth first: the 16 on-chain confirmations must match exactly.
    let mut wrong = Vec::new();
    for (pool, want) in ON_CHAIN_TRUTH {
        match final_token.get(*pool) {
            Some((_, got)) if got == want => {}
            Some((_, got)) => wrong.push(format!("{pool}: got {got}, chain says {want}")),
            None => wrong.push(format!("{pool}: no sighting at all")),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of 16 on-chain-verified pools disagree:\n{}",
        wrong.len(),
        wrong.join("\n")
    );

    // Coverage: T6 measured 305 share-minting pools. The registry is live, so
    // newer pools may push this up — it must never come back DOWN.
    assert!(
        final_token.len() >= 305,
        "expected at least 305 share-minting pools, got {}",
        final_token.len()
    );
    assert!(txs > 80_000, "corpus looks truncated: {txs} txs");
}

// ---------------------------------------------------------------------------
// The detector under test. Moved OUT of `pool_router.rs` (task 0374): the
// deposit⇄mint rule is a verification oracle, not the live share-token
// source (`TokenShare` instance storage is — see `pool_state`), so it lives
// with its corpus instead of shipping in the production crate. The four
// typed-JSON accessors are test-local copies of `pool_router`'s private
// helpers.
// ---------------------------------------------------------------------------

use serde_json::Value;

/// One share-token sighting: a pool's deposit minting its share token in the
/// same transaction (task 0374, step 15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareTokenSighting {
    /// Pool contract StrKey (the `deposit_liquidity` emitter).
    pub pool: String,
    /// Share-token contract StrKey (the SEP-41 `mint` emitter).
    pub token: String,
}

/// Detect share tokens by the T6 rule, verified 16/16 against `share_id()`
/// on chain and holding for 75 200 of 75 200 share mints across all history:
///
/// in ONE transaction, a pool emits `deposit_liquidity` whose data vector
/// starts with `shares`, and its share token emits a SEP-41-shaped `mint`
/// (topics `[sym "mint", address admin, address to]` — topic 2 an ADDRESS;
/// the classic-SAC shape carries a `string` there and is excluded) whose
/// admin (topic 1) IS the pool and whose amount equals `shares`. When two
/// candidates survive — measured exactly once in all history, a share-token
/// migration dual-writing at ledger 53 552 533 — the highest `event_index`
/// wins, which is what `share_id()` returns.
///
/// Concentrated pools mint nothing, ever (measured over all 84 624 deposit
/// transactions), so they simply never produce a sighting — the safe
/// failure, by construction.
pub fn detect_share_tokens(events: &[(String, Vec<ExtractedEvent>)]) -> Vec<ShareTokenSighting> {
    let mut out = Vec::new();
    for (_tx, evs) in events {
        // (pool, shares) pairs announced by deposits in this tx.
        let deposits: Vec<(&str, String)> = evs
            .iter()
            .filter_map(|ev| {
                let pool = ev.contract_id.as_deref()?;
                let topics = ev.topics.as_array()?;
                if symbol_value(topics.first()?)? != "deposit_liquidity" {
                    return None;
                }
                let shares = vec_elements(&ev.data)?
                    .first()
                    .and_then(|v| v.get("value"))
                    .and_then(Value::as_str)?
                    .to_string();
                Some((pool, shares))
            })
            .collect();
        if deposits.is_empty() {
            continue;
        }
        for (pool, shares) in deposits {
            // Highest event_index wins the (single-occurrence) migration tie.
            let winner = evs
                .iter()
                .filter(|ev| sep41_mint_matches(ev, pool, &shares))
                .max_by_key(|ev| ev.event_index)
                .and_then(|ev| ev.contract_id.clone());
            if let Some(token) = winner {
                out.push(ShareTokenSighting {
                    pool: pool.to_string(),
                    token,
                });
            }
        }
    }
    out
}

/// SEP-41 `mint` with `admin == pool` and `amount == shares`.
fn sep41_mint_matches(ev: &ExtractedEvent, pool: &str, shares: &str) -> bool {
    let Some(topics) = ev.topics.as_array() else {
        return false;
    };
    if topics.first().and_then(symbol_value) != Some("mint") {
        return false;
    }
    // topic 2 must be an ADDRESS — the classic-SAC mint carries a string
    // ("CODE:ISSUER") there, and a SAC wrapping a deposited token in the same
    // tx is real (4 pools) and must not be mistaken for the share token.
    if !topics.get(2).is_some_and(|t| has_type(t, "address")) {
        return false;
    }
    if topics.get(1).and_then(address_value).as_deref() != Some(pool) {
        return false;
    }
    ev.data.get("value").and_then(Value::as_str) == Some(shares)
}

fn vec_elements(v: &Value) -> Option<&[Value]> {
    (v.get("type")?.as_str()? == "vec")
        .then(|| v.get("value")?.as_array().map(Vec::as_slice))
        .flatten()
}

fn has_type(v: &Value, tag: &str) -> bool {
    v.get("type").and_then(Value::as_str) == Some(tag)
}

fn address_value(v: &Value) -> Option<String> {
    (v.get("type")?.as_str()? == "address")
        .then(|| v.get("value")?.as_str().map(str::to_string))
        .flatten()
}

fn symbol_value(v: &Value) -> Option<&str> {
    (v.get("type")?.as_str()? == "sym")
        .then(|| v.get("value")?.as_str())
        .flatten()
}

mod shape_tests {
    use super::*;
    use serde_json::json;
    use xdr_parser::types::EventSource;

    fn ev(
        contract: &str,
        topics: serde_json::Value,
        data: serde_json::Value,
        idx: u32,
    ) -> ExtractedEvent {
        ExtractedEvent {
            transaction_hash: "tx".into(),
            event_type: ContractEventType::Contract,
            source: EventSource::TxLevel,
            contract_id: Some(contract.into()),
            topics,
            data,
            event_index: idx,
            op_index: None,
            event_pos_in_op: None,
            stage: None,
            ledger_sequence: 61_777_648,
            created_at: 0,
        }
    }

    /// Verbatim from mainnet — ledger 61 777 648, the transaction the T6
    /// research decoded by hand: the pool deposits `[shares, a, b]`, a classic
    /// SAC mints one of the deposited tokens (amount == a token amount), and
    /// the SEP-41 share token mints exactly `shares`. The SAC must lose on
    /// SHAPE (string topic 2), not luck.
    #[test]
    fn picks_the_share_token_and_excludes_the_sac_wrap() {
        const POOL: &str = "CAMXZXXBD7DFBLYLHUW24U4MY37X7SU5XXT5ZVVUBXRXWLAIM7INI7G2";
        const SAC: &str = "CBMFDIRY5OKI4JJURXC4SMEQPWB4UUADIADJK4NA6CYBNOYK4W4TMLLF";
        const SHARE: &str = "CDMRHKJCYYHZTRQVR7NY43PR7ISMRBYC2O57IMVAQ7B7P2I2XGIZLI5E";
        let events = vec![(
            "tx".to_string(),
            vec![
                ev(
                    SAC,
                    json!([
                        {"type":"sym","value":"mint"},
                        {"type":"address","value":"GDO24KCXPB2CKZTG3TXUQISUCOGURIPKRHEXYRJUQEFQTLFVLSA6XP7W"},
                        {"type":"string","value":"USDX:GDGQDVO6XPFSY4NMX75A7AOVYCF5FBTSXUFWNMURKM5PX7VDVYX4FXNW"}
                    ]),
                    json!({"type":"i128","value":"797243789"}),
                    2,
                ),
                ev(
                    SHARE,
                    json!([
                        {"type":"sym","value":"mint"},
                        {"type":"address","value":POOL},
                        {"type":"address","value":"GDO24KCXPB2CKZTG3TXUQISUCOGURIPKRHEXYRJUQEFQTLFVLSA6XP7W"}
                    ]),
                    json!({"type":"i128","value":"1632259506"}),
                    6,
                ),
                ev(
                    POOL,
                    json!([
                        {"type":"sym","value":"deposit_liquidity"},
                        {"type":"vec","value":[]}
                    ]),
                    json!({"type":"vec","value":[
                        {"type":"i128","value":"1632259506"},
                        {"type":"i128","value":"827371042"},
                        {"type":"i128","value":"797243789"}
                    ]}),
                    7,
                ),
            ],
        )];

        let got = detect_share_tokens(&events);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pool, POOL);
        assert_eq!(got[0].token, SHARE, "the SAC wrap must lose on shape");
    }

    /// The one migration transaction in all history (ledger 53 552 533):
    /// old and new share token both mint `shares` with the pool as admin.
    /// The higher event_index wins — that is what `share_id()` returns.
    #[test]
    fn migration_dual_write_resolves_to_the_higher_index() {
        const POOL: &str = "CDE57N6XTUPBKYYDGQMXX7E7SLNOLFY3JEQB4MULSMR2AKTSAENGX2HC";
        const OLD: &str = "CA4J7OKJRXHAAZGVT5QO7DRPYKE5X24PWA77JPNBZWNBWIL4FLZDYMS4";
        const NEW: &str = "CBWYOO6AFZ6RNOBDVDB4BMPJDUTN3L45SEKMMDOFNY4PWJNMELYJRUMC";
        let admin_mint = |token: &str, idx: u32| {
            ev(
                token,
                json!([
                    {"type":"sym","value":"mint"},
                    {"type":"address","value":POOL},
                    {"type":"address","value":"GABC"}
                ]),
                json!({"type":"i128","value":"614232397"}),
                idx,
            )
        };
        let events = vec![(
            "tx".to_string(),
            vec![
                admin_mint(OLD, 4),
                admin_mint(NEW, 5),
                ev(
                    POOL,
                    json!([
                        {"type":"sym","value":"deposit_liquidity"},
                        {"type":"vec","value":[]}
                    ]),
                    json!({"type":"vec","value":[{"type":"i128","value":"614232397"}]}),
                    6,
                ),
            ],
        )];

        let got = detect_share_tokens(&events);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].token, NEW, "share_id() returns the later token");
    }

    /// A mint whose admin is the pool but whose amount is a TOKEN amount
    /// (not shares) must not match — the amount test is what makes the rule
    /// deterministic (75 200/75 200 measured).
    #[test]
    fn amount_mismatch_is_no_sighting() {
        const POOL: &str = "CAMXZXXBD7DFBLYLHUW24U4MY37X7SU5XXT5ZVVUBXRXWLAIM7INI7G2";
        let events = vec![(
            "tx".to_string(),
            vec![
                ev(
                    "CDMRHKJCYYHZTRQVR7NY43PR7ISMRBYC2O57IMVAQ7B7P2I2XGIZLI5E",
                    json!([
                        {"type":"sym","value":"mint"},
                        {"type":"address","value":POOL},
                        {"type":"address","value":"GABC"}
                    ]),
                    json!({"type":"i128","value":"999"}),
                    1,
                ),
                ev(
                    POOL,
                    json!([
                        {"type":"sym","value":"deposit_liquidity"},
                        {"type":"vec","value":[]}
                    ]),
                    json!({"type":"vec","value":[{"type":"i128","value":"1000"}]}),
                    2,
                ),
            ],
        )];
        assert!(detect_share_tokens(&events).is_empty());
    }

    /// Concentrated pools deposit but never mint — no sighting, by
    /// construction, not by special-casing.
    #[test]
    fn a_mintless_deposit_yields_nothing() {
        const POOL: &str = "CBBMQBNHB2FYVZYV7VNHOJHUMTFJLR4PUMRVQYNW6RHIKZO2NQMIBUCV";
        let events = vec![(
            "tx".to_string(),
            vec![ev(
                POOL,
                json!([
                    {"type":"sym","value":"deposit_liquidity"},
                    {"type":"vec","value":[]}
                ]),
                json!({"type":"vec","value":[{"type":"i128","value":"5"}]}),
                1,
            )],
        )];
        assert!(detect_share_tokens(&events).is_empty());
    }
}
