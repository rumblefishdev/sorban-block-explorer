//! The 0279 targeted write must persist `lp_operation_amounts` and NOTHING
//! else.
//!
//! That promise is what keeps the historical re-parse additive: a run that
//! also re-emitted the other tables would rewrite the 12 Tier-1 columns which
//! cannot survive parallel `ReplacingMergeTree` collapse, and would owe a
//! `repair-tier1` pass afterwards (`docs/backfills.md` §3). It is also silent
//! when broken — the extra rows are valid, they just quietly re-arm that
//! obligation — so it gets an assertion rather than a comment.
//!
//! Gated on `CLICKHOUSE_URL`, like every other CH test here: skipped cleanly
//! when no instance is reachable.
//!
//! ```bash
//! CLICKHOUSE_URL=http://localhost:8123 \
//!     cargo test -p db-clickhouse --test lp_amounts_targeted_write_e2e
//! ```

use db_clickhouse::persist::rows::{LedgerRow, LpOperationAmountRow};
use db_clickhouse::persist::stage::StagedLedger;
use db_clickhouse::persist::{PartitionWriter, TargetedTables};
use db_clickhouse::{Config, apply_init_sql, client};

/// Out-of-band sentinel, same convention as `smoke.rs`.
const TEST_LEDGER: i64 = 99_999_301;

#[tokio::test]
async fn targeted_write_persists_only_lp_operation_amounts() {
    let Some(url) = std::env::var("CLICKHOUSE_URL").ok() else {
        eprintln!("CLICKHOUSE_URL not set — skipping");
        return;
    };
    let cfg = Config {
        url,
        ..Config::from_env()
    };
    let ch = client(&cfg);
    apply_init_sql(&ch).await.expect("apply init.sql");

    for table in ["lp_operation_amounts", "ledgers"] {
        ch.query(&format!(
            "ALTER TABLE {table} DELETE WHERE {} = ?",
            if table == "ledgers" {
                "sequence"
            } else {
                "ledger_sequence"
            }
        ))
        .bind(TEST_LEDGER)
        .with_setting("mutations_sync", "1")
        .execute()
        .await
        .expect("cleanup");
    }

    // A staged ledger carrying BOTH kinds of row: the amounts we want and a
    // `ledgers` commit marker we must not get.
    let staged = StagedLedger {
        ledger_sequence: TEST_LEDGER,
        ledger_rows: vec![LedgerRow {
            sequence: TEST_LEDGER,
            hash: [0x7d; 32],
            closed_at: 1_760_000_000_000,
            protocol_version: 23,
            transaction_count: 1,
            base_fee: 100,
        }],
        lp_amount_rows: vec![LpOperationAmountRow {
            pool_id: [0x44; 32],
            ledger_sequence: TEST_LEDGER,
            transaction_id: 7,
            application_order: 1,
            asset_id: 42,
            amount: -1_000,
        }],
        ..Default::default()
    };

    let mut writer = PartitionWriter::open(ch.clone());
    let only = TargetedTables::parse("lp_operation_amounts").expect("targetable");
    writer
        .write_only(&staged, &only)
        .await
        .expect("targeted write");
    writer.commit().await.expect("commit");

    let amounts: u64 = ch
        .query("SELECT count() FROM lp_operation_amounts WHERE ledger_sequence = ?")
        .bind(TEST_LEDGER)
        .fetch_one()
        .await
        .expect("count amounts");
    assert_eq!(amounts, 1, "the targeted table must receive its row");

    // The marker is the canary: `write_ledger` would have buffered and
    // flushed it on commit, so its absence proves the other 20-odd tables
    // were skipped too.
    let markers: u64 = ch
        .query("SELECT count() FROM ledgers WHERE sequence = ?")
        .bind(TEST_LEDGER)
        .fetch_one()
        .await
        .expect("count ledgers");
    assert_eq!(
        markers, 0,
        "targeted write must not write the ledgers commit marker"
    );

    for table in ["lp_operation_amounts", "ledgers"] {
        let _ = ch
            .query(&format!(
                "ALTER TABLE {table} DELETE WHERE {} = ?",
                if table == "ledgers" {
                    "sequence"
                } else {
                    "ledger_sequence"
                }
            ))
            .bind(TEST_LEDGER)
            .with_setting("mutations_sync", "1")
            .execute()
            .await;
    }
}

/// Task 0540: the generalised `--only` write persists exactly the named
/// tables. Also the first place the three new row structs meet a real
/// ClickHouse — `Option<i128>`, `Option<u64>` and `LowCardinality(String)`
/// over RowBinary — which is the driver-vs-`DESCRIBE` check task 0310 taught
/// us to run before a deploy, not after.
#[tokio::test]
async fn write_only_persists_the_value_flow_tables_and_nothing_else() {
    use db_clickhouse::persist::rows::{AssetTransferRow, SorobanEventOpRow, TransactionMemoRow};

    let Some(url) = std::env::var("CLICKHOUSE_URL").ok() else {
        eprintln!("CLICKHOUSE_URL not set — skipping");
        return;
    };
    let cfg = Config {
        url,
        ..Config::from_env()
    };
    let ch = client(&cfg);
    apply_init_sql(&ch).await.expect("apply init.sql");

    const LEDGER: i64 = 99_999_302;
    for table in [
        "asset_transfers",
        "transaction_memos",
        "soroban_event_ops",
        "lp_operation_amounts",
        "ledgers",
    ] {
        ch.query(&format!(
            "ALTER TABLE {table} DELETE WHERE {} = ?",
            if table == "ledgers" {
                "sequence"
            } else {
                "ledger_sequence"
            }
        ))
        .bind(LEDGER)
        .with_setting("mutations_sync", "1")
        .execute()
        .await
        .expect("cleanup");
    }

    let staged = StagedLedger {
        ledger_sequence: LEDGER,
        ledger_rows: vec![LedgerRow {
            sequence: LEDGER,
            hash: [0x7e; 32],
            closed_at: 1_760_000_000_000,
            protocol_version: 23,
            transaction_count: 1,
            base_fee: 100,
        }],
        // Must NOT land: not in the `--only` list.
        lp_amount_rows: vec![LpOperationAmountRow {
            pool_id: [0x44; 32],
            ledger_sequence: LEDGER,
            transaction_id: 7,
            application_order: 1,
            asset_id: 42,
            amount: -1_000,
        }],
        asset_transfer_rows: vec![
            AssetTransferRow {
                ledger_sequence: LEDGER,
                application_order: 1,
                op_index: 0,
                event_pos_in_op: 0,
                event_index: 2,
                asset_id: -6_959_166_271_784_855_184,
                amount: Some(10_000),
                from_id: Some(11),
                from_kind: "G".into(),
                from_muxed_id: None,
                to_id: Some(22),
                to_kind: "G".into(),
                to_muxed_id: Some(3_539_365_402),
                verb: "transfer".into(),
            },
            // non-fungible: NULL amount, no `to`
            AssetTransferRow {
                ledger_sequence: LEDGER,
                application_order: 1,
                op_index: 0,
                event_pos_in_op: 1,
                event_index: 3,
                asset_id: 99,
                amount: None,
                from_id: Some(11),
                from_kind: "G".into(),
                from_muxed_id: None,
                to_id: None,
                to_kind: String::new(),
                to_muxed_id: None,
                verb: "burn".into(),
            },
        ],
        transaction_memo_rows: vec![TransactionMemoRow {
            ledger_sequence: LEDGER,
            application_order: 1,
            memo_type: "text".into(),
            memo: "pspb:5721732".into(),
        }],
        event_op_rows: vec![SorobanEventOpRow {
            ledger_sequence: LEDGER,
            application_order: 7,
            event_index: 2,
            op_index: 0,
            event_pos_in_op: 0,
        }],
        ..Default::default()
    };

    let only = TargetedTables::parse("asset_transfers,transaction_memos,soroban_event_ops")
        .expect("valid table list");
    let mut writer = PartitionWriter::open(ch.clone());
    writer.write_only(&staged, &only).await.expect("write_only");
    writer.commit().await.expect("commit");

    let count = |sql: &'static str| {
        let ch = ch.clone();
        async move {
            ch.query(sql)
                .bind(LEDGER)
                .fetch_one::<u64>()
                .await
                .expect(sql)
        }
    };
    assert_eq!(
        count("SELECT count() FROM asset_transfers WHERE ledger_sequence = ?").await,
        2
    );
    assert_eq!(
        count("SELECT count() FROM transaction_memos WHERE ledger_sequence = ?").await,
        1
    );
    assert_eq!(
        count("SELECT count() FROM soroban_event_ops WHERE ledger_sequence = ?").await,
        1
    );
    assert_eq!(
        count("SELECT count() FROM lp_operation_amounts WHERE ledger_sequence = ?").await,
        0,
        "a table outside the --only list must not be written"
    );
    assert_eq!(
        count("SELECT count() FROM ledgers WHERE sequence = ?").await,
        0,
        "the targeted write must not plant a ledgers commit marker"
    );

    // Round-trip the nullable / muxed columns: what went in is what is stored.
    let (amount, to_muxed, verb): (Option<i128>, Option<u64>, String) = ch
        .query(
            "SELECT amount, to_muxed_id, verb FROM asset_transfers \
             WHERE ledger_sequence = ? AND event_pos_in_op = 0",
        )
        .bind(LEDGER)
        .fetch_one()
        .await
        .expect("read back");
    assert_eq!(
        (amount, to_muxed, verb.as_str()),
        (Some(10_000), Some(3_539_365_402), "transfer")
    );
    let nf: Option<i128> = ch
        .query(
            "SELECT amount FROM asset_transfers WHERE ledger_sequence = ? AND event_pos_in_op = 1",
        )
        .bind(LEDGER)
        .fetch_one()
        .await
        .expect("read back nf");
    assert_eq!(nf, None);
}

/// Task 0518 — the three pool tables joined the targetable list so the 0540
/// full-range re-parse carries the pool families' whole history in the SAME
/// descent instead of owing a second one (and, for the config family, a
/// separate ~40 GB targeted fetch).
///
/// Two things are asserted, because two things can silently go wrong:
///
/// 1. all three land, and nothing outside the list does — the same promise the
///    tests above pin for the value-flow tables;
/// 2. a registry row that TIES on `last_updated_ledger` with one already in
///    the table replaces it. That is the whole basis for pointing a re-parse
///    at `liquidity_pools`: the backfill re-emits a pool at the same
///    last-change ledger the original ingest used, so every row it writes is a
///    version tie, and a tie that resolved the other way would leave the 86% of
///    classic pools with empty `legs` exactly as they are while reporting
///    success. Merge order (last insert wins) is what makes it work, which is
///    also why the run owes an `OPTIMIZE ... FINAL` before anything reads with
///    `argMax` — until the merge, both rows are live and `argMax` may pick
///    either.
#[tokio::test]
async fn targeted_write_persists_pool_tables_and_a_tied_registry_row_replaces() {
    use db_clickhouse::persist::rows::{
        LiquidityPoolRow, PoolInstanceStateRow, PoolStateChangeRow,
    };

    const LEDGER: i64 = 99_999_303;
    const POOL: [u8; 32] = [0x51; 32];

    let Some(url) = std::env::var("CLICKHOUSE_URL").ok() else {
        eprintln!("CLICKHOUSE_URL not set — skipping");
        return;
    };
    let cfg = Config {
        url,
        ..Config::from_env()
    };
    let ch = client(&cfg);
    apply_init_sql(&ch).await.expect("apply init.sql");

    for (table, col, val) in [
        ("pool_state_changes", "ledger_sequence", LEDGER),
        ("ledgers", "sequence", LEDGER),
    ] {
        ch.query(&format!("ALTER TABLE {table} DELETE WHERE {col} = ?"))
            .bind(val)
            .with_setting("mutations_sync", "1")
            .execute()
            .await
            .expect("cleanup");
    }
    let pool_hex = POOL.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for table in ["pool_instance_state", "liquidity_pools"] {
        ch.query(&format!(
            "ALTER TABLE {table} DELETE WHERE pool_id = unhex(?)"
        ))
        .bind(&pool_hex)
        .with_setting("mutations_sync", "1")
        .execute()
        .await
        .expect("cleanup");
    }

    let staged = StagedLedger {
        ledger_sequence: LEDGER,
        ledger_rows: vec![LedgerRow {
            sequence: LEDGER,
            hash: [0x51; 32],
            closed_at: 1_760_000_000_000,
            protocol_version: 23,
            transaction_count: 1,
            base_fee: 100,
        }],
        pool_state_change_rows: vec![PoolStateChangeRow {
            pool_id: POOL,
            ledger_sequence: LEDGER,
            reserves: vec![1_000, 2_000],
            plane_id: 4_242,
        }],
        pool_instance_state_rows: vec![PoolInstanceStateRow {
            pool_id: POOL,
            plane_id: 4_242,
            share_token_id: 777,
            total_shares: 3_000,
            derived_at_ledger: LEDGER,
        }],
        pool_rows: vec![LiquidityPoolRow {
            pool_id: POOL,
            fee_bps: 30,
            last_updated_ledger: LEDGER,
            pool_kind: 1,
            legs: vec![1, 2],
            deployment_id: 9_999,
            pool_type_raw: "xyk".into(),
        }],
        ..Default::default()
    };

    // The row the original ingest already wrote for this pool: SAME pool, SAME
    // `last_updated_ledger`, empty `legs` — the shape 45,603 of 52,798 classic
    // pools are in on production today.
    let mut pre_existing = staged.pool_rows[0].clone();
    pre_existing.legs = Vec::new();
    let mut seed = PartitionWriter::open(ch.clone());
    seed.write_only(
        &StagedLedger {
            ledger_sequence: LEDGER,
            pool_rows: vec![pre_existing],
            ..Default::default()
        },
        &TargetedTables::parse("liquidity_pools").expect("targetable"),
    )
    .await
    .expect("seed the pre-existing row");
    seed.commit().await.expect("seed commit");

    let only = TargetedTables::parse("pool_state_changes,pool_instance_state,liquidity_pools")
        .expect("all three pool tables are targetable");
    let mut writer = PartitionWriter::open(ch.clone());
    writer.write_only(&staged, &only).await.expect("write_only");
    writer.commit().await.expect("commit");
    ch.query("OPTIMIZE TABLE liquidity_pools FINAL")
        .execute()
        .await
        .expect("collapse the version tie");

    let by_ledger = |sql: &'static str| {
        let ch = ch.clone();
        async move {
            ch.query(sql)
                .bind(LEDGER)
                .fetch_one::<u64>()
                .await
                .expect(sql)
        }
    };
    let by_pool = |sql: &'static str| {
        let ch = ch.clone();
        let hex = pool_hex.clone();
        async move { ch.query(sql).bind(hex).fetch_one::<u64>().await.expect(sql) }
    };

    assert_eq!(
        by_ledger("SELECT count() FROM pool_state_changes WHERE ledger_sequence = ?").await,
        1
    );
    assert_eq!(
        by_pool("SELECT count() FROM pool_instance_state WHERE pool_id = unhex(?)").await,
        1
    );
    assert_eq!(
        by_pool("SELECT count() FROM liquidity_pools WHERE pool_id = unhex(?)").await,
        1,
        "the tie must collapse to ONE row, not accumulate"
    );
    let legs: Vec<i64> = ch
        .query("SELECT legs FROM liquidity_pools WHERE pool_id = unhex(?)")
        .bind(&pool_hex)
        .fetch_one()
        .await
        .expect("read back legs");
    assert_eq!(
        legs,
        vec![1, 2],
        "on a version tie the re-parse row (inserted last) must win — otherwise \
         the classic `legs` migration silently does nothing"
    );
    assert_eq!(
        by_ledger("SELECT count() FROM ledgers WHERE sequence = ?").await,
        0,
        "the targeted write must not plant a ledgers commit marker"
    );

    // The reserve pair survives the round trip — a half-pair here would be the
    // config family's "both-or-neither" invariant broken at the sink.
    let (reserves, plane): (Vec<i128>, i64) = ch
        .query("SELECT reserves, plane_id FROM pool_state_changes WHERE ledger_sequence = ?")
        .bind(LEDGER)
        .fetch_one()
        .await
        .expect("read back reserves");
    assert_eq!((reserves, plane), (vec![1_000, 2_000], 4_242));
}
