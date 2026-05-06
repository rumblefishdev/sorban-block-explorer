//! `accounts` — REMAP pass. Surrogate `id BIGSERIAL` is referenced as
//! FK across many tables so we must capture every (source_id → target_id)
//! mapping into `merge_remap.accounts`.
//!
//! Mirror of `crates/indexer/.../write.rs::upsert_accounts` (~line 86).
//! Key differences:
//! - reads from `merge_source.accounts` (FDW) instead of UNNEST arrays;
//! - the final CTE writes the remap row for both INSERT and UPDATE branches.
//!
//! Batched by `first_seen_ledger` — monotonic-per-account on a single
//! source DB (a row's first_seen never moves once set; we LEAST it down
//! on conflict, which is fine because that path operates on the target
//! row, not the source row).

use sqlx::PgConnection;

use crate::batcher::{MergeStats, ledger_windowed};
use crate::error::MergeError;

pub async fn run(conn: &mut PgConnection) -> Result<MergeStats, MergeError> {
    ledger_windowed(
        conn,
        "accounts",
        "merge_source.accounts",
        "first_seen_ledger",
        r#"
        WITH input AS (
            SELECT id AS src_id, account_id, first_seen_ledger, last_seen_ledger,
                   sequence_number, home_domain
              FROM merge_source.accounts
             WHERE first_seen_ledger BETWEEN {lo} AND {hi}
        ),
        inserted AS (
            INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain)
            SELECT account_id, first_seen_ledger, last_seen_ledger,
                   COALESCE(NULLIF(sequence_number, -1), 0), home_domain
              FROM input
            ON CONFLICT (account_id) DO NOTHING
            RETURNING id, account_id
        ),
        updated AS (
            UPDATE accounts a SET
                last_seen_ledger = GREATEST(a.last_seen_ledger, i.last_seen_ledger),
                -- Source's sequence_number is post-COALESCE (the indexer
                -- ran NULLIF($, -1)→0 on insert). The indexer's own UPDATE
                -- clause checks `sq <> -1` because it sees raw input;
                -- here we see the converted form, so check `> 0` instead.
                -- 0 means "no real sequence ever observed" — never overwrite
                -- a real value with that sentinel.
                sequence_number  = CASE
                    WHEN i.last_seen_ledger >= a.last_seen_ledger
                     AND i.sequence_number > 0
                    THEN i.sequence_number
                    ELSE a.sequence_number
                END,
                home_domain = CASE
                    WHEN i.last_seen_ledger >= a.last_seen_ledger
                     AND i.home_domain IS NOT NULL
                    THEN i.home_domain
                    ELSE a.home_domain
                END,
                first_seen_ledger = LEAST(a.first_seen_ledger, i.first_seen_ledger)
            FROM input i
            WHERE a.account_id = i.account_id
              AND NOT EXISTS (SELECT 1 FROM inserted ins WHERE ins.account_id = a.account_id)
            RETURNING a.id, a.account_id
        )
        INSERT INTO merge_remap.accounts (source_id, target_id)
        SELECT i.src_id, COALESCE(ins.id, upd.id)
          FROM input i
          LEFT JOIN inserted ins ON ins.account_id = i.account_id
          LEFT JOIN updated  upd ON upd.account_id = i.account_id
        ON CONFLICT (source_id) DO UPDATE SET target_id = EXCLUDED.target_id
        "#,
    )
    .await
}
