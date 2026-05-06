//! `wasm_interface_metadata` — append, dedup by `wasm_hash`,
//! newer non-empty metadata wins.
//!
//! Mirror of indexer upsert at `crates/indexer/src/handler/persist/write.rs`
//! around line 158 (`upsert_wasm_metadata`). No FK referrers; no remap.
//!
//! Small table (~hundreds of rows even on full mainnet) — single batch
//! is fine, no ledger windowing.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, single};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    single(
        conn,
        "wasm_interface_metadata",
        r#"
        INSERT INTO wasm_interface_metadata (wasm_hash, metadata)
        SELECT wasm_hash, metadata FROM merge_source.wasm_interface_metadata
        ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata
        "#,
    )
    .await
}
