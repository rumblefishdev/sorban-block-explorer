//! One-off audit (task 0540, T05): how often does a transaction carry a memo,
//! how many bytes is it, and how often is a payment destination or a source
//! account muxed (`M…`)? Read from the ENVELOPE, which is where both live
//! unambiguously — the CAP-67 event field `to_muxed_id` merges the two.
//!
//! Sizes `transaction_memos` and the two `*_muxed_id` columns before the
//! S3 pass writes them.
//!
//! Usage:
//!   cargo run -q -p xdr-parser --example memo_mux_audit -- <ledger.xdr.zst>...
use std::collections::BTreeMap;

use stellar_xdr::{LedgerCloseMeta, Memo, MuxedAccount, OperationBody};
use xdr_parser::envelope::{extract_envelopes, inner_transaction};

fn ledger_seq(lcm: &LedgerCloseMeta) -> u32 {
    match lcm {
        LedgerCloseMeta::V0(v) => v.ledger_header.header.ledger_seq,
        LedgerCloseMeta::V1(v) => v.ledger_header.header.ledger_seq,
        LedgerCloseMeta::V2(v) => v.ledger_header.header.ledger_seq,
    }
}

fn is_muxed(m: &MuxedAccount) -> bool {
    matches!(m, MuxedAccount::MuxedEd25519(_))
}

fn main() {
    let net_id = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);
    let mut txs = 0usize;
    let mut missing_envelope = 0usize;
    let mut memo_by_type: BTreeMap<&str, usize> = BTreeMap::new();
    let mut memo_bytes = 0usize;
    let mut txs_with_memo = 0usize;
    let mut muxed_tx_source = 0usize;
    let mut muxed_op_source = 0usize;
    let mut ops = 0usize;
    let mut muxed_destinations = 0usize;
    let mut destinations = 0usize;

    for path in std::env::args().skip(1) {
        let compressed = std::fs::read(&path).expect("read ledger file");
        let xdr = xdr_parser::decompress_zstd(&compressed).expect("zstd");
        let batch = xdr_parser::deserialize_batch(&xdr).expect("LedgerCloseMetaBatch");
        for lcm in batch.ledger_close_metas.iter() {
            let envelopes = extract_envelopes(lcm, &net_id);
            let mut ledger_memo = 0usize;
            for env in &envelopes {
                txs += 1;
                let Some(env) = env else {
                    missing_envelope += 1;
                    continue;
                };
                let inner = inner_transaction(env);
                let (kind, bytes) = match inner.memo() {
                    Memo::None => ("none", 0),
                    Memo::Text(t) => ("text", t.len()),
                    Memo::Id(_) => ("id", 8),
                    Memo::Hash(_) => ("hash", 32),
                    Memo::Return(_) => ("return", 32),
                };
                *memo_by_type.entry(kind).or_default() += 1;
                if kind != "none" {
                    txs_with_memo += 1;
                    ledger_memo += 1;
                    memo_bytes += bytes;
                }
                // Source accounts: the transaction's and each operation's.
                let (src_muxed, op_list) = match &inner {
                    xdr_parser::envelope::InnerTxRef::V0(tx) => (false, tx.operations.as_slice()),
                    xdr_parser::envelope::InnerTxRef::V1(tx) => {
                        (is_muxed(&tx.source_account), tx.operations.as_slice())
                    }
                };
                if src_muxed {
                    muxed_tx_source += 1;
                }
                for op in op_list {
                    ops += 1;
                    if op.source_account.as_ref().is_some_and(is_muxed) {
                        muxed_op_source += 1;
                    }
                    // Every classic destination that CAP-27 allows to be muxed.
                    let dest: Option<&MuxedAccount> = match &op.body {
                        OperationBody::Payment(p) => Some(&p.destination),
                        OperationBody::PathPaymentStrictReceive(p) => Some(&p.destination),
                        OperationBody::PathPaymentStrictSend(p) => Some(&p.destination),
                        OperationBody::AccountMerge(d) => Some(d),
                        _ => None,
                    };
                    if let Some(d) = dest {
                        destinations += 1;
                        if is_muxed(d) {
                            muxed_destinations += 1;
                        }
                    }
                }
            }
            println!(
                "ledger {}: {} txs, {} with memo",
                ledger_seq(lcm),
                envelopes.len(),
                ledger_memo
            );
        }
    }

    let pct = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    println!("\n---------------------------------------------");
    println!("transactions                  : {txs} (envelope missing: {missing_envelope})");
    println!(
        "with memo                     : {txs_with_memo} ({:.1}%)",
        pct(txs_with_memo, txs)
    );
    println!("memo by type                  : {memo_by_type:?}");
    println!(
        "memo payload bytes, total/avg  : {memo_bytes} / {:.1} per memo",
        if txs_with_memo == 0 {
            0.0
        } else {
            memo_bytes as f64 / txs_with_memo as f64
        }
    );
    println!(
        "muxed tx source                : {muxed_tx_source} ({:.2}% of txs)",
        pct(muxed_tx_source, txs)
    );
    println!(
        "muxed op source                : {muxed_op_source} ({:.2}% of {ops} ops)",
        pct(muxed_op_source, ops)
    );
    println!(
        "muxed destination              : {muxed_destinations} ({:.2}% of {destinations} classic destinations)",
        pct(muxed_destinations, destinations)
    );
}
