//! Ledger header extraction from LedgerCloseMeta.

use stellar_xdr::curr::*;

use crate::types::ExtractedLedger;

/// Extract structured ledger data from a LedgerCloseMeta.
///
/// Infallible: every field is read directly from the meta. The ledger hash
/// is `LedgerHeaderHistoryEntry.hash` — the canonical value already
/// populated by stellar-core (matches Horizon `/ledgers/:N.hash` and every
/// other Stellar tool).
pub fn extract_ledger(meta: &LedgerCloseMeta) -> ExtractedLedger {
    let header_entry = ledger_header_entry(meta);
    let header = &header_entry.header;

    let hash = hex::encode(header_entry.hash.0);
    let closed_at = header.scp_value.close_time.0 as i64; // safe: Unix seconds fit i64
    let transaction_count: u32 = tx_count(meta).try_into().unwrap_or(u32::MAX);

    ExtractedLedger {
        sequence: header.ledger_seq,
        hash,
        closed_at,
        protocol_version: header.ledger_version,
        transaction_count,
        base_fee: header.base_fee,
    }
}

/// Get the ledger header entry from any LedgerCloseMeta variant.
fn ledger_header_entry(meta: &LedgerCloseMeta) -> &LedgerHeaderHistoryEntry {
    match meta {
        LedgerCloseMeta::V0(v) => &v.ledger_header,
        LedgerCloseMeta::V1(v) => &v.ledger_header,
        LedgerCloseMeta::V2(v) => &v.ledger_header,
    }
}

/// Get the transaction count from the processing results.
fn tx_count(meta: &LedgerCloseMeta) -> usize {
    match meta {
        LedgerCloseMeta::V0(v) => v.tx_processing.len(),
        LedgerCloseMeta::V1(v) => v.tx_processing.len(),
        LedgerCloseMeta::V2(v) => v.tx_processing.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the smallest LedgerCloseMetaV0 that round-trips through the
    /// parser, with a caller-controlled `entry.hash` so the test can assert
    /// the parser surfaces that exact value.
    fn synthetic_meta(entry_hash: [u8; 32], ledger_seq: u32) -> LedgerCloseMeta {
        let header = LedgerHeader {
            ledger_version: 23,
            previous_ledger_hash: Hash([0; 32]),
            scp_value: StellarValue {
                tx_set_hash: Hash([0; 32]),
                close_time: TimePoint(1_700_000_000),
                upgrades: VecM::default(),
                ext: StellarValueExt::Basic,
            },
            tx_set_result_hash: Hash([0; 32]),
            bucket_list_hash: Hash([0; 32]),
            ledger_seq,
            total_coins: 0,
            fee_pool: 0,
            inflation_seq: 0,
            id_pool: 0,
            base_fee: 100,
            base_reserve: 0,
            max_tx_set_size: 0,
            skip_list: [Hash([0; 32]), Hash([0; 32]), Hash([0; 32]), Hash([0; 32])],
            ext: LedgerHeaderExt::V0,
        };

        LedgerCloseMeta::V0(LedgerCloseMetaV0 {
            ledger_header: LedgerHeaderHistoryEntry {
                hash: Hash(entry_hash),
                header,
                ext: LedgerHeaderHistoryEntryExt::V0,
            },
            tx_set: TransactionSet {
                previous_ledger_hash: Hash([0; 32]),
                txs: VecM::default(),
            },
            tx_processing: VecM::default(),
            upgrades_processing: VecM::default(),
            scp_info: VecM::default(),
        })
    }

    #[test]
    fn extract_ledger_returns_canonical_entry_hash() {
        // Distinct byte pattern per index so any off-by-one slicing in
        // `hex::encode` would change the resulting hex string.
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        let meta = synthetic_meta(bytes, 12_345);

        let extracted = extract_ledger(&meta);

        assert_eq!(extracted.hash, hex::encode(bytes));
        assert_eq!(extracted.sequence, 12_345);
    }

    #[test]
    fn extract_ledger_does_not_recompute_hash() {
        // If extraction ever falls back to SHA256(header_xdr) the result
        // will be deterministic but unrelated to the canonical 0xAB pattern.
        let canonical = [0xAB; 32];
        let meta = synthetic_meta(canonical, 1);

        let extracted = extract_ledger(&meta);

        assert_eq!(
            extracted.hash,
            "ab".repeat(32),
            "parser must surface entry.hash.0 verbatim, not a derived digest"
        );
    }
}
