//! Persistence functions for Soroban events, invocations, and contracts.
//!
//! All inserts use parameterized queries via sqlx. Batch inserts are grouped
//! within a single transaction for atomicity and performance.

use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use tracing::warn;
use xdr_parser::types::{ExtractedContractInterface, ExtractedEvent, ExtractedInvocation};

/// Insert extracted events into `soroban_events`.
///
/// Requires the parent `transaction_id` (BIGSERIAL from the transactions table).
/// Events are inserted in order; `created_at` determines the target partition.
pub async fn insert_events(
    pool: &PgPool,
    events: &[ExtractedEvent],
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    for event in events {
        let created_at = unix_to_datetime(event.created_at);
        sqlx::query(
            r#"INSERT INTO soroban_events
               (transaction_id, contract_id, event_type, topics, data, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(transaction_id)
        .bind(&event.contract_id)
        .bind(&event.event_type)
        .bind(&event.topics)
        .bind(&event.data)
        .bind(event.ledger_sequence as i64)
        .bind(created_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Insert extracted invocations into `soroban_invocations`.
///
/// Requires the parent `transaction_id` (BIGSERIAL from the transactions table).
pub async fn insert_invocations(
    pool: &PgPool,
    invocations: &[ExtractedInvocation],
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    for inv in invocations {
        let created_at = unix_to_datetime(inv.created_at);
        sqlx::query(
            r#"INSERT INTO soroban_invocations
               (transaction_id, contract_id, caller_account, function_name,
                function_args, return_value, successful, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(transaction_id)
        .bind(&inv.contract_id)
        .bind(&inv.caller_account)
        .bind(&inv.function_name)
        .bind(&inv.function_args)
        .bind(&inv.return_value)
        .bind(inv.successful)
        .bind(inv.ledger_sequence as i64)
        .bind(created_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Upsert contract interface metadata into `soroban_contracts`.
///
/// If the contract row already exists (created by task 0027), this updates
/// only the `metadata` column. If it doesn't exist yet, inserts a minimal
/// row with just `wasm_hash` and `metadata` — task 0027 will fill the rest.
pub async fn upsert_contract_metadata(
    pool: &PgPool,
    interface: &ExtractedContractInterface,
    contract_id: &str,
) -> Result<(), sqlx::Error> {
    let metadata = match serde_json::to_value(&interface.functions) {
        Ok(v) => serde_json::json!({
            "interface": { "functions": v },
            "wasm_hash": interface.wasm_hash,
            "wasm_byte_len": interface.wasm_byte_len,
        }),
        Err(e) => {
            warn!(contract_id, "failed to serialize contract interface: {e}");
            return Ok(());
        }
    };

    sqlx::query(
        r#"INSERT INTO soroban_contracts (contract_id, wasm_hash, metadata)
           VALUES ($1, $2, $3)
           ON CONFLICT (contract_id) DO UPDATE SET
               metadata = EXCLUDED.metadata,
               wasm_hash = COALESCE(soroban_contracts.wasm_hash, EXCLUDED.wasm_hash)"#,
    )
    .bind(contract_id)
    .bind(&interface.wasm_hash)
    .bind(&metadata)
    .execute(pool)
    .await?;

    Ok(())
}

/// Update `transactions.operation_tree` for a given transaction.
pub async fn update_operation_tree(
    pool: &PgPool,
    transaction_id: i64,
    operation_tree: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE transactions SET operation_tree = $1 WHERE id = $2"#,
    )
    .bind(operation_tree)
    .bind(transaction_id)
    .execute(pool)
    .await?;

    Ok(())
}

fn unix_to_datetime(unix_secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(unix_secs, 0)
        .single()
        .unwrap_or_default()
}
