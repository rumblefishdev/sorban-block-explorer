//! One-off audit (task 0540): for every token event in the real-mainnet corpus,
//! print which container it came from and whether it carries an operation index.
//!
//! Answers the question the edge table's row identity depends on: is the CAP-67
//! official identity (op_index + position within the operation) TOTAL for
//! transfer / mint / burn / clawback, even though it is not total for
//! `soroban_events` as a whole (fee and diagnostic events have no operation)?
//!
//! Usage:
//!   cargo run -q -p xdr-parser --example event_op_index_audit
//!       — audits the RPC-sourced corpus fixtures.
//!   cargo run -q -p xdr-parser --example event_op_index_audit -- <ledger.xdr.zst>
//!       — audits every transaction of one ARCHIVE ledger, which is the source
//!         that actually carries classic asset events (the RPC corpus does not).
use stellar_xdr::{Limits, ReadXdr, TransactionMeta};
use xdr_parser::{EventSource, extract_events};

const CORPUS: &[(&str, &str)] = &[
    (
        "account_merge",
        include_str!("../tests/fixtures/corpus/account_merge.b64"),
    ),
    (
        "amm_swap_sac",
        include_str!("../tests/fixtures/corpus/amm_swap_sac.b64"),
    ),
    (
        "bespoke_swap",
        include_str!("../tests/fixtures/corpus/bespoke_swap.b64"),
    ),
    (
        "claimable_balance",
        include_str!("../tests/fixtures/corpus/claimable_balance.b64"),
    ),
    (
        "create_account",
        include_str!("../tests/fixtures/corpus/create_account.b64"),
    ),
    (
        "failed_tx",
        include_str!("../tests/fixtures/corpus/failed_tx.b64"),
    ),
    (
        "path_payment",
        include_str!("../tests/fixtures/corpus/path_payment.b64"),
    ),
    (
        "soroban_mint",
        include_str!("../tests/fixtures/corpus/soroban_mint.b64"),
    ),
    (
        "tx_0a120260_v4",
        include_str!("../tests/fixtures/tx_0a120260_meta_v4.b64"),
    ),
];

/// The verbs the edge table stores. Everything else (fee, contract-specific
/// events) is out of its scope and may legitimately lack an operation.
const TOKEN_VERBS: &[&str] = &["transfer", "mint", "burn", "clawback"];

fn signature(topics: &serde_json::Value) -> Option<String> {
    topics
        .as_array()?
        .first()?
        .get("value")?
        .as_str()
        .map(str::to_string)
}

fn main() {
    if let Some(path) = std::env::args().nth(1) {
        audit_archive_ledger(&path);
        return;
    }
    let mut token_total = 0usize;
    let mut token_without_op = 0usize;
    let mut meta_version_note: Vec<&str> = Vec::new();

    for (name, b64) in CORPUS {
        let bytes = match base64_decode(b64.trim()) {
            Some(b) => b,
            None => {
                println!("{name}: base64 decode failed — SKIPPED");
                continue;
            }
        };
        let Ok(meta) = TransactionMeta::from_xdr(bytes, Limits::none()) else {
            println!("{name}: meta decode failed — SKIPPED");
            continue;
        };
        let version = match meta {
            TransactionMeta::V0(_) => "V0",
            TransactionMeta::V1(_) => "V1",
            TransactionMeta::V2(_) => "V2",
            TransactionMeta::V3(_) => "V3",
            TransactionMeta::V4(_) => "V4",
        };
        if version != "V4" {
            meta_version_note.push(name);
        }

        let events = extract_events(&meta, "0".repeat(64).as_str(), 0, 0);
        println!("\n=== {name}  (meta {version}, {} events)", events.len());
        for ev in &events {
            let sig = signature(&ev.topics).unwrap_or_else(|| "<none>".into());
            let is_token = TOKEN_VERBS.contains(&sig.as_str());
            let src = match ev.source {
                EventSource::TxLevel => "TxLevel",
                EventSource::PerOp => "PerOp",
                EventSource::Diagnostic => "Diagnostic",
            };
            if is_token && ev.source != EventSource::Diagnostic {
                token_total += 1;
                if ev.op_index.is_none() {
                    token_without_op += 1;
                }
            }
            println!(
                "  idx {:>3}  {:<11}  op_index {:>6}  {}{}",
                ev.event_index,
                src,
                ev.op_index.map_or("None".into(), |o| o.to_string()),
                sig,
                if is_token { "  <- token verb" } else { "" }
            );
        }
    }

    println!("\n---------------------------------------------");
    println!("token events (non-diagnostic) : {token_total}");
    println!("…of those WITHOUT op_index    : {token_without_op}");
    if !meta_version_note.is_empty() {
        println!("fixtures whose meta is not V4 : {meta_version_note:?}");
    }
    println!(
        "VERDICT: official identity is {} for the edge table's verbs in this corpus",
        if token_without_op == 0 {
            "TOTAL"
        } else {
            "NOT total"
        }
    );
}

/// Audit one archive ledger file (`<hex>--<seq>.xdr.zst` from the public
/// `aws-public-blockchain` dataset). Reports, over every transaction in the
/// ledger, whether any token-verb event lacks an operation index.
fn audit_archive_ledger(path: &str) {
    let compressed = std::fs::read(path).expect("read ledger file");
    let xdr = xdr_parser::decompress_zstd(&compressed).expect("zstd");
    let batch = xdr_parser::deserialize_batch(&xdr).expect("LedgerCloseMetaBatch");

    let mut token_total = 0usize;
    let mut token_without_op = 0usize;
    let mut tx_level_token = 0usize;
    let mut meta_versions: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut tx_seen = 0usize;

    for lcm in batch.ledger_close_metas.iter() {
        let (seq, metas) = ledger_tx_metas(lcm);
        println!("ledger {seq}: {} transactions", metas.len());
        for meta in metas {
            tx_seen += 1;
            *meta_versions.entry(meta_version(&meta)).or_default() += 1;
            for ev in extract_events(&meta, "0".repeat(64).as_str(), seq, 0) {
                if ev.source == EventSource::Diagnostic {
                    continue;
                }
                let Some(sig) = signature(&ev.topics) else {
                    continue;
                };
                if !TOKEN_VERBS.contains(&sig.as_str()) {
                    continue;
                }
                token_total += 1;
                if ev.source == EventSource::TxLevel {
                    tx_level_token += 1;
                }
                if ev.op_index.is_none() {
                    token_without_op += 1;
                    println!(
                        "  MISSING op_index: tx#{tx_seen} idx {} src {:?} {sig}",
                        ev.event_index, ev.source
                    );
                }
            }
        }
    }

    println!("\n---------------------------------------------");
    println!("transactions audited          : {tx_seen}");
    println!("meta versions                 : {meta_versions:?}");
    println!("token events (non-diagnostic) : {token_total}");
    println!("…at TRANSACTION level         : {tx_level_token}");
    println!("…WITHOUT op_index             : {token_without_op}");
    println!(
        "VERDICT: official identity is {} for the edge table's verbs in this ledger",
        if token_without_op == 0 {
            "TOTAL"
        } else {
            "NOT total"
        }
    );
}

fn meta_version(m: &TransactionMeta) -> &'static str {
    match m {
        TransactionMeta::V0(_) => "V0",
        TransactionMeta::V1(_) => "V1",
        TransactionMeta::V2(_) => "V2",
        TransactionMeta::V3(_) => "V3",
        TransactionMeta::V4(_) => "V4",
    }
}

/// Pull `(ledger sequence, per-transaction metas)` out of any
/// `LedgerCloseMeta` variant.
fn ledger_tx_metas(lcm: &stellar_xdr::LedgerCloseMeta) -> (u32, Vec<TransactionMeta>) {
    use stellar_xdr::LedgerCloseMeta as L;
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

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}
