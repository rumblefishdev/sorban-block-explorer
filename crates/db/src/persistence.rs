//! Idempotent persistence layer for the indexing pipeline.
//!
//! All writes are replay-safe:
//! - Immutable tables use INSERT ON CONFLICT DO NOTHING (or DO UPDATE ... RETURNING for id recovery)
//! - Derived-state tables use watermark-guarded upserts (in soroban.rs)
//! - Inserts use UNNEST batching — one round trip per table per ledger
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
///
/// Uses `UNNEST` to insert all rows in a single round trip.
pub async fn insert_transactions_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    transactions: &[Transaction],
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    if transactions.is_empty() {
        return Ok(Vec::new());
    }

    let len = transactions.len();
    let mut hashes = Vec::with_capacity(len);
    let mut ledger_sequences = Vec::with_capacity(len);
    let mut source_accounts = Vec::with_capacity(len);
    let mut fees_charged = Vec::with_capacity(len);
    let mut successfuls = Vec::with_capacity(len);
    let mut result_codes: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut envelope_xdrs = Vec::with_capacity(len);
    let mut result_xdrs = Vec::with_capacity(len);
    let mut result_meta_xdrs: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut memo_types: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut memos: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut created_ats = Vec::with_capacity(len);
    let mut parse_errors: Vec<Option<bool>> = Vec::with_capacity(len);
    let mut operation_trees: Vec<Option<&serde_json::Value>> = Vec::with_capacity(len);

    for tx in transactions {
        hashes.push(tx.hash.as_str());
        ledger_sequences.push(tx.ledger_sequence);
        source_accounts.push(tx.source_account.as_str());
        fees_charged.push(tx.fee_charged);
        successfuls.push(tx.successful);
        result_codes.push(tx.result_code.as_deref());
        envelope_xdrs.push(tx.envelope_xdr.as_str());
        result_xdrs.push(tx.result_xdr.as_str());
        result_meta_xdrs.push(tx.result_meta_xdr.as_deref());
        memo_types.push(tx.memo_type.as_deref());
        memos.push(tx.memo.as_deref());
        created_ats.push(tx.created_at);
        parse_errors.push(tx.parse_error);
        operation_trees.push(tx.operation_tree.as_ref());
    }

    let mut conn = executor.acquire().await?;
    let rows = sqlx::query_as::<_, (String, i64)>(
        r#"INSERT INTO transactions
               (hash, ledger_sequence, source_account, fee_charged, successful,
                result_code, envelope_xdr, result_xdr, result_meta_xdr,
                memo_type, memo, created_at, parse_error, operation_tree)
           SELECT * FROM unnest(
               $1::text[], $2::bigint[], $3::text[], $4::bigint[], $5::bool[],
               $6::text[], $7::text[], $8::text[], $9::text[],
               $10::text[], $11::text[], $12::timestamptz[], $13::bool[], $14::jsonb[]
           )
           ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash
           RETURNING hash, id"#,
    )
    .bind(&hashes)
    .bind(&ledger_sequences)
    .bind(&source_accounts)
    .bind(&fees_charged)
    .bind(&successfuls)
    .bind(&result_codes)
    .bind(&envelope_xdrs)
    .bind(&result_xdrs)
    .bind(&result_meta_xdrs)
    .bind(&memo_types)
    .bind(&memos)
    .bind(&created_ats)
    .bind(&parse_errors)
    .bind(&operation_trees)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Operations (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert operations for a transaction. Skips duplicates.
/// Uses `UNNEST` to insert all rows in a single round trip.
pub async fn insert_operations_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    operations: &[Operation],
) -> Result<(), sqlx::Error> {
    if operations.is_empty() {
        return Ok(());
    }

    let len = operations.len();
    let mut transaction_ids = Vec::with_capacity(len);
    let mut application_orders = Vec::with_capacity(len);
    let mut source_accounts = Vec::with_capacity(len);
    let mut types = Vec::with_capacity(len);
    let mut details = Vec::with_capacity(len);

    for op in operations {
        transaction_ids.push(op.transaction_id);
        application_orders.push(op.application_order);
        source_accounts.push(op.source_account.as_str());
        types.push(op.op_type.as_str());
        details.push(&op.details);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO operations
               (transaction_id, application_order, source_account, type, details)
           SELECT * FROM unnest(
               $1::bigint[], $2::smallint[], $3::text[], $4::text[], $5::jsonb[]
           )
           ON CONFLICT ON CONSTRAINT uq_operations_tx_order DO NOTHING"#,
    )
    .bind(&transaction_ids)
    .bind(&application_orders)
    .bind(&source_accounts)
    .bind(&types)
    .bind(&details)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Events (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert events for a transaction. Skips duplicates.
/// Uses `UNNEST` to insert all rows in a single round trip.
pub async fn insert_events_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    events: &[SorobanEvent],
) -> Result<(), sqlx::Error> {
    if events.is_empty() {
        return Ok(());
    }

    let len = events.len();
    let mut transaction_ids = Vec::with_capacity(len);
    let mut contract_ids: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut event_types = Vec::with_capacity(len);
    let mut topics = Vec::with_capacity(len);
    let mut data = Vec::with_capacity(len);
    let mut event_indices = Vec::with_capacity(len);
    let mut ledger_sequences = Vec::with_capacity(len);
    let mut created_ats = Vec::with_capacity(len);

    for event in events {
        transaction_ids.push(event.transaction_id);
        contract_ids.push(event.contract_id.as_deref());
        event_types.push(event.event_type.as_str());
        topics.push(&event.topics);
        data.push(&event.data);
        event_indices.push(event.event_index);
        ledger_sequences.push(event.ledger_sequence);
        created_ats.push(event.created_at);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_events
               (transaction_id, contract_id, event_type, topics, data,
                event_index, ledger_sequence, created_at)
           SELECT * FROM unnest(
               $1::bigint[], $2::text[], $3::text[], $4::jsonb[], $5::jsonb[],
               $6::smallint[], $7::bigint[], $8::timestamptz[]
           )
           ON CONFLICT ON CONSTRAINT uq_events_tx_index DO NOTHING"#,
    )
    .bind(&transaction_ids)
    .bind(&contract_ids)
    .bind(&event_types)
    .bind(&topics)
    .bind(&data)
    .bind(&event_indices)
    .bind(&ledger_sequences)
    .bind(&created_ats)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Invocations (immutable, batch)
// ---------------------------------------------------------------------------

/// Batch insert invocations for a transaction. Skips duplicates.
/// Uses `UNNEST` to insert all rows in a single round trip.
pub async fn insert_invocations_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    invocations: &[SorobanInvocation],
) -> Result<(), sqlx::Error> {
    if invocations.is_empty() {
        return Ok(());
    }

    let len = invocations.len();
    let mut transaction_ids = Vec::with_capacity(len);
    let mut contract_ids: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut caller_accounts: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut function_names = Vec::with_capacity(len);
    let mut function_args: Vec<Option<&serde_json::Value>> = Vec::with_capacity(len);
    let mut return_values: Vec<Option<&serde_json::Value>> = Vec::with_capacity(len);
    let mut successfuls = Vec::with_capacity(len);
    let mut invocation_indices = Vec::with_capacity(len);
    let mut ledger_sequences = Vec::with_capacity(len);
    let mut created_ats = Vec::with_capacity(len);

    for inv in invocations {
        transaction_ids.push(inv.transaction_id);
        contract_ids.push(inv.contract_id.as_deref());
        caller_accounts.push(inv.caller_account.as_deref());
        function_names.push(inv.function_name.as_str());
        function_args.push(inv.function_args.as_ref());
        return_values.push(inv.return_value.as_ref());
        successfuls.push(inv.successful);
        invocation_indices.push(inv.invocation_index);
        ledger_sequences.push(inv.ledger_sequence);
        created_ats.push(inv.created_at);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_invocations
               (transaction_id, contract_id, caller_account, function_name,
                function_args, return_value, successful,
                invocation_index, ledger_sequence, created_at)
           SELECT * FROM unnest(
               $1::bigint[], $2::text[], $3::text[], $4::text[],
               $5::jsonb[], $6::jsonb[], $7::bool[],
               $8::smallint[], $9::bigint[], $10::timestamptz[]
           )
           ON CONFLICT ON CONSTRAINT uq_invocations_tx_index DO NOTHING"#,
    )
    .bind(&transaction_ids)
    .bind(&contract_ids)
    .bind(&caller_accounts)
    .bind(&function_names)
    .bind(&function_args)
    .bind(&return_values)
    .bind(&successfuls)
    .bind(&invocation_indices)
    .bind(&ledger_sequences)
    .bind(&created_ats)
    .execute(&mut *conn)
    .await?;

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

    /// UNNEST batch with nullable fields handles NULLs correctly.
    #[tokio::test]
    async fn transactions_batch_nullable_fields() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let seq = 88_888_883_i64;
        let hash = "c".repeat(64);

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

        // All nullable fields set to None
        let tx = Transaction {
            id: 0,
            hash: hash.clone(),
            ledger_sequence: seq,
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
            parse_error: None,
            operation_tree: None,
        };

        let result = insert_transactions_batch(&pool, &[tx]).await.unwrap();
        assert_eq!(result.len(), 1, "transaction with all NULLs inserted");

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
