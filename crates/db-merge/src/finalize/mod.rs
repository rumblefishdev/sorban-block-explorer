//! `merge finalize` — Step 13 + Step 14 from task 0186 §Step 3.
//! Runs once after the last `merge ingest`; idempotent.

use sqlx::Connection;

use crate::error::MergeError;

pub mod nfts_current_owner;
pub mod sequences;

/// Same key as `ingest::ADVISORY_LOCK_KEY` — finalize must not race a
/// concurrent ingest on the same target.
const ADVISORY_LOCK_KEY: i64 = 0x44_42_4D_45_52_47_45_31;

pub async fn execute(target_url: &str) -> Result<(), MergeError> {
    let mut target = sqlx::PgConnection::connect(target_url).await?;

    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .fetch_one(&mut target)
        .await?;
    if !acquired {
        return Err(MergeError::ConcurrentInvocation);
    }

    let result = run_inner(&mut target).await;

    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut target)
        .await
    {
        tracing::warn!(error = %e, "failed to release advisory lock");
    }

    result
}

async fn run_inner(conn: &mut sqlx::PgConnection) -> Result<(), MergeError> {
    nfts_current_owner::run(conn).await?;
    let n = sequences::run(conn).await?;
    tracing::info!(adjusted = n, "finalize complete");
    Ok(())
}
