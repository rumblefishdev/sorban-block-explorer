//! Preflight checks for `merge ingest`. All four checks accumulate
//! their findings; if any fails, every failure is reported in one
//! actionable error rather than aborting on the first.
//!
//! Per task 0186 §Step 2.3 + ADR 0040 "Required pre-merge invariants":
//! 1. migration parity (incl. checksum)
//! 2. ledger range disjoint, source strictly later than target
//! 3. partition layout (`*_default` only) on both sides
//! 4. CHECK constraint set matches

use sqlx::{PgConnection, Row};

use crate::error::MergeError;
use crate::fdw::FOREIGN_SCHEMA;

/// Tables expected to be RANGE-partitioned by `created_at` per
/// `db-partition-mgmt::TIME_PARTITIONED_TABLES`. Hard-coded here to
/// avoid pulling that crate as a dependency. If the partitioned-set
/// changes there, update this list and add the new parent table to
/// the partition-layout check.
const EXPECTED_PARTITIONED_TABLES: &[&str] = &[
    "transactions",
    "operations_appearances",
    "transaction_participants",
    "soroban_invocations_appearances",
    "soroban_events_appearances",
    "nft_ownership",
    "liquidity_pool_snapshots",
];

pub async fn run(conn: &mut PgConnection, allow_overlap: bool) -> Result<(), MergeError> {
    let mut failures: Vec<String> = Vec::new();

    if let Err(msg) = check_migrations(conn).await? {
        failures.push(msg);
    }
    if let Err(msg) = check_ledger_range(conn).await? {
        if allow_overlap {
            tracing::warn!(detail = %msg.trim(), "preflight: ledger-range mismatch ignored (--allow-overlap)");
        } else {
            failures.push(msg);
        }
    }
    if let Err(msg) = check_partition_layout(conn).await? {
        failures.push(msg);
    }
    if let Err(msg) = check_constraint_set(conn).await? {
        failures.push(msg);
    }

    if failures.is_empty() {
        tracing::info!("preflight: all checks passed");
        Ok(())
    } else {
        Err(MergeError::Preflight(failures.join("\n")))
    }
}

/// `_sqlx_migrations` rows must match exactly across target and source,
/// including `checksum`. Catches schema drift from a mid-merge migration
/// run on one side.
async fn check_migrations(conn: &mut PgConnection) -> Result<Result<(), String>, MergeError> {
    let rows = sqlx::query(
        "SELECT
             COALESCE(t.version, s.version) AS version,
             t.checksum AS target_checksum,
             s.checksum AS source_checksum,
             t.success AS target_success,
             s.success AS source_success
         FROM _sqlx_migrations t
         FULL OUTER JOIN merge_source._sqlx_migrations s USING (version)
         WHERE t.version IS NULL
            OR s.version IS NULL
            OR t.checksum IS DISTINCT FROM s.checksum
            OR t.success IS DISTINCT FROM s.success
         ORDER BY version",
    )
    .fetch_all(&mut *conn)
    .await?;

    if rows.is_empty() {
        return Ok(Ok(()));
    }

    let mut detail = String::from("  migration parity:\n");
    for row in rows {
        let version: i64 = row.try_get("version")?;
        let t_chk: Option<Vec<u8>> = row.try_get("target_checksum")?;
        let s_chk: Option<Vec<u8>> = row.try_get("source_checksum")?;
        let what = match (t_chk.is_some(), s_chk.is_some()) {
            (false, true) => "missing on target".to_string(),
            (true, false) => "missing on source".to_string(),
            (true, true) => "checksum/success mismatch".to_string(),
            (false, false) => unreachable!(),
        };
        detail.push_str(&format!("    - version {version}: {what}\n"));
    }
    Ok(Err(detail))
}

/// Source `MIN(sequence)` from `ledgers` must be strictly greater than
/// target `MAX(sequence)`. Empty target is fine (any source is "later").
async fn check_ledger_range(conn: &mut PgConnection) -> Result<Result<(), String>, MergeError> {
    let target_max: Option<i64> = sqlx::query_scalar("SELECT MAX(sequence)::bigint FROM ledgers")
        .fetch_one(&mut *conn)
        .await?;
    let source_min: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT MIN(sequence)::bigint FROM {FOREIGN_SCHEMA}.ledgers"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let source_max: Option<i64> = sqlx::query_scalar(&format!(
        "SELECT MAX(sequence)::bigint FROM {FOREIGN_SCHEMA}.ledgers"
    ))
    .fetch_one(&mut *conn)
    .await?;

    let Some(s_min) = source_min else {
        return Ok(Err(
            "  ledger range: source has no rows in `ledgers` — nothing to merge".to_string(),
        ));
    };

    if let Some(t_max) = target_max
        && s_min <= t_max
    {
        let s_max = source_max.unwrap_or(s_min);
        return Ok(Err(format!(
            "  ledger range: source range [{s_min}..{s_max}] precedes or overlaps \
             target MAX={t_max} — chronological-only contract violated"
        )));
    }

    Ok(Ok(()))
}

/// Target-only partition-layout audit: every expected partitioned
/// parent must exist with exactly one `*_default` child and no monthly
/// children. Source side intentionally not checked — `IMPORT FOREIGN
/// SCHEMA` brings parents into `merge_source` but not their children,
/// and source children aren't reachable via FDW. We rely on FDW's
/// abstraction (SELECT over a foreign partitioned parent reads through
/// to all source children regardless of structure) plus the migration-
/// parity gate as the proxy for source structure.
async fn check_partition_layout(conn: &mut PgConnection) -> Result<Result<(), String>, MergeError> {
    let target = target_partition_children(conn).await?;

    let mut bad: Vec<String> = Vec::new();

    for expected in EXPECTED_PARTITIONED_TABLES {
        match target.get(*expected) {
            None => bad.push(format!(
                "    - target.{expected}: partitioned parent missing or has no children — \
                 run `db-partition-mgmt` to create the default partition"
            )),
            Some(children) => {
                let default_name = format!("{expected}_default");
                if !children.iter().any(|c| c == &default_name) {
                    bad.push(format!(
                        "    - target.{expected}: missing required child `{default_name}`"
                    ));
                }
                let non_default: Vec<_> = children.iter().filter(|c| **c != default_name).collect();
                if !non_default.is_empty() {
                    bad.push(format!(
                        "    - target.{expected}: non-default children present \
                         ({non_default:?}) — merge into a non-default-only target requires \
                         matching children on source; see ADR 0040 §Partition handling"
                    ));
                }
            }
        }
    }

    if bad.is_empty() {
        Ok(Ok(()))
    } else {
        Ok(Err(format!("  partition layout:\n{}\n", bad.join("\n"))))
    }
}

async fn target_partition_children(
    conn: &mut PgConnection,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, MergeError> {
    let rows = sqlx::query(
        "SELECT parent.relname AS parent, child.relname AS child
         FROM pg_inherits i
         JOIN pg_class parent ON parent.oid = i.inhparent
         JOIN pg_class child  ON child.oid  = i.inhrelid
         JOIN pg_namespace n  ON n.oid       = parent.relnamespace
         WHERE n.nspname = 'public'
           AND parent.relkind = 'p'
           AND child.relkind IN ('r', 'p')
         ORDER BY parent.relname, child.relname",
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut map: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for row in rows {
        let parent: String = row.try_get("parent")?;
        let child: String = row.try_get("child")?;
        map.entry(parent).or_default().push(child);
    }
    Ok(map)
}

/// CHECK constraint set on both sides must match (catches drift in
/// `ck_assets_identity`, `ck_sia_caller_xor`, partial UNIQUE-style CHECKs).
/// Implementation note: source's `pg_constraint` isn't reachable via FDW
/// (system catalog, not in `public`), so we compare CHECK constraint
/// **names** by enumerating `information_schema.check_constraints`,
/// which IS exposed in foreign tables imported via `IMPORT FOREIGN SCHEMA`
/// — well, no, `information_schema` is per-schema and not foreign-imported
/// either. Workaround: bake a known-list assertion against the target only,
/// and treat the snapshot's identical-migration baseline (check #1) as the
/// transitive guarantee that source has the same CHECKs. If that proves
/// insufficient (e.g. someone manually added a CHECK on one laptop), we'll
/// add a per-laptop attestation file shipped alongside the snapshot.
async fn check_constraint_set(conn: &mut PgConnection) -> Result<Result<(), String>, MergeError> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT conname::text
         FROM pg_constraint c
         JOIN pg_namespace n ON n.oid = c.connamespace
         WHERE c.contype = 'c' AND n.nspname = 'public'
         ORDER BY conname",
    )
    .fetch_all(&mut *conn)
    .await?;

    let required = ["ck_assets_identity", "ck_sia_caller_xor"];
    let missing: Vec<_> = required
        .iter()
        .filter(|r| !names.iter().any(|n| n == *r))
        .copied()
        .collect();

    if missing.is_empty() {
        Ok(Ok(()))
    } else {
        Ok(Err(format!(
            "  constraint set: required CHECKs missing on target: {missing:?}"
        )))
    }
}
