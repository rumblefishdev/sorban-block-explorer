//! `account_balances_current` — watermark UPSERT. ADR 0035: only the
//! current value is kept; history dropped. Two paths matching the partial
//! UNIQUEs from migration 0007:
//!
//! 1. **Native** — `uidx_abc_native` on `(account_id) WHERE asset_type = 0`.
//!    `account_id` remap; `asset_code` + `issuer_id` are NULL.
//! 2. **Credit** — `uidx_abc_credit` on `(account_id, asset_code, issuer_id)
//!    WHERE asset_type <> 0`. Both `account_id` and `issuer_id` need remap.
//!
//! Mirror of `write.rs:1866-1948`. The watermark CASE is identical to the
//! indexer's: `EXCLUDED.last_updated_ledger >= existing.last_updated_ledger`
//! decides whether to overwrite. Without this, naive load order corrupts
//! balances on cross-snapshot merge.

use sqlx::PgConnection;

use crate::batcher::{MergeStats, single};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    let mut total = MergeStats::default();

    let native = single(
        conn,
        "account_balances_current:native",
        r#"
        INSERT INTO account_balances_current
            (account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger)
        SELECT ra.target_id, 0, NULL, NULL, abc.balance, abc.last_updated_ledger
          FROM merge_source.account_balances_current abc
          JOIN merge_remap.accounts ra ON ra.source_id = abc.account_id
         WHERE abc.asset_type = 0
        ON CONFLICT (account_id) WHERE asset_type = 0 DO UPDATE SET
            balance = CASE
                WHEN EXCLUDED.last_updated_ledger >= account_balances_current.last_updated_ledger
                THEN EXCLUDED.balance
                ELSE account_balances_current.balance
            END,
            last_updated_ledger = GREATEST(
                account_balances_current.last_updated_ledger,
                EXCLUDED.last_updated_ledger
            )
        "#,
    )
    .await?;
    total.batches += native.batches;
    total.rows_affected += native.rows_affected;

    let credit = single(
        conn,
        "account_balances_current:credit",
        r#"
        INSERT INTO account_balances_current
            (account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger)
        SELECT ra.target_id, abc.asset_type, abc.asset_code, ri.target_id,
               abc.balance, abc.last_updated_ledger
          FROM merge_source.account_balances_current abc
          JOIN merge_remap.accounts ra ON ra.source_id = abc.account_id
          JOIN merge_remap.accounts ri ON ri.source_id = abc.issuer_id
         WHERE abc.asset_type <> 0
        ON CONFLICT (account_id, asset_code, issuer_id) WHERE asset_type <> 0 DO UPDATE SET
            balance = CASE
                WHEN EXCLUDED.last_updated_ledger >= account_balances_current.last_updated_ledger
                THEN EXCLUDED.balance
                ELSE account_balances_current.balance
            END,
            last_updated_ledger = GREATEST(
                account_balances_current.last_updated_ledger,
                EXCLUDED.last_updated_ledger
            ),
            asset_type = account_balances_current.asset_type
        "#,
    )
    .await?;
    total.batches += credit.batches;
    total.rows_affected += credit.rows_affected;

    Ok(total)
}
