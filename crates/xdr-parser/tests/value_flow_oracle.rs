//! Task 0540 / T04 — the events-vs-ledger oracle.
//!
//! For every transaction of every listed archive ledger, run BOTH readers on
//! the same `TransactionMeta` and compare per (holder, asset), bit-exact:
//!
//! - **edges**: `extract_asset_transfers` — the decode that feeds
//!   `asset_transfers` (per-op consensus events, emitter gate, payload rules);
//! - **witness**: `operation_balance_deltas` — the before→after balance changes
//!   consensus actually applied (accounts, trustlines, SAC/bespoke contract
//!   balances, pool reserves, claimable balances).
//!
//! Three states. `reconciled`: our decoder read what core wrote. `contradicted`:
//! a defect — in the decoder, the reader, or a protocol assumption — and the
//! test FAILS, printing tx, holder, asset and both figures. `no_witness`: the
//! asset is one the ledger reader cannot see (a bespoke token with its own
//! storage layout, a position token, an NFT); counted, never a failure. It is
//! a property of the reader, not a tolerance.
//!
//! Known-by-construction rules, not tolerances: fees are on neither side — the
//! charge is settled outside `TransactionMeta`, and the Soroban refund is
//! excluded by reading the operations' changes only
//! (`operation_balance_deltas`); a `mint`'s `from` and a `burn`'s `to` are NULL and the issuer
//! has no trustline to its own asset; NULL amounts (non-fungible) are
//! excluded; a failed transaction is empty on both sides.
//!
//! What this does NOT prove: a contract's honesty. A bespoke token that lies in
//! its events and in its storage reconciles perfectly.
//!
//! Gated on `LEDGER_CACHE_DIR` — a directory holding the listed ledgers as
//! `<HEX>--<seq>.xdr.zst` straight from the public archive
//! (`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/…`). Skips with the
//! exact fetch commands when unset or incomplete, so `cargo test` stays offline.
//!
//! ```bash
//! LEDGER_CACHE_DIR=.temp/ledger-cache \
//!     cargo test -p xdr-parser --test value_flow_oracle -- --nocapture
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::str::FromStr;

use stellar_xdr::{LedgerCloseMeta, TransactionMeta};
use xdr_parser::{
    EventAsset, ExtractedAssetTransfer, LedgerAsset, MAINNET_PASSPHRASE, derive_sac_strkey,
    extract_asset_transfers, extract_events, network_id, operation_balance_deltas,
};

/// The sample: thirty ledgers spread evenly over the ingested range
/// (50 457 424 – 64 268 152, step 476 232, protocols 20 through 27 — the same
/// thirty the V4 measurement used, research note §10), plus the named edge
/// cases from the research note — identical transfers inside one operation,
/// the six-hop pool arbitrage, an 86-operation batch, pre-Protocol-23 SAC
/// mints, a failed Soroban call.
const EPOCH_LEDGERS: &[u32] = &[
    50_457_424, 50_933_656, 51_409_888, 51_886_120, 52_362_352, 52_838_584, 53_314_816, 53_791_048,
    54_267_280, 54_743_512, 55_219_744, 55_695_976, 56_172_208, 56_648_440, 57_124_672, 57_600_904,
    58_077_136, 58_553_368, 59_029_600, 59_505_832, 59_982_064, 60_458_296, 60_934_528, 61_410_760,
    61_886_992, 62_363_224, 62_839_456, 63_315_688, 63_791_920, 64_268_152,
];
const EDGE_CASE_LEDGERS: &[u32] = &[
    64_249_110, // A573C63C…: two byte-identical AQUA transfers inside one operation
    64_260_088, // the six-hop classic-pool arbitrage displayed as 48 stroops
    60_000_138, // A99DF008…: 86 payment operations in one transaction
];

const PARTITION: u32 = 64_000;

fn s3_key(seq: u32) -> String {
    let start = seq - seq % PARTITION;
    format!(
        "s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/{:08X}--{}-{}/{:08X}--{}.xdr.zst",
        u32::MAX - start,
        start,
        start + PARTITION - 1,
        u32::MAX - seq,
        seq
    )
}

fn cache_path(dir: &std::path::Path, seq: u32) -> PathBuf {
    dir.join(format!("{:08X}--{}.xdr.zst", u32::MAX - seq, seq))
}

#[derive(Default, Debug)]
struct Tally {
    transactions: usize,
    edges: usize,
    reconciled: usize,
    contradicted: Vec<String>,
    no_witness: usize,
    rejects: usize,
}

/// One side of the comparison, keyed by (holder StrKey, asset) — the asset
/// normalised to the ledger vocabulary so both sides use the same key.
type Side = BTreeMap<(String, LedgerAsset), i128>;

fn edge_asset(t: &ExtractedAssetTransfer) -> LedgerAsset {
    match &t.asset {
        EventAsset::Native => LedgerAsset::Native,
        EventAsset::Credit { code, issuer } => LedgerAsset::Credit {
            code: code.clone(),
            issuer: issuer.clone(),
        },
        EventAsset::Bespoke => LedgerAsset::Bespoke(t.emitter.clone()),
    }
}

/// An `M…` endpoint counts for its underlying `G…` — the ledger has no `M…`.
fn holder(strkey: &str) -> String {
    if !strkey.starts_with('M') {
        return strkey.to_string();
    }
    match stellar_xdr::MuxedAccount::from_str(strkey) {
        Ok(m) => xdr_parser::envelope::muxed_to_g_strkey(&m),
        Err(_) => strkey.to_string(),
    }
}

fn tx_metas(lcm: &LedgerCloseMeta) -> (u32, Vec<TransactionMeta>) {
    use LedgerCloseMeta as L;
    match lcm {
        L::V0(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| p.tx_apply_processing.clone())
                .collect(),
        ),
        L::V1(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| p.tx_apply_processing.clone())
                .collect(),
        ),
        L::V2(v) => (
            v.ledger_header.header.ledger_seq,
            v.tx_processing
                .iter()
                .map(|p| p.tx_apply_processing.clone())
                .collect(),
        ),
    }
}

fn reconcile_tx(
    seq: u32,
    tx_index: usize,
    meta: &TransactionMeta,
    net: &[u8; 32],
    tally: &mut Tally,
) {
    tally.transactions += 1;
    let events = extract_events(meta, &format!("{seq}:{tx_index}"), seq, 0);
    let decoded = extract_asset_transfers(&events, net);
    tally.rejects += decoded.rejects.len();
    tally.edges += decoded.transfers.len();

    // E: signed sum per (holder, asset) over fungible edges.
    let mut e: Side = BTreeMap::new();
    // Labelled assets seen in this tx, by their derived SAC address — lets a
    // ledger-side `SacWrapped(C…)` (a contract-held SAC balance) be compared
    // against the event that named the classic asset.
    let mut sac_to_asset: HashMap<String, LedgerAsset> = HashMap::new();
    for t in &decoded.transfers {
        let Some(amount) = t.amount else { continue };
        let asset = edge_asset(t);
        if let LedgerAsset::Credit { code, issuer } = &asset
            && let Some(sac) = derive_sac_strkey(code, issuer, net)
        {
            sac_to_asset.insert(sac, asset.clone());
        }
        if asset == LedgerAsset::Native
            && let Some(sac) = derive_sac_strkey("", "", net)
        {
            sac_to_asset.insert(sac, LedgerAsset::Native);
        }
        if let Some(from) = &t.from {
            *e.entry((holder(from), asset.clone())).or_default() -= amount;
        }
        if let Some(to) = &t.to {
            *e.entry((holder(to), asset.clone())).or_default() += amount;
        }
    }
    // Fee events are on neither side. The charge never enters `TransactionMeta`;
    // the Soroban refund does before Protocol 23 (`tx_changes_after`) and not
    // after it — so the witness reads the OPERATIONS' changes only, and the
    // tx-level `fee` event is ignored here as it is in the edge table. (The
    // first two runs of this oracle found each half of that fact: 277 native
    // credits with no event pre-P23, then 1 907 refund events with no ledger
    // change post-P23.)
    e.retain(|_, v| *v != 0);

    // L: the witness, with SAC-wrapped balances translated to the classic asset.
    let mut l: Side = BTreeMap::new();
    let mut l_unresolved_sac = 0usize;
    for d in operation_balance_deltas(meta) {
        let asset = match d.asset {
            LedgerAsset::SacWrapped(sac) => match sac_to_asset.get(&sac) {
                Some(a) => a.clone(),
                None => {
                    // A contract-held SAC balance moved with no labelled event
                    // naming that asset in this tx — nothing to compare it
                    // against; counted as no witness on the event side.
                    l_unresolved_sac += 1;
                    continue;
                }
            },
            other => other,
        };
        *l.entry((d.account, asset)).or_default() += d.delta;
    }
    l.retain(|_, v| *v != 0);
    tally.no_witness += l_unresolved_sac;

    let keys: std::collections::BTreeSet<_> = e.keys().chain(l.keys()).cloned().collect();
    for key in keys {
        match (e.get(&key), l.get(&key)) {
            (Some(a), Some(b)) if a == b => tally.reconciled += 1,
            // Event-only bespoke: the reader cannot see this token's storage
            // (only the soroban-sdk `Balance(Address)` + bare i128 layout is
            // read). Nothing proven; counted.
            (Some(_), None) if matches!(key.1, LedgerAsset::Bespoke(_)) => tally.no_witness += 1,
            // Ledger-only bespoke: a token with the sdk layout moved without
            // an event we decoded. That IS a witness saying we missed one.
            (a, b) => tally.contradicted.push(format!(
                "ledger {seq} tx#{tx_index} holder {} asset {:?}: events {:?} ledger {:?}",
                key.0, key.1, a, b
            )),
        }
    }
}

#[test]
fn every_transfer_reconciles_against_the_ledger() {
    let Some(dir) = std::env::var_os("LEDGER_CACHE_DIR").map(PathBuf::from) else {
        eprintln!("LEDGER_CACHE_DIR not set — skipping the events-vs-ledger oracle");
        return;
    };
    let ledgers: Vec<u32> = EPOCH_LEDGERS
        .iter()
        .chain(EDGE_CASE_LEDGERS)
        .copied()
        .collect();
    let missing: Vec<u32> = ledgers
        .iter()
        .copied()
        .filter(|s| !cache_path(&dir, *s).exists())
        .collect();
    if !missing.is_empty() {
        eprintln!(
            "{} of {} ledgers missing from {} — skipping. Fetch them with:",
            missing.len(),
            ledgers.len(),
            dir.display()
        );
        for s in &missing {
            eprintln!("  aws s3 cp {} {}/", s3_key(*s), dir.display());
        }
        return;
    }

    let net = network_id(MAINNET_PASSPHRASE);
    let mut tally = Tally::default();
    for seq in &ledgers {
        let bytes = std::fs::read(cache_path(&dir, *seq)).expect("read cached ledger");
        let xdr = xdr_parser::decompress_zstd(&bytes).expect("zstd");
        let batch = xdr_parser::deserialize_batch(&xdr).expect("LedgerCloseMetaBatch");
        for lcm in batch.ledger_close_metas.iter() {
            let (got_seq, metas) = tx_metas(lcm);
            assert_eq!(got_seq, *seq, "cache file carries a different ledger");
            for (i, meta) in metas.iter().enumerate() {
                reconcile_tx(*seq, i, meta, &net, &mut tally);
            }
        }
    }

    eprintln!(
        "\nevents-vs-ledger oracle over {} ledgers\n  transactions : {}\n  edges        : {}\n  reconciled   : {} (holder, asset) keys\n  no_witness   : {}\n  rejects      : {} (token verbs the decoder refused)\n  contradicted : {}",
        ledgers.len(),
        tally.transactions,
        tally.edges,
        tally.reconciled,
        tally.no_witness,
        tally.rejects,
        tally.contradicted.len()
    );
    for line in tally.contradicted.iter().take(40) {
        eprintln!("  ✗ {line}");
    }
    assert!(
        tally.contradicted.is_empty(),
        "{} (holder, asset) keys where the events and the ledger disagree — a defect, not a tolerance",
        tally.contradicted.len()
    );
    assert!(tally.reconciled > 0, "the oracle compared nothing");
}
