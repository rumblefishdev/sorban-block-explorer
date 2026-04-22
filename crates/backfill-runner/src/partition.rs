//! Stellar public-archive S3 partition math.
//!
//! Layout: `v1.1/stellar/ledgers/pubnet/{HEX}--{start}-{end}/{HEX}--{seq}.xdr.zst`
//! where `HEX = uppercase_hex(u32::MAX - seq_or_start)` zero-padded to 8 chars,
//! and each partition folder holds exactly `PARTITION_SIZE` ledgers.

use std::path::{Path, PathBuf};

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

    /// S3 key prefix (no bucket, no scheme, no trailing slash):
    /// `v1.1/stellar/ledgers/pubnet/FC4DB5FF--62016000-62079999`.
    pub fn folder_key(&self) -> String {
        format!("{ROOT_PREFIX}/{}--{}-{}", self.hex, self.start, self.end)
    }

    /// Full `s3://` URL for the partition folder, suitable as the source
    /// argument to `aws s3 sync`. Trailing slash is intentional — the AWS
    /// CLI treats it as "sync directory contents" rather than "sync one
    /// object".
    pub fn s3_folder(&self) -> String {
        format!("s3://{BUCKET}/{}/", self.folder_key())
    }

    /// Local directory where this partition's `.xdr.zst` files land after
    /// `aws s3 sync` — `{temp_dir}/{HEX}--{start}-{end}`.
    ///
    /// Directory name intentionally matches the S3 folder name so an
    /// operator can `ls` the temp dir and immediately see which partition
    /// each dir represents.
    pub fn local_folder(&self, temp_dir: &Path) -> PathBuf {
        temp_dir.join(format!("{}--{}-{}", self.hex, self.start, self.end))
    }

    /// Intersect this partition's `[start, end]` with a run's requested
    /// `[run_start, run_end]`. Returned bounds are inclusive.
    ///
    /// A partition at either edge of the run range may only partially
    /// overlap it — every site that loops ledgers or checks "is this
    /// partition fully in the DB" needs these clamped bounds. Centralized
    /// here so the inclusive math isn't duplicated (and re-debugged) at
    /// each call site.
    ///
    /// Caller must ensure `run_start <= run_end` and the partition at
    /// least partially overlaps the run range; otherwise the returned
    /// pair may have `first > last`.
    pub fn clamped(&self, run_start: u32, run_end: u32) -> (u32, u32) {
        (run_start.max(self.start), run_end.min(self.end))
    }
}

/// Local filesystem path for a single ledger within its partition's local
/// folder: `{temp_dir}/{HEX}--{start}-{end}/{HEX}--{seq}.xdr.zst`.
///
/// Filename layout mirrors the S3 layout 1:1 so `aws s3 sync` produces
/// exactly these paths without transformation.
pub fn local_ledger_path(partition: &Partition, seq: u32, temp_dir: &Path) -> PathBuf {
    let file_hex = format!("{:08X}", u32::MAX - seq);
    partition
        .local_folder(temp_dir)
        .join(format!("{file_hex}--{seq}.xdr.zst"))
}

/// Enumerate the partitions covering `[start, end]` inclusive, in
/// ascending sequence order. Returns an empty `Vec` if `start > end`.
///
/// Each returned partition may overflow the requested range at its edges
/// — the caller is expected to clamp per-ledger iteration to `[start, end]`.
pub fn partitions_for_range(start: u32, end: u32) -> Vec<Partition> {
    let mut result = Vec::new();
    if start > end {
        return result;
    }
    let mut cursor = start;
    while cursor <= end {
        let p = Partition::from_ledger(cursor);
        let next = p.end + 1;
        result.push(p);
        cursor = next;
    }
    result
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

    #[test]
    fn s3_folder_has_scheme_bucket_and_trailing_slash() {
        let p = Partition::from_ledger(62_026_937);
        assert_eq!(
            p.s3_folder(),
            "s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/FC4DB5FF--62016000-62079999/"
        );
    }

    #[test]
    fn local_folder_under_temp_dir() {
        let p = Partition::from_ledger(62_026_937);
        let temp = Path::new("/var/tmp/backfill");
        assert_eq!(
            p.local_folder(temp),
            PathBuf::from("/var/tmp/backfill/FC4DB5FF--62016000-62079999")
        );
    }

    #[test]
    fn local_ledger_path_joins_partition_and_file_name() {
        let p = Partition::from_ledger(62_026_937);
        let temp = Path::new("/var/tmp/backfill");
        assert_eq!(
            local_ledger_path(&p, 62_026_937, temp),
            PathBuf::from(
                "/var/tmp/backfill/FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst"
            )
        );
    }

    #[test]
    fn partitions_for_range_single_partition_when_both_bounds_inside() {
        let ps = partitions_for_range(62_020_000, 62_025_000);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].start, 62_016_000);
        assert_eq!(ps[0].end, 62_079_999);
    }

    #[test]
    fn partitions_for_range_start_equal_end_yields_one() {
        let ps = partitions_for_range(50_457_424, 50_457_424);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].start, 50_432_000);
    }

    #[test]
    fn partitions_for_range_spans_three_partitions() {
        // Inside first, across second, into third.
        let ps = partitions_for_range(62_020_000, 62_150_000);
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[0].start, 62_016_000);
        assert_eq!(ps[1].start, 62_080_000);
        assert_eq!(ps[2].start, 62_144_000);
    }

    #[test]
    fn partitions_for_range_end_exactly_on_boundary() {
        // end = last ledger of a partition → that partition is included, not
        // the next one.
        let ps = partitions_for_range(62_016_000, 62_079_999);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].end, 62_079_999);
    }

    #[test]
    fn partitions_for_range_end_just_past_boundary_includes_next() {
        let ps = partitions_for_range(62_016_000, 62_080_000);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[1].start, 62_080_000);
    }

    #[test]
    fn partitions_for_range_empty_when_start_gt_end() {
        let ps = partitions_for_range(100, 50);
        assert!(ps.is_empty());
    }
}
