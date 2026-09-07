//! Tier-1 column rebuild for the post-merge Hetzner CH (task 0228 Phase 5).
//!
//! ## Why this exists
//!
//! Under cross-machine parallel backfill (ADR 0040 + ADR 0045), each
//! worker ingests a disjoint ledger range into its own local CH. Worker
//! N's parser stamps `first_seen_ledger = first ledger of N's range`
//! for accounts it sees there, with no visibility into earlier worker
//! ranges. After FREEZE + rsync + ATTACH PART merges the three local
//! CHs into Hetzner, the `ReplacingMergeTree(last_seen_ledger)` collapse
//! keeps the row with the **highest** version — so the value of
//! `first_seen_ledger` ends up reflecting whatever the latest-touching
//! worker observed, not the actual earliest observation across the
//! union.
//!
//! **Not only under parallel backfill.** The same corruption happens on
//! ordinary live ingest: the indexer sees only its current batch, so any
//! later event for an entity carries no historic minimum and the RMT
//! replace erases whatever was stored. Measured 2026-09-01 —
//! `nfts.minted_at_ledger` was losing ~30 tokens/day, and 3.5% of a
//! 400-row `accounts` sample carried a `first_seen_ledger` later than the
//! true first appearance. So a clean run of this pass is point-in-time
//! cleanup, never a guarantee. Task 0497 retires the compromise itself;
//! task 0528 already moved `nfts.minted_at_ledger` off this pass by
//! deriving it at read time.
//!
//! Twelve Tier-1 columns silently corrupt this way; this pass repairs
//! the **6** that derive from on-chain facts (across **5** tables). The
//! remaining 6 (NFT metadata: `collection_name`, `name`, `media_url` ×
//! `nfts` + `nfts_pending`) require external HTTP/IPFS fetches and are
//! delivered by task 0231 (Stage 2 SEP-1 + NFT `token_uri` enrichment).
//!
//! ### Stage 1 scope (this module) — 6 columns × 5 tables
//!
//! | Table | Column | Correct rebuild |
//! |-------|--------|-----------------|
//! | `accounts` | `first_seen_ledger` | `MIN(ledger_sequence) FROM transaction_participants` |
//! | `lp_positions` | `first_deposit_ledger` | `MIN(ledger_sequence) FROM operations_appearances WHERE type = 22 (LiquidityPoolDeposit)` |
//! | `nfts` | `minted_at_ledger` | `MIN(ledger_sequence) FROM nft_ownership WHERE event_type = 0 (Mint)` |
//! | `nfts_pending` | `minted_at_ledger` | `MIN(ledger_sequence) FROM nft_ownership_pending WHERE event_type = 0` |
//! | `soroban_contracts` | `deployer_id` + `deployed_at_ledger` | `argMin(deployer_id, wasm_uploaded_at_ledger)` + `MIN(wasm_uploaded_at_ledger)` over rows where `deployer_id IS NOT NULL` |
//!
//! ### Stage 2 scope (task 0231, deferred) — 6 columns × 2 tables
//!
//! `nfts.{collection_name, name, media_url}` and
//! `nfts_pending.{collection_name, name, media_url}` — populated by
//! per-row SEP-1 / NFT `token_uri` enrichment loop. Not repaired here.
//!
//! **Source selection rule**: state-shaped tables under
//! `ReplacingMergeTree` collapse history on `OPTIMIZE FINAL`, so the
//! historic MIN must come from append-only fact tables
//! (`transaction_participants`, `operations_appearances`,
//! `nft_ownership`). The one exception is `soroban_contracts`: deployer
//! info is only stored on `soroban_contracts` itself (no dedicated fact
//! table exists for deployments), so the rebuild reads the raw
//! pre-FINAL table and filters non-NULL rows — fragile if a full
//! `OPTIMIZE soroban_contracts FINAL` runs before this pass and the
//! kept row has NULL deployer fields. Run Tier-1 **before** any
//! `OPTIMIZE FINAL` on `soroban_contracts` (the Phase 5 plan
//! Step 2 lists only `wasm_interface_metadata` + `ledgers` for the
//! pre-repair OPTIMIZE, so this is fine in the documented sequence).
//!
//! The remaining `nfts.{collection_name, name, media_url}` (and
//! `nfts_pending.{collection_name, name, media_url}`) columns are filled
//! by Stage 2 of the CH-enrichment plan (SEP-1 + NFT `token_uri` port,
//! tracked separately). They remain NULL after Tier-1 — that is
//! correct.
//!
//! ## Pattern
//!
//! Per-table: build `<table>_staging_repair_tier1` with the same engine
//! and schema, INSERT rebuilt rows, then `EXCHANGE TABLES` to atomically
//! swap the live table with the staging one and `DROP TABLE` the now-old
//! data. `--dry-run` short-circuits before the swap, logs the staging
//! row count, then drops the staging table — used on laptop 1's local
//! CH as a sandbox before running for real on Hetzner post-ATTACH.
//!
//! ## Idempotency
//!
//! Running twice in a row is safe. The second run re-derives the same
//! aggregates from the same source rows and produces the same staging
//! contents; the EXCHANGE then no-ops the column values. If a previous
//! run crashed mid-INSERT, the staging table is left behind — the
//! `CREATE TABLE IF NOT EXISTS` arm in [`drop_if_exists`] cleans it up
//! before the next attempt.

use clickhouse::Client as ClickhouseClient;
use tracing::info;

use crate::ch_staging::{create_staging_like, drop_if_exists, finalize, staging_row_count};
use crate::error::BackfillError;
use crate::sink::Sink;

/// Counters logged after the repair pass. Each `*_rows` is the number
/// of rows in the staging table after the rebuild INSERT — useful for
/// the operator to confirm the swap touched the expected order of
/// magnitude vs the live table's `count()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct RepairTier1Stats {
    pub accounts_rows: u64,
    pub lp_positions_rows: u64,
    pub nfts_rows: u64,
    pub nfts_pending_rows: u64,
    pub soroban_contracts_rows: u64,
    pub dry_run: bool,
}

/// Run the full Tier-1 column rebuild pass.
///
/// CH-only. Each table
/// rebuilds sequentially so a failure mid-way leaves the live tables
/// untouched (failures happen on staging-table writes; EXCHANGE only
/// fires after the INSERT completes).
pub async fn execute(sink: &Sink, dry_run: bool) -> Result<RepairTier1Stats, BackfillError> {
    let client = sink.client();

    let mut stats = RepairTier1Stats {
        dry_run,
        ..Default::default()
    };

    stats.accounts_rows = rebuild_accounts(client, dry_run).await?;
    stats.lp_positions_rows = rebuild_lp_positions(client, dry_run).await?;
    stats.nfts_rows = rebuild_nfts(client, "nfts", "nft_ownership", dry_run).await?;
    stats.nfts_pending_rows =
        rebuild_nfts(client, "nfts_pending", "nft_ownership_pending", dry_run).await?;
    stats.soroban_contracts_rows = rebuild_soroban_contracts(client, dry_run).await?;

    info!(
        accounts = stats.accounts_rows,
        lp_positions = stats.lp_positions_rows,
        nfts = stats.nfts_rows,
        nfts_pending = stats.nfts_pending_rows,
        soroban_contracts = stats.soroban_contracts_rows,
        dry_run,
        "repair_tier1: completed"
    );
    Ok(stats)
}

/// `accounts.first_seen_ledger` ← `MIN(tp.ledger_sequence)` joined on
/// the surrogate `accounts.id`. The transaction_participants table is
/// the participant universe — every account that has ever touched a
/// transaction shows up there, so its `min(ledger_sequence)` is the
/// authoritative earliest observation across the merged union.
///
/// COALESCE preserves the existing `first_seen_ledger` for any account
/// row that lacks a participants entry (defensive — bootstrap-only
/// accounts that came in via Soroban RPC `getLedgerEntries` but never
/// hit a tx in the indexed range; rare, but not zero).
async fn rebuild_accounts(client: &ClickhouseClient, dry_run: bool) -> Result<u64, BackfillError> {
    let staging = "accounts_staging_repair_tier1";
    drop_if_exists(client, staging).await?;
    create_staging_like(client, "accounts", staging).await?;

    let insert_sql = format!(
        "INSERT INTO {staging} (id, account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain)
         SELECT
             a.id,
             a.account_id,
             ifNull(m.min_ledger, a.first_seen_ledger) AS first_seen_ledger,
             a.last_seen_ledger,
             a.sequence_number,
             a.home_domain
           FROM accounts AS a FINAL
           LEFT JOIN (
             SELECT account_id AS id, min(ledger_sequence) AS min_ledger
               FROM transaction_participants
              GROUP BY id
           ) AS m ON m.id = a.id"
    );
    client
        .query(&insert_sql)
        .execute()
        .await
        .map_err(BackfillError::Ch)?;

    let rows = staging_row_count(client, staging).await?;
    info!(staging, rows, "repair_tier1: accounts staging built");

    finalize(client, "accounts", staging, dry_run).await?;
    Ok(rows)
}

/// `lp_positions.first_deposit_ledger` ← `MIN(ledger_sequence)` over
/// `operations_appearances` filtered to `LiquidityPoolDeposit` (op
/// type 22), joined by `(pool_id, source_id)`. The fact table
/// `operations_appearances` does not collapse under RMT in a way that
/// loses history (ORDER BY includes `ledger_sequence`,
/// `transaction_id`, `application_order` — distinct deposits stay
/// distinct), so the MIN there is the authoritative earliest deposit.
///
/// We deliberately do NOT read MIN from `lp_positions` itself: under
/// `RMT(last_updated_ledger)` collapse, only the latest version of the
/// `(pool, account)` row survives — MIN would then equal MAX and the
/// rebuild would no-op (or worse, overwrite a correct value with the
/// withdrawal/last-touched ledger).
///
/// `isNotNull(source_id)` + `isNotNull(pool_id)` is defensive — type
/// 22 ops should always carry both, but the schema makes both columns
/// `Nullable(…)` so an out-of-spec row would otherwise NULL the JOIN
/// key and skip a position.
async fn rebuild_lp_positions(
    client: &ClickhouseClient,
    dry_run: bool,
) -> Result<u64, BackfillError> {
    let staging = "lp_positions_staging_repair_tier1";
    drop_if_exists(client, staging).await?;
    create_staging_like(client, "lp_positions", staging).await?;

    // OperationType::LiquidityPoolDeposit = 22 (see
    // crates/domain/src/enums/operation_type.rs).
    let insert_sql = format!(
        "INSERT INTO {staging} (pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger)
         SELECT
             lp.pool_id,
             lp.account_id,
             lp.shares,
             ifNull(m.min_ledger, lp.first_deposit_ledger) AS first_deposit_ledger,
             lp.last_updated_ledger
           FROM lp_positions AS lp FINAL
           LEFT JOIN (
             SELECT
                 arrayJoin(pool_ids) AS pool_id,
                 source_id AS account_id,
                 min(ledger_sequence) AS min_ledger
               FROM operations_appearances
              WHERE type = 22 AND isNotNull(source_id) AND notEmpty(pool_ids)
              GROUP BY pool_id, source_id
           ) AS m ON m.pool_id = lp.pool_id AND m.account_id = lp.account_id"
    );
    client
        .query(&insert_sql)
        .execute()
        .await
        .map_err(BackfillError::Ch)?;

    let rows = staging_row_count(client, staging).await?;
    info!(staging, rows, "repair_tier1: lp_positions staging built");

    finalize(client, "lp_positions", staging, dry_run).await?;
    Ok(rows)
}

/// `nfts.minted_at_ledger` (and `nfts_pending.minted_at_ledger`) ←
/// `MIN(ledger_sequence)` over rows in the matching `nft_ownership*`
/// fact table filtered to `event_type = 0 (Mint)`. The plain "earliest
/// ownership ledger" would be wrong: a Transfer/Burn at an earlier
/// ledger (rare but legal under XDR replay races) would set
/// `minted_at_ledger` to a non-mint ledger.
///
/// `event_type = 0` matches `domain::enums::NftEventType::Mint`.
///
/// `collection_name`, `name`, `media_url` stay at their existing values
/// — populating those is Stage 2 work (SEP-1 + NFT `token_uri` port).
async fn rebuild_nfts(
    client: &ClickhouseClient,
    nfts_table: &str,
    ownership_table: &str,
    dry_run: bool,
) -> Result<u64, BackfillError> {
    let staging = format!("{nfts_table}_staging_repair_tier1");
    drop_if_exists(client, &staging).await?;
    create_staging_like(client, nfts_table, &staging).await?;

    let insert_sql = format!(
        "INSERT INTO {staging} (contract_id, token_id, collection_name, name, media_url, minted_at_ledger, current_owner_id, current_owner_ledger)
         SELECT
             n.contract_id,
             n.token_id,
             n.collection_name,
             n.name,
             n.media_url,
             ifNull(m.min_ledger, n.minted_at_ledger) AS minted_at_ledger,
             n.current_owner_id,
             n.current_owner_ledger
           FROM {nfts_table} AS n FINAL
           LEFT JOIN (
             SELECT contract_id, token_id, min(ledger_sequence) AS min_ledger
               FROM {ownership_table}
              WHERE event_type = 0
              GROUP BY contract_id, token_id
           ) AS m ON m.contract_id = n.contract_id AND m.token_id = n.token_id"
    );
    client
        .query(&insert_sql)
        .execute()
        .await
        .map_err(BackfillError::Ch)?;

    let rows = staging_row_count(client, &staging).await?;
    info!(
        staging = staging.as_str(),
        rows, "repair_tier1: nfts staging built"
    );

    finalize(client, nfts_table, &staging, dry_run).await?;
    Ok(rows)
}

/// `soroban_contracts.{deployer_id, deployed_at_ledger}` ← rebuild from
/// the row with the smallest `wasm_uploaded_at_ledger` across the
/// un-FINAL union where `deployer_id IS NOT NULL`. Per
/// `crates/db-clickhouse/src/persist/stage.rs`, every real deployment
/// row carries `wasm_uploaded_at_ledger = deployed_at_ledger` and a
/// non-NULL deployer; stub/SAC-override rows carry NULL deployer.
/// So `MIN(wasm_uploaded_at_ledger) WHERE deployer_id IS NOT NULL`
/// is the original deployment ledger, and `argMin(deployer_id,
/// wasm_uploaded_at_ledger)` is the original deployer.
///
/// Reads the raw (non-FINAL) table — see module-level docstring for
/// the caveat about running this before any aggressive
/// `OPTIMIZE soroban_contracts FINAL`.
async fn rebuild_soroban_contracts(
    client: &ClickhouseClient,
    dry_run: bool,
) -> Result<u64, BackfillError> {
    let staging = "soroban_contracts_staging_repair_tier1";
    drop_if_exists(client, staging).await?;
    create_staging_like(client, "soroban_contracts", staging).await?;

    // Subquery aliases use a distinct suffix (`_rebuilt`) to avoid
    // CH 26.3 rejecting `WHERE isNotNull(deployer_id)` as
    // ILLEGAL_AGGREGATION when the projected alias shadows the raw
    // column name (the parser resolves the WHERE reference against
    // the projection list and sees an aggregate there).
    let insert_sql = format!(
        "INSERT INTO {staging} (id, contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, deployed_at_ledger, contract_type, is_sac)
         SELECT
             sc.id,
             sc.contract_id,
             sc.wasm_hash,
             sc.wasm_uploaded_at_ledger,
             ifNull(d.deployer_id_rebuilt, sc.deployer_id) AS deployer_id,
             ifNull(d.deployed_at_ledger_rebuilt, sc.deployed_at_ledger) AS deployed_at_ledger,
             sc.contract_type,
             sc.is_sac
           FROM soroban_contracts AS sc FINAL
           LEFT JOIN (
             SELECT
                 contract_id,
                 argMin(deployer_id, wasm_uploaded_at_ledger) AS deployer_id_rebuilt,
                 min(wasm_uploaded_at_ledger) AS deployed_at_ledger_rebuilt
               FROM soroban_contracts
              WHERE isNotNull(deployer_id)
              GROUP BY contract_id
           ) AS d ON d.contract_id = sc.contract_id"
    );
    client
        .query(&insert_sql)
        .execute()
        .await
        .map_err(BackfillError::Ch)?;

    let rows = staging_row_count(client, staging).await?;
    info!(
        staging,
        rows, "repair_tier1: soroban_contracts staging built"
    );

    finalize(client, "soroban_contracts", staging, dry_run).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    //! Tier-1 rebuild tests. Same gating posture as `bootstrap.rs` and
    //! `sink.rs` — skipped without `CLICKHOUSE_URL`. Each test cleans up
    //! its fixture range before and after to stay safe on a shared CH.
    use super::*;
    use crate::sink::Sink;
    use db_clickhouse::persist::rows::{AccountRow, TransactionParticipantRow};

    const TEST_BASE: u32 = 4_000_020_000;

    async fn build_ch_sink() -> Option<Sink> {
        let url = std::env::var("CLICKHOUSE_URL").ok()?;
        let cfg = db_clickhouse::Config {
            url,
            ..db_clickhouse::Config::from_env()
        };
        let client = db_clickhouse::client(&cfg);
        if let Err(err) = db_clickhouse::apply_init_sql(&client).await {
            eprintln!("CLICKHOUSE_URL set but apply_init_sql failed ({err}) — skipping");
            return None;
        }
        Some(Sink::new(client))
    }

    /// Dry-run smoke for the accounts rebuild: stamp an account with a
    /// deliberately-wrong `first_seen_ledger` (RMT collapse outcome),
    /// stamp two `transaction_participants` rows at earlier ledgers,
    /// invoke `rebuild_accounts` in dry-run mode. Asserts the live
    /// `accounts` table is UNCHANGED — the staging-table swap only
    /// happens on a real run.
    ///
    /// This test deliberately does NOT verify staging contents: the
    /// dry-run path drops the staging table inside `finalize()` before
    /// the test can query it. Real-run correctness (staging actually
    /// contains the corrected MIN) is exercised by
    /// `clickhouse_rebuild_accounts_real_run_writes_corrected_min`
    /// below.
    #[tokio::test]
    async fn clickhouse_rebuild_accounts_dry_run_leaves_live_untouched() {
        let Some(sink) = build_ch_sink().await else {
            eprintln!("CLICKHOUSE_URL not set — skipping");
            return;
        };
        let client = sink.client();

        let strkey = "GCQFXHQUTKDRRTPRDB7RH3FNRRJUQB3FA3KZGY42PXTH3FRWXWCATXFE";
        let acct_id = db_clickhouse::persist::ids::account_id(strkey);
        let early = i64::from(TEST_BASE + 5);
        let later = i64::from(TEST_BASE + 50);

        // Cleanup any prior fixture.
        for q in [
            "ALTER TABLE accounts DELETE WHERE id = ?",
            "ALTER TABLE transaction_participants DELETE WHERE account_id = ?",
            "DROP TABLE IF EXISTS accounts_staging_repair_tier1",
        ] {
            let _ = client
                .query(q)
                .bind(acct_id)
                .with_setting("mutations_sync", "1")
                .execute()
                .await;
        }

        // Plant a "collapsed" row: first_seen_ledger says `later` even
        // though earlier participation should set it to `early`.
        let mut accounts = client
            .insert::<AccountRow>("accounts")
            .await
            .expect("open accounts insert");
        accounts
            .write(&AccountRow {
                id: acct_id,
                account_id: strkey.to_string(),
                first_seen_ledger: later,
                last_seen_ledger: later,
                sequence_number: 1,
                home_domain: None,
            })
            .await
            .expect("write account fixture");
        accounts.end().await.expect("close accounts insert");

        // Two participant rows: one at `early`, one at `later`.
        let mut parts = client
            .insert::<TransactionParticipantRow>("transaction_participants")
            .await
            .expect("open participants insert");
        parts
            .write(&TransactionParticipantRow {
                account_id: acct_id,
                ledger_sequence: early,
                transaction_id: 1,
            })
            .await
            .expect("write early participant");
        parts
            .write(&TransactionParticipantRow {
                account_id: acct_id,
                ledger_sequence: later,
                transaction_id: 2,
            })
            .await
            .expect("write later participant");
        parts.end().await.expect("close participants insert");

        // Dry-run: builds the staging table, logs row count, then drops
        // staging in `finalize()`. We can only check the count returned
        // and that the live table is untouched — staging contents are
        // gone by the time this assertion runs.
        let rows = rebuild_accounts(client, /* dry_run */ true)
            .await
            .expect("rebuild_accounts must succeed");
        assert!(rows > 0, "staging must contain rows");

        #[derive(Debug, serde::Deserialize, clickhouse::Row)]
        struct AcctFsl {
            first_seen_ledger: i64,
        }
        let row: AcctFsl = client
            .query("SELECT first_seen_ledger FROM accounts FINAL WHERE id = ? LIMIT 1")
            .bind(acct_id)
            .fetch_one()
            .await
            .expect("read account row after dry-run");
        assert_eq!(
            row.first_seen_ledger, later,
            "dry-run must not touch the live table"
        );

        // Cleanup.
        for q in [
            "ALTER TABLE accounts DELETE WHERE id = ?",
            "ALTER TABLE transaction_participants DELETE WHERE account_id = ?",
            "DROP TABLE IF EXISTS accounts_staging_repair_tier1",
        ] {
            let _ = client
                .query(q)
                .bind(acct_id)
                .with_setting("mutations_sync", "1")
                .execute()
                .await;
        }
    }

    /// Real-run end-to-end: same fixture as the dry-run test, but
    /// invokes `rebuild_accounts(client, dry_run=false)` so the staging
    /// table is built AND EXCHANGE'd with `accounts`. Asserts the live
    /// `accounts.first_seen_ledger` for the fixture account flips from
    /// `later` (planted) to `early` (the actual MIN of participants).
    ///
    /// This is the proof that the rebuild logic produces correct
    /// values — the dry-run test only proves "doesn't touch live".
    ///
    /// Uses a different StrKey from the dry-run test so the two can run
    /// independently (test-threads=1 still recommended on shared CH).
    #[tokio::test]
    async fn clickhouse_rebuild_accounts_real_run_writes_corrected_min() {
        let Some(sink) = build_ch_sink().await else {
            eprintln!("CLICKHOUSE_URL not set — skipping");
            return;
        };
        let client = sink.client();

        // Distinct StrKey from the dry-run test so concurrent runs don't
        // share a fixture row.
        let strkey = "GAQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";
        let acct_id = db_clickhouse::persist::ids::account_id(strkey);
        let early = i64::from(TEST_BASE + 7);
        let later = i64::from(TEST_BASE + 70);

        // Cleanup any prior fixture.
        for q in [
            "ALTER TABLE accounts DELETE WHERE id = ?",
            "ALTER TABLE transaction_participants DELETE WHERE account_id = ?",
            "DROP TABLE IF EXISTS accounts_staging_repair_tier1",
        ] {
            let _ = client
                .query(q)
                .bind(acct_id)
                .with_setting("mutations_sync", "1")
                .execute()
                .await;
        }

        // Plant collapsed row with WRONG first_seen_ledger.
        let mut accounts = client
            .insert::<AccountRow>("accounts")
            .await
            .expect("open accounts insert");
        accounts
            .write(&AccountRow {
                id: acct_id,
                account_id: strkey.to_string(),
                first_seen_ledger: later,
                last_seen_ledger: later,
                sequence_number: 1,
                home_domain: None,
            })
            .await
            .expect("write account fixture");
        accounts.end().await.expect("close accounts insert");

        // Two participant rows at `early` and `later` ledgers.
        let mut parts = client
            .insert::<TransactionParticipantRow>("transaction_participants")
            .await
            .expect("open participants insert");
        parts
            .write(&TransactionParticipantRow {
                account_id: acct_id,
                ledger_sequence: early,
                transaction_id: 11,
            })
            .await
            .expect("write early participant");
        parts
            .write(&TransactionParticipantRow {
                account_id: acct_id,
                ledger_sequence: later,
                transaction_id: 12,
            })
            .await
            .expect("write later participant");
        parts.end().await.expect("close participants insert");

        // REAL RUN — staging built, EXCHANGE TABLES swaps it with
        // `accounts`. After this, FINAL read of the fixture account
        // returns the corrected first_seen_ledger.
        let rows = rebuild_accounts(client, /* dry_run */ false)
            .await
            .expect("rebuild_accounts real-run must succeed");
        assert!(rows > 0, "swap must have touched at least the fixture row");

        #[derive(Debug, serde::Deserialize, clickhouse::Row)]
        struct AcctFsl {
            first_seen_ledger: i64,
            last_seen_ledger: i64,
            sequence_number: i64,
        }
        let row: AcctFsl = client
            .query("SELECT first_seen_ledger, last_seen_ledger, sequence_number FROM accounts FINAL WHERE id = ? LIMIT 1")
            .bind(acct_id)
            .fetch_one()
            .await
            .expect("read account row after real run");
        assert_eq!(
            row.first_seen_ledger, early,
            "real run must REPLACE first_seen_ledger with MIN(participants.ledger_sequence) = {early}"
        );
        // Other columns must be preserved (RMT version + non-rebuilt fields).
        assert_eq!(
            row.last_seen_ledger, later,
            "last_seen_ledger must pass through unchanged"
        );
        assert_eq!(
            row.sequence_number, 1,
            "sequence_number must pass through unchanged"
        );

        // Cleanup. Note: after EXCHANGE, the original `accounts` table
        // is renamed to `accounts_staging_repair_tier1` and dropped by
        // finalize(). The current `accounts` is the rebuilt staging.
        for q in [
            "ALTER TABLE accounts DELETE WHERE id = ?",
            "ALTER TABLE transaction_participants DELETE WHERE account_id = ?",
            "DROP TABLE IF EXISTS accounts_staging_repair_tier1",
        ] {
            let _ = client
                .query(q)
                .bind(acct_id)
                .with_setting("mutations_sync", "1")
                .execute()
                .await;
        }
    }
}
