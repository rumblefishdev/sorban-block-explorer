//! DB-only resume: which sequences in `[start, end]` are already in `ledgers`?
//!
//! Single batch query at startup — no watermark file, no side-channel state.
//! The `ledgers` table (ADR 0027) is the single source of truth.

use sqlx::PgPool;
use std::collections::HashSet;
use tracing::info;

use crate::error::BackfillError;

/// Load sequences already present in `ledgers` within `[start, end]`.
pub async fn load_completed(
    pool: &PgPool,
    start: u32,
    end: u32,
) -> Result<HashSet<u32>, BackfillError> {
    let rows: Vec<i64> =
        sqlx::query_scalar("SELECT sequence FROM ledgers WHERE sequence BETWEEN $1 AND $2")
            .bind(start as i64)
            .bind(end as i64)
            .fetch_all(pool)
            .await?;

    let set: HashSet<u32> = rows.into_iter().map(|s| s as u32).collect();
    info!(
        start,
        end,
        completed = set.len(),
        total = end - start + 1,
        "resume state loaded"
    );
    Ok(set)
}

#[cfg(test)]
mod tests {
    //! DB-gated: skips cleanly when `DATABASE_URL` is unset or unreachable so
    //! `cargo test -p backfill-runner` doesn't fail in CI without Postgres.
    //!
    //! Run locally:
    //!   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
    //!       cargo test -p backfill-runner --lib -- --test-threads=1
    //!
    //! A fixture range well above realistic Soroban sequences (`TEST_BASE`)
    //! avoids collisions with real rows on a shared staging DB.
    use super::*;
    use sqlx::PgPool;

    /// Far above any realistic Soroban sequence — fits in both BIGINT and u32.
    const TEST_BASE: u32 = 4_000_000_000;

    async fn connect() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok()?;
        match PgPool::connect(&url).await {
            Ok(p) => Some(p),
            Err(err) => {
                eprintln!("DATABASE_URL unreachable ({err}) — skipping resume test");
                None
            }
        }
    }

    async fn cleanup(pool: &PgPool, start: u32, end: u32) {
        let _ = sqlx::query("DELETE FROM ledgers WHERE sequence BETWEEN $1 AND $2")
            .bind(i64::from(start))
            .bind(i64::from(end))
            .execute(pool)
            .await;
    }

    async fn insert_ledger(pool: &PgPool, seq: u32) {
        // 32-byte hash derived from seq so every row has a unique hash
        // without colliding across tests.
        let mut hash = [0u8; 32];
        hash[28..32].copy_from_slice(&seq.to_be_bytes());
        sqlx::query(
            "INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee)
             VALUES ($1, $2, to_timestamp(0), 22, 0, 100)
             ON CONFLICT (sequence) DO NOTHING",
        )
        .bind(i64::from(seq))
        .bind(hash.to_vec())
        .execute(pool)
        .await
        .expect("insert fixture ledger");
    }

    #[tokio::test]
    async fn empty_range_returns_empty_set() {
        let Some(pool) = connect().await else { return };
        let start = TEST_BASE;
        let end = TEST_BASE + 4;
        cleanup(&pool, start, end).await;

        let got = load_completed(&pool, start, end).await.expect("query");
        assert!(got.is_empty(), "expected no completed ledgers, got {got:?}");
    }

    #[tokio::test]
    async fn returns_only_sequences_within_range() {
        let Some(pool) = connect().await else { return };
        // Sparse fixture: one below the range, three inside, one above.
        let below = TEST_BASE + 100;
        let a = TEST_BASE + 110;
        let b = TEST_BASE + 111;
        let c = TEST_BASE + 115;
        let above = TEST_BASE + 120;
        cleanup(&pool, below, above).await;
        for seq in [below, a, b, c, above] {
            insert_ledger(&pool, seq).await;
        }

        let got = load_completed(&pool, a, c).await.expect("query");
        let expected: HashSet<u32> = [a, b, c].into_iter().collect();
        assert_eq!(got, expected);

        cleanup(&pool, below, above).await;
    }

    #[tokio::test]
    async fn inclusive_bounds() {
        let Some(pool) = connect().await else { return };
        let start = TEST_BASE + 200;
        let end = TEST_BASE + 205;
        cleanup(&pool, start, end).await;
        insert_ledger(&pool, start).await;
        insert_ledger(&pool, end).await;

        let got = load_completed(&pool, start, end).await.expect("query");
        assert!(got.contains(&start), "start must be inclusive");
        assert!(got.contains(&end), "end must be inclusive");
        assert_eq!(got.len(), 2);

        cleanup(&pool, start, end).await;
    }
}
