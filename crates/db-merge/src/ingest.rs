//! Orchestrator for `merge ingest`. Phase C wires snapshot reset →
//! pg_restore → FDW setup → preflight → pre-merge pg_dump → FDW teardown.
//! The actual topological merge SQL (Phase D) plugs in between the
//! pg_dump and the teardown.

use std::path::Path;

use sqlx::Connection;

use crate::backup;
use crate::error::MergeError;
use crate::fdw;
use crate::preflight;
use crate::snapshot_source;

/// Stable advisory lock key for db-merge ingest. Picked once and never
/// changed — collisions with other tooling are vanishingly unlikely
/// across `pg_advisory_lock`'s 64-bit space.
const ADVISORY_LOCK_KEY: i64 = 0x44_42_4D_45_52_47_45_31; // "DBMERGE1" in ASCII

pub async fn execute(
    snapshot: &Path,
    target_url: &str,
    snapshot_source_url: &str,
    allow_overlap: bool,
) -> Result<(), MergeError> {
    let mut target = sqlx::PgConnection::connect(target_url).await?;

    // Hold an advisory lock for the entire ingest to forbid concurrent
    // invocations (which would race on FDW objects + remap tables).
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .fetch_one(&mut target)
        .await?;
    if !acquired {
        return Err(MergeError::ConcurrentInvocation);
    }

    let result = run_inner(
        &mut target,
        snapshot,
        target_url,
        snapshot_source_url,
        allow_overlap,
    )
    .await;

    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut target)
        .await
    {
        tracing::warn!(error = %e, "failed to release advisory lock");
    }

    result
}

async fn run_inner(
    target: &mut sqlx::PgConnection,
    snapshot: &Path,
    target_url: &str,
    snapshot_source_url: &str,
    allow_overlap: bool,
) -> Result<(), MergeError> {
    snapshot_source::reset_and_restore(snapshot, snapshot_source_url).await?;

    fdw::setup(target).await?;

    if let Err(e) = preflight::run(target, allow_overlap).await {
        let _ = fdw::teardown(target).await;
        return Err(e);
    }

    backup::dump_target(target_url).await?;

    // Phase D: topological merge. D1 implements the simple steps (no
    // remap, no FK rewrite); D2/D3 fill the gaps in `steps::execute`.
    crate::steps::execute(target).await?;

    fdw::teardown(target).await?;

    // Stop the snapshot-source container; volume is left for inspection.
    snapshot_source::stop().await?;

    Ok(())
}
