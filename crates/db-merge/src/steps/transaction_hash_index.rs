//! `transaction_hash_index` — natural PK on `hash`. UNION; no FK
//! referrers. Mirror of `write.rs:649`.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "transaction_hash_index",
        "merge_source.transaction_hash_index",
        "ledger_sequence",
        r#"
        INSERT INTO transaction_hash_index (hash, ledger_sequence, created_at)
        SELECT hash, ledger_sequence, created_at
          FROM merge_source.transaction_hash_index
         WHERE ledger_sequence BETWEEN {lo} AND {hi}
        ON CONFLICT (hash) DO NOTHING
        "#,
    )
    .await
}
