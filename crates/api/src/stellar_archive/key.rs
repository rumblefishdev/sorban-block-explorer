//! Forward mapping: ledger sequence → Galexie S3 key.
//!
//! The inverse of `xdr_parser::parse_s3_key`. Galexie partitions ledgers into
//! folders of 64,000 ledgers each. Hex prefixes are derived from `u32::MAX - N`
//! (reverse-sorted listing convention).
//!
//! The public Stellar archive at `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
//! writes one ledger per file (`ledgersPerBatch=1`), so the forward map
//! always produces `{file_hex}--{ledger}.xdr.zst` without a range suffix.

/// Number of ledgers grouped into a single partition folder.
pub const PARTITION_SIZE: u32 = 64_000;

/// Build the S3 object key for a given ledger sequence in the public archive.
///
/// Example: `ledger 62_026_937` → `FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst`
pub fn build_s3_key(ledger_seq: u32) -> String {
    let partition_start = ledger_seq - (ledger_seq % PARTITION_SIZE);
    let partition_end = partition_start.saturating_add(PARTITION_SIZE - 1);
    let partition_hex = format!("{:08X}", u32::MAX - partition_start);
    let file_hex = format!("{:08X}", u32::MAX - ledger_seq);
    format!("{partition_hex}--{partition_start}-{partition_end}/{file_hex}--{ledger_seq}.xdr.zst")
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdr_parser::parse_s3_key;

    #[test]
    fn round_trip_soroban_era() {
        let seq = 62_026_937;
        let key = build_s3_key(seq);
        assert_eq!(
            key,
            "FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst"
        );
        let (start, end) = parse_s3_key(&key).unwrap();
        assert_eq!(start, seq);
        assert_eq!(end, seq);
    }

    #[test]
    fn round_trip_first_soroban_ledger() {
        let seq = 50_457_424;
        let key = build_s3_key(seq);
        let (start, end) = parse_s3_key(&key).unwrap();
        assert_eq!(start, seq);
        assert_eq!(end, seq);
    }

    #[test]
    fn partition_boundary_start() {
        // Ledger 64000 starts a new partition
        let key = build_s3_key(64_000);
        assert!(
            key.starts_with("FFFF05FF--64000-127999/"),
            "unexpected key: {key}"
        );
        let (start, _) = parse_s3_key(&key).unwrap();
        assert_eq!(start, 64_000);
    }

    #[test]
    fn partition_boundary_end() {
        // Ledger 63999 is last in first partition
        let key = build_s3_key(63_999);
        assert!(
            key.starts_with("FFFFFFFF--0-63999/"),
            "unexpected key: {key}"
        );
        let (start, _) = parse_s3_key(&key).unwrap();
        assert_eq!(start, 63_999);
    }

    #[test]
    fn ledger_zero() {
        let key = build_s3_key(0);
        let (start, end) = parse_s3_key(&key).unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }
}
