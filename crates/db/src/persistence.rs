//! Idempotent persistence layer for the indexing pipeline.
//!
//! All writes are replay-safe:
//! - Immutable tables use INSERT ON CONFLICT DO NOTHING (or DO UPDATE ... RETURNING for id recovery)
//! - Derived-state tables use watermark-guarded upserts (in soroban.rs)
//! - Inserts are grouped per ledger; each row is one round trip (UNNEST batching is a future optimisation)
//! - No delete-then-reinsert patterns (preserves ON DELETE CASCADE children)

use domain::{
    ledger::Ledger,
    operation::Operation,
    soroban::{SorobanEvent, SorobanInvocation},
    transaction::Transaction,
};
use sqlx::Acquire;

// ---------------------------------------------------------------------------
// Ledger (immutable)
// ---------------------------------------------------------------------------

/// Insert a ledger row. Returns `true` if inserted, `false` if already existed.
pub async fn insert_ledger(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    ledger: &Ledger,
) -> Result<bool, sqlx::Error> {
    let mut conn = executor.acquire().await?;
    let result = sqlx::query(
        r#"INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (sequence) DO NOTHING"#,
    )
    .bind(ledger.sequence)
    .bind(&ledger.hash)
    .bind(ledger.closed_at)
    .bind(ledger.protocol_version)
    .bind(ledger.transaction_count)
    .bind(ledger.base_fee)
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Transactions (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert transactions for a ledger. Idempotent — replays are safe.
///
/// Returns the list of (hash, id) pairs for ALL input transactions,
/// whether freshly inserted or already existing. This ensures child rows
/// (operations, events, invocations) can always be linked by transaction_id
/// even when replaying a previously partially-processed ledger.
pub async fn insert_transactions_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    transactions: &[Transaction],
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    if transactions.is_empty() {
        return Ok(Vec::new());
    }

    let mut conn = executor.acquire().await?;
    let mut result = Vec::with_capacity(transactions.len());

    for tx in transactions {
        let (id,) = sqlx::query_as::<_, (i64,)>(
            r#"INSERT INTO transactions
                   (hash, ledger_sequence, source_account, fee_charged, successful,
                    result_code, envelope_xdr, result_xdr, result_meta_xdr,
                    memo_type, memo, created_at, parse_error, operation_tree)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
               ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash
               RETURNING id"#,
        )
        .bind(&tx.hash)
        .bind(tx.ledger_sequence)
        .bind(&tx.source_account)
        .bind(tx.fee_charged)
        .bind(tx.successful)
        .bind(&tx.result_code)
        .bind(&tx.envelope_xdr)
        .bind(&tx.result_xdr)
        .bind(&tx.result_meta_xdr)
        .bind(&tx.memo_type)
        .bind(&tx.memo)
        .bind(tx.created_at)
        .bind(tx.parse_error)
        .bind(&tx.operation_tree)
        .fetch_one(&mut *conn)
        .await?;

        result.push((tx.hash.clone(), id));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Operations (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert operations for a transaction. Skips duplicates.
pub async fn insert_operations_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    operations: &[Operation],
) -> Result<(), sqlx::Error> {
    if operations.is_empty() {
        return Ok(());
    }

    let mut conn = executor.acquire().await?;

    for op in operations {
        sqlx::query(
            r#"INSERT INTO operations
                   (transaction_id, application_order, source_account, type, details)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT ON CONSTRAINT uq_operations_tx_order DO NOTHING"#,
        )
        .bind(op.transaction_id)
        .bind(op.application_order)
        .bind(&op.source_account)
        .bind(&op.op_type)
        .bind(&op.details)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Events (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert events for a transaction. Skips duplicates.
pub async fn insert_events_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    events: &[SorobanEvent],
) -> Result<(), sqlx::Error> {
    if events.is_empty() {
        return Ok(());
    }

    let mut conn = executor.acquire().await?;

    for event in events {
        sqlx::query(
            r#"INSERT INTO soroban_events
                   (transaction_id, contract_id, event_type, topics, data,
                    event_index, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT ON CONSTRAINT uq_events_tx_index DO NOTHING"#,
        )
        .bind(event.transaction_id)
        .bind(&event.contract_id)
        .bind(&event.event_type)
        .bind(&event.topics)
        .bind(&event.data)
        .bind(event.event_index)
        .bind(event.ledger_sequence)
        .bind(event.created_at)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Invocations (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert invocations for a transaction. Skips duplicates.
pub async fn insert_invocations_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    invocations: &[SorobanInvocation],
) -> Result<(), sqlx::Error> {
    if invocations.is_empty() {
        return Ok(());
    }

    let mut conn = executor.acquire().await?;

    for inv in invocations {
        sqlx::query(
            r#"INSERT INTO soroban_invocations
                   (transaction_id, contract_id, caller_account, function_name,
                    function_args, return_value, successful,
                    invocation_index, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT ON CONSTRAINT uq_invocations_tx_index DO NOTHING"#,
        )
        .bind(inv.transaction_id)
        .bind(&inv.contract_id)
        .bind(&inv.caller_account)
        .bind(&inv.function_name)
        .bind(&inv.function_args)
        .bind(&inv.return_value)
        .bind(inv.successful)
        .bind(inv.invocation_index)
        .bind(inv.ledger_sequence)
        .bind(inv.created_at)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    async fn test_pool() -> Option<sqlx::PgPool> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "SKIP: DATABASE_URL not set — integration tests require a running PostgreSQL instance"
            );
            return None;
        };
        Some(
            sqlx::PgPool::connect(&url)
                .await
                .expect("failed to connect to DATABASE_URL"),
        )
    }

    fn test_ledger(seq: i64) -> Ledger {
        Ledger {
            sequence: seq,
            hash: format!("{:064x}", seq),
            closed_at: Utc::now(),
            protocol_version: 21,
            transaction_count: 0,
            base_fee: 100,
        }
    }

    fn test_transaction(hash: &str, ledger_seq: i64) -> Transaction {
        Transaction {
            id: 0,
            hash: hash.to_string(),
            ledger_sequence: ledger_seq,
            source_account: "GABC123".to_string(),
            fee_charged: 100,
            successful: true,
            result_code: None,
            envelope_xdr: "envelope".to_string(),
            result_xdr: "result".to_string(),
            result_meta_xdr: None,
            memo_type: None,
            memo: None,
            created_at: Utc::now(),
            parse_error: Some(false),
            operation_tree: None,
        }
    }

    /// AC 13: Replaying the same ledger produces no duplicate rows and no errors.
    #[tokio::test]
    async fn ledger_insert_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let seq = 88_888_881_i64;
        sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
            .bind(seq)
            .execute(&pool)
            .await
            .unwrap();

        let ledger = test_ledger(seq);
        let first = insert_ledger(&pool, &ledger).await.unwrap();
        let second = insert_ledger(&pool, &ledger).await.unwrap();

        assert!(first, "first insert should return true");
        assert!(!second, "duplicate insert should return false");
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ledgers WHERE sequence = $1")
            .bind(seq)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);

        sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
            .bind(seq)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC 13: Replaying the same transaction produces no duplicate rows and no errors.
    #[tokio::test]
    async fn transactions_batch_insert_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let seq = 88_888_882_i64;
        let hash = "b".repeat(64);

        sqlx::query("DELETE FROM transactions WHERE hash = $1")
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
            .bind(seq)
            .execute(&pool)
            .await
            .unwrap();

        insert_ledger(&pool, &test_ledger(seq)).await.unwrap();

        let txs = vec![test_transaction(&hash, seq)];
        let r1 = insert_transactions_batch(&pool, &txs).await.unwrap();
        let r2 = insert_transactions_batch(&pool, &txs).await.unwrap();

        assert_eq!(r1.len(), 1, "first batch returns id");
        assert_eq!(
            r2.len(),
            1,
            "replay also returns id (needed for child inserts)"
        );
        assert_eq!(r1[0].1, r2[0].1, "same id on both calls");
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions WHERE hash = $1")
            .bind(&hash)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "still exactly one row after replay");

        sqlx::query("DELETE FROM transactions WHERE hash = $1")
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
            .bind(seq)
            .execute(&pool)
            .await
            .unwrap();
    }
}
