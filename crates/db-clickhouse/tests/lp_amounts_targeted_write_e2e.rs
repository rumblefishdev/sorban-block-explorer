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

use db_clickhouse::persist::PartitionWriter;
use db_clickhouse::persist::rows::{LedgerRow, LpOperationAmountRow};
use db_clickhouse::persist::stage::StagedLedger;
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
    writer
        .write_lp_amounts_only(&staged)
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
    use db_clickhouse::persist::TargetedTables;
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
