/// An inclusive ledger range [start, end].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LedgerRange {
    pub start: u32,
    pub end: u32,
}

impl LedgerRange {
    pub fn new(start: u32, end: u32) -> Self {
        assert!(start <= end, "start ({start}) must be <= end ({end})");
        Self { start, end }
    }

    pub fn len(&self) -> u32 {
        self.end - self.start + 1
    }
}

impl std::fmt::Display for LedgerRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}..{}] ({} ledgers)", self.start, self.end, self.len())
    }
}

/// Split a total range into batches of at most `batch_size` ledgers.
pub fn split_into_batches(total: LedgerRange, batch_size: u32) -> Vec<LedgerRange> {
    assert!(batch_size > 0, "batch_size must be > 0");
    let mut batches = Vec::new();
    let mut cursor = total.start;
    while cursor <= total.end {
        let batch_end = cursor.saturating_add(batch_size - 1).min(total.end);
        batches.push(LedgerRange::new(cursor, batch_end));
        cursor = batch_end + 1;
    }
    batches
}

/// Merge overlapping or adjacent ranges into a minimal sorted set.
fn merge_ranges(mut ranges: Vec<LedgerRange>) -> Vec<LedgerRange> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<LedgerRange> = vec![ranges[0]];
    for r in &ranges[1..] {
        let last = merged.last_mut().unwrap();
        // Adjacent ranges (end + 1 == start) should also merge.
        if r.start <= last.end.saturating_add(1) {
            last.end = last.end.max(r.end);
        } else {
            merged.push(*r);
        }
    }
    merged
}

/// Find gaps within `total` that are not covered by any range in `covered`.
/// Returns the uncovered segments, each split into `batch_size` chunks.
pub fn find_gaps(total: LedgerRange, covered: &[LedgerRange], batch_size: u32) -> Vec<LedgerRange> {
    let merged = merge_ranges(covered.to_vec());
    let mut gaps = Vec::new();
    let mut cursor = total.start;

    for r in &merged {
        let cov_start = r.start.max(total.start);
        let cov_end = r.end.min(total.end);
        if cov_start > cov_end || cov_start > total.end {
            continue;
        }
        if cursor < cov_start {
            gaps.push(LedgerRange::new(cursor, cov_start - 1));
        }
        cursor = cov_end + 1;
    }

    if cursor <= total.end {
        gaps.push(LedgerRange::new(cursor, total.end));
    }

    gaps.into_iter()
        .flat_map(|gap| split_into_batches(gap, batch_size))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_exact_multiple() {
        let batches = split_into_batches(LedgerRange::new(1, 10), 5);
        assert_eq!(
            batches,
            vec![LedgerRange::new(1, 5), LedgerRange::new(6, 10)]
        );
    }

    #[test]
    fn split_with_remainder() {
        let batches = split_into_batches(LedgerRange::new(1, 7), 3);
        assert_eq!(
            batches,
            vec![
                LedgerRange::new(1, 3),
                LedgerRange::new(4, 6),
                LedgerRange::new(7, 7),
            ]
        );
    }

    #[test]
    fn split_single_ledger() {
        let batches = split_into_batches(LedgerRange::new(5, 5), 100);
        assert_eq!(batches, vec![LedgerRange::new(5, 5)]);
    }

    #[test]
    fn split_batch_larger_than_range() {
        let batches = split_into_batches(LedgerRange::new(1, 3), 100);
        assert_eq!(batches, vec![LedgerRange::new(1, 3)]);
    }

    #[test]
    fn gaps_no_coverage() {
        let gaps = find_gaps(LedgerRange::new(1, 100), &[], 50);
        assert_eq!(
            gaps,
            vec![LedgerRange::new(1, 50), LedgerRange::new(51, 100)]
        );
    }

    #[test]
    fn gaps_full_coverage() {
        let covered = vec![LedgerRange::new(1, 100)];
        let gaps = find_gaps(LedgerRange::new(1, 100), &covered, 50);
        assert!(gaps.is_empty());
    }

    #[test]
    fn gaps_partial_coverage() {
        let covered = vec![LedgerRange::new(20, 40), LedgerRange::new(60, 80)];
        let gaps = find_gaps(LedgerRange::new(1, 100), &covered, 100);
        assert_eq!(
            gaps,
            vec![
                LedgerRange::new(1, 19),
                LedgerRange::new(41, 59),
                LedgerRange::new(81, 100),
            ]
        );
    }

    #[test]
    fn gaps_overlapping_coverage() {
        let covered = vec![
            LedgerRange::new(10, 30),
            LedgerRange::new(25, 50),
            LedgerRange::new(70, 90),
        ];
        let gaps = find_gaps(LedgerRange::new(1, 100), &covered, 100);
        assert_eq!(
            gaps,
            vec![
                LedgerRange::new(1, 9),
                LedgerRange::new(51, 69),
                LedgerRange::new(91, 100),
            ]
        );
    }

    #[test]
    fn gaps_adjacent_coverage() {
        let covered = vec![LedgerRange::new(1, 50), LedgerRange::new(51, 100)];
        let gaps = find_gaps(LedgerRange::new(1, 100), &covered, 50);
        assert!(gaps.is_empty());
    }

    #[test]
    fn gaps_coverage_extends_beyond_total() {
        let covered = vec![LedgerRange::new(1, 200)];
        let gaps = find_gaps(LedgerRange::new(50, 100), &covered, 50);
        assert!(gaps.is_empty());
    }

    #[test]
    fn gaps_split_into_batches() {
        let covered = vec![LedgerRange::new(30, 50)];
        let gaps = find_gaps(LedgerRange::new(1, 100), &covered, 20);
        assert_eq!(
            gaps,
            vec![
                LedgerRange::new(1, 20),
                LedgerRange::new(21, 29),
                LedgerRange::new(51, 70),
                LedgerRange::new(71, 90),
                LedgerRange::new(91, 100),
            ]
        );
    }

    #[test]
    fn merge_ranges_unsorted() {
        let merged = merge_ranges(vec![
            LedgerRange::new(50, 60),
            LedgerRange::new(10, 20),
            LedgerRange::new(15, 55),
        ]);
        assert_eq!(merged, vec![LedgerRange::new(10, 60)]);
    }

    #[test]
    fn ledger_range_display() {
        let r = LedgerRange::new(100, 200);
        assert_eq!(format!("{r}"), "[100..200] (101 ledgers)");
    }

    #[test]
    fn realistic_backfill_resume_scenario() {
        let soroban_start = 50_457_424u32;
        let target_end = 50_507_424u32;
        let batch_size = 10_000u32;

        let existing = vec![
            LedgerRange::new(50_457_424, 50_467_423),
            LedgerRange::new(50_467_424, 50_477_423),
            LedgerRange::new(50_477_424, 50_480_000),
        ];

        let total = LedgerRange::new(soroban_start, target_end);
        let gaps = find_gaps(total, &existing, batch_size);

        assert_eq!(gaps[0].start, 50_480_001);
        for window in gaps.windows(2) {
            assert_eq!(window[0].end + 1, window[1].start);
        }
        let covered: u32 = existing.iter().map(|r| r.len()).sum();
        let gap_ledgers: u32 = gaps.iter().map(|r| r.len()).sum();
        assert_eq!(covered + gap_ledgers, total.len());
    }

    #[test]
    fn batches_produce_non_overlapping_ranges() {
        let total = LedgerRange::new(50_457_424, 50_557_424);
        let batches = split_into_batches(total, 10_000);

        for window in batches.windows(2) {
            assert!(window[0].end < window[1].start);
            assert_eq!(window[0].end + 1, window[1].start);
        }
        assert_eq!(batches.first().unwrap().start, total.start);
        assert_eq!(batches.last().unwrap().end, total.end);
        let total_ledgers: u32 = batches.iter().map(|r| r.len()).sum();
        assert_eq!(total_ledgers, total.len());
    }
}
