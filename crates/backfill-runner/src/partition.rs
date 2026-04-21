//! Stellar public-archive S3 partition math.
//!
//! Layout: `v1.1/stellar/ledgers/pubnet/{HEX}--{start}-{end}/{HEX}--{seq}.xdr.zst`
//! where `HEX = uppercase_hex(u32::MAX - seq_or_start)` zero-padded to 8 chars,
//! and each partition folder holds exactly `PARTITION_SIZE` ledgers.

/// Root prefix inside `aws-public-blockchain`.
pub const BUCKET: &str = "aws-public-blockchain";
pub const ROOT_PREFIX: &str = "v1.1/stellar/ledgers/pubnet";
pub const PARTITION_SIZE: u32 = 64_000;

/// S3 partition folder covering a given ledger sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub start: u32,
    pub end: u32,
    pub hex: String,
}

impl Partition {
    pub fn from_ledger(seq: u32) -> Self {
        let start = seq - (seq % PARTITION_SIZE);
        let end = start + PARTITION_SIZE - 1;
        let hex = format!("{:08X}", u32::MAX - start);
        Self { start, end, hex }
    }

    /// Folder key (no trailing slash): `v1.1/.../FC4DB5FF--62016000-62079999`.
    pub fn folder_key(&self) -> String {
        format!("{ROOT_PREFIX}/{}--{}-{}", self.hex, self.start, self.end)
    }
}

/// Full S3 object key for a single ledger in the given partition.
pub fn ledger_key(partition: &Partition, seq: u32) -> String {
    let file_hex = format!("{:08X}", u32::MAX - seq);
    format!("{}/{file_hex}--{seq}.xdr.zst", partition.folder_key())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_bounds() {
        let p = Partition::from_ledger(62_026_937);
        assert_eq!(p.start, 62_016_000);
        assert_eq!(p.end, 62_079_999);
        assert_eq!(p.hex, "FC4DB5FF");
    }

    #[test]
    fn partition_folder_key() {
        let p = Partition::from_ledger(62_026_937);
        assert_eq!(
            p.folder_key(),
            "v1.1/stellar/ledgers/pubnet/FC4DB5FF--62016000-62079999"
        );
    }

    #[test]
    fn ledger_key_matches_archive_layout() {
        let p = Partition::from_ledger(62_026_937);
        assert_eq!(
            ledger_key(&p, 62_026_937),
            "v1.1/stellar/ledgers/pubnet/FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst"
        );
    }

    /// First Soroban-era ledger (Protocol 20 go-live, 2024-02-20). Guards
    /// against off-by-one in the hex math at a known-good real sequence.
    #[test]
    fn soroban_start_ledger_partition() {
        let p = Partition::from_ledger(50_457_424);
        assert_eq!(p.start, 50_432_000);
        assert_eq!(p.end, 50_495_999);
        assert_eq!(p.hex, "FCFE77FF");
    }

    /// A sequence exactly on a partition boundary must land in the new
    /// partition (start == seq), not the previous one.
    #[test]
    fn partition_boundary_start_is_inclusive() {
        let p = Partition::from_ledger(62_016_000);
        assert_eq!(p.start, 62_016_000);
        assert_eq!(p.end, 62_079_999);
    }

    /// A sequence at the last slot of a partition must stay in that
    /// partition, not roll forward.
    #[test]
    fn partition_boundary_end_is_inclusive() {
        let p = Partition::from_ledger(62_079_999);
        assert_eq!(p.start, 62_016_000);
        assert_eq!(p.end, 62_079_999);
    }

    /// Adjacent ledgers that straddle a partition boundary must produce
    /// different partitions — catches modular-arithmetic regressions.
    #[test]
    fn adjacent_ledgers_across_boundary_differ() {
        let prev = Partition::from_ledger(62_015_999);
        let next = Partition::from_ledger(62_016_000);
        assert_ne!(prev, next);
        assert_eq!(prev.end + 1, next.start);
    }
}
