//! Integration tests for the ADR 0027 write-path (task 0149).
//!
//! Gated on `DATABASE_URL`. Skips cleanly when no database is reachable so CI
//! jobs without Postgres don't fail spuriously. Run locally:
//!
//!   docker compose up -d
//!   npm run db:migrate
//!   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
//!       cargo test -p indexer --test persist_integration -- --test-threads=1
//!
//! The test uses a dedicated ledger sequence (`TEST_LEDGER_SEQ`) so concurrent
//! runs don't stomp each other. It ensures DEFAULT partitions exist on every
//! partitioned table (the monthly-range partitions are provisioned by
//! `db-partition-mgmt` in production; default partitions make the write-path
//! work in isolation).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use indexer::handler::persist::persist_ledger;
use serde_json::json;
use sqlx::{PgPool, Row};
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedContractInterface, ExtractedEvent,
    ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedLpPosition, ExtractedNft, ExtractedNftEvent, ExtractedOperation, ExtractedToken,
    ExtractedTransaction,
};

const TEST_LEDGER_SEQ: u32 = 90_000_001;
/// 2026-04-21 12:00:00 UTC — arbitrary, stable across runs.
const TEST_CLOSED_AT: i64 = 1_777_118_400;

const SRC_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAASRC";
const DST_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADST";
const ISSUER_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAISSUER";
const TOKEN_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAASAC";
const NFT_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAANFT";
const TEST_TX_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TEST_LEDGER_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const POOL_ID: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const WASM_HASH: &str = "4444444444444444444444444444444444444444444444444444444444444444";

#[tokio::test]
async fn synthetic_ledger_insert_and_replay_is_idempotent() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping persist integration test");
        return;
    };

    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping persist integration test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_test_ledger(&pool).await;

    let ledger = make_ledger();
    let transactions = vec![make_transaction()];
    let operations = vec![(
        TEST_TX_HASH.to_string(),
        vec![make_payment_op(), make_invoke_op()],
    )];
    let events = vec![(TEST_TX_HASH.to_string(), vec![make_transfer_event()])];
    let invocations = vec![(TEST_TX_HASH.to_string(), vec![make_invocation()])];
    let operation_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let contract_interfaces = vec![make_contract_interface()];
    let contract_deployments = vec![make_contract_deployment()];
    let account_states = vec![make_account_state()];
    let liquidity_pools = vec![make_liquidity_pool()];
    let pool_snapshots = vec![make_pool_snapshot()];
    let tokens = vec![make_sac_token()];
    let nfts = vec![make_nft()];
    let nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();

    // --- First insert ---
    persist_ledger(
        &pool,
        &ledger,
        &transactions,
        &operations,
        &events,
        &invocations,
        &operation_trees,
        &contract_interfaces,
        &contract_deployments,
        &account_states,
        &liquidity_pools,
        &pool_snapshots,
        &tokens,
        &nfts,
        &nft_events,
        &lp_positions,
        &inner_tx_hashes,
    )
    .await
    .expect("first persist_ledger failed");

    let counts_first = test_counts(&pool).await;
    assert_eq!(counts_first.ledgers, 1, "ledgers row count");
    assert!(
        counts_first.accounts >= 3,
        "accounts touched (src+dst+issuer+…)"
    );
    assert_eq!(counts_first.transactions, 1, "transactions row count");
    assert_eq!(
        counts_first.hash_index, 1,
        "transaction_hash_index row count"
    );
    assert!(counts_first.participants >= 2, "participants ≥ 2");
    assert_eq!(counts_first.operations, 2, "operations row count");
    assert_eq!(counts_first.events, 1, "soroban_events row count");
    assert_eq!(counts_first.invocations, 1, "soroban_invocations row count");
    assert!(counts_first.contracts >= 1, "contracts row count");
    assert_eq!(counts_first.wasm, 1, "wasm_interface_metadata row count");
    assert_eq!(counts_first.tokens, 1, "tokens row count");
    assert_eq!(counts_first.nfts, 1, "nfts row count");
    assert_eq!(counts_first.pools, 1, "liquidity_pools row count");
    assert_eq!(
        counts_first.pool_snapshots, 1,
        "liquidity_pool_snapshots row count"
    );
    assert!(
        counts_first.balances_current >= 1,
        "account_balances_current row count"
    );
    assert!(
        counts_first.balance_history >= 1,
        "account_balance_history row count"
    );

    // Parser does not yet produce these today.
    assert_eq!(
        counts_first.nft_ownership, 0,
        "nft_ownership expected empty"
    );
    assert_eq!(counts_first.lp_positions, 0, "lp_positions expected empty");

    // --- Replay — counts must not change ---
    persist_ledger(
        &pool,
        &ledger,
        &transactions,
        &operations,
        &events,
        &invocations,
        &operation_trees,
        &contract_interfaces,
        &contract_deployments,
        &account_states,
        &liquidity_pools,
        &pool_snapshots,
        &tokens,
        &nfts,
        &nft_events,
        &lp_positions,
        &inner_tx_hashes,
    )
    .await
    .expect("replay persist_ledger failed");

    let counts_replay = test_counts(&pool).await;
    assert_eq!(counts_replay, counts_first, "replay must be idempotent");
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn make_ledger() -> ExtractedLedger {
    ExtractedLedger {
        sequence: TEST_LEDGER_SEQ,
        hash: TEST_LEDGER_HASH.to_string(),
        closed_at: TEST_CLOSED_AT,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    }
}

fn make_transaction() -> ExtractedTransaction {
    ExtractedTransaction {
        hash: TEST_TX_HASH.to_string(),
        ledger_sequence: TEST_LEDGER_SEQ,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 1000,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: TEST_CLOSED_AT,
        parse_error: false,
    }
}

fn make_payment_op() -> ExtractedOperation {
    ExtractedOperation {
        transaction_hash: TEST_TX_HASH.to_string(),
        operation_index: 0,
        op_type: "PAYMENT".to_string(),
        source_account: None,
        details: json!({
            "destination": DST_STRKEY,
            "asset": format!("USDC:{ISSUER_STRKEY}"),
            "amount": 50_000_000i64,
        }),
    }
}

fn make_invoke_op() -> ExtractedOperation {
    ExtractedOperation {
        transaction_hash: TEST_TX_HASH.to_string(),
        operation_index: 1,
        op_type: "INVOKE_HOST_FUNCTION".to_string(),
        source_account: None,
        details: json!({
            "hostFunctionType": "invokeContract",
            "contractId": TOKEN_CONTRACT,
            "functionName": "transfer",
            "functionArgs": [],
            "returnValue": serde_json::Value::Null,
        }),
    }
}

fn make_transfer_event() -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: TEST_TX_HASH.to_string(),
        event_type: "contract".to_string(),
        contract_id: Some(TOKEN_CONTRACT.to_string()),
        topics: json!([
            {"type": "sym", "value": "transfer"},
            {"type": "address", "value": SRC_STRKEY},
            {"type": "address", "value": DST_STRKEY},
        ]),
        data: json!({"type": "i128", "value": "50000000"}),
        event_index: 0,
        ledger_sequence: TEST_LEDGER_SEQ,
        created_at: TEST_CLOSED_AT,
    }
}

fn make_invocation() -> ExtractedInvocation {
    ExtractedInvocation {
        transaction_hash: TEST_TX_HASH.to_string(),
        contract_id: Some(TOKEN_CONTRACT.to_string()),
        caller_account: Some(SRC_STRKEY.to_string()),
        function_name: Some("transfer".to_string()),
        function_args: json!([]),
        return_value: serde_json::Value::Null,
        successful: true,
        invocation_index: 0,
        depth: 0,
        ledger_sequence: TEST_LEDGER_SEQ,
        created_at: TEST_CLOSED_AT,
    }
}

fn make_contract_interface() -> ExtractedContractInterface {
    ExtractedContractInterface {
        wasm_hash: WASM_HASH.to_string(),
        functions: Vec::new(),
        wasm_byte_len: 256,
    }
}

fn make_contract_deployment() -> ExtractedContractDeployment {
    ExtractedContractDeployment {
        contract_id: TOKEN_CONTRACT.to_string(),
        wasm_hash: Some(WASM_HASH.to_string()),
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: TEST_LEDGER_SEQ,
        contract_type: "token".to_string(),
        is_sac: true,
        metadata: json!({"name": "TEST"}),
    }
}

fn make_account_state() -> ExtractedAccountState {
    ExtractedAccountState {
        account_id: SRC_STRKEY.to_string(),
        first_seen_ledger: Some(TEST_LEDGER_SEQ),
        last_seen_ledger: TEST_LEDGER_SEQ,
        sequence_number: 42,
        balances: json!([
            {"asset_type": "native", "balance": "1.0000000"},
            {"asset_type": "credit_alphanum4", "asset_code": "USDC", "issuer": ISSUER_STRKEY, "balance": "5.0000000"},
        ]),
        removed_trustlines: Vec::new(),
        home_domain: Some("example.com".to_string()),
        created_at: TEST_CLOSED_AT,
    }
}

fn make_liquidity_pool() -> ExtractedLiquidityPool {
    ExtractedLiquidityPool {
        pool_id: POOL_ID.to_string(),
        asset_a: json!("native"),
        asset_b: json!({"type": "credit_alphanum4", "code": "USDC", "issuer": ISSUER_STRKEY}),
        fee_bps: 30,
        reserves: json!({"a": 1_000_000i64, "b": 2_000_000i64}),
        total_shares: "1414213".to_string(),
        tvl: None,
        created_at_ledger: Some(TEST_LEDGER_SEQ),
        last_updated_ledger: TEST_LEDGER_SEQ,
        created_at: TEST_CLOSED_AT,
    }
}

fn make_pool_snapshot() -> ExtractedLiquidityPoolSnapshot {
    ExtractedLiquidityPoolSnapshot {
        pool_id: POOL_ID.to_string(),
        ledger_sequence: TEST_LEDGER_SEQ,
        created_at: TEST_CLOSED_AT,
        reserves: json!({"a": 1_000_000i64, "b": 2_000_000i64}),
        total_shares: "1414213".to_string(),
        tvl: None,
        volume: None,
        fee_revenue: None,
    }
}

fn make_sac_token() -> ExtractedToken {
    ExtractedToken {
        asset_type: "sac".to_string(),
        asset_code: Some("USDC".to_string()),
        issuer_address: Some(ISSUER_STRKEY.to_string()),
        contract_id: Some(TOKEN_CONTRACT.to_string()),
        name: Some("USDC".to_string()),
        total_supply: None,
        holder_count: None,
    }
}

fn make_nft() -> ExtractedNft {
    // nfts.contract_id FK → soroban_contracts(contract_id). Use the token
    // contract we already deployed so the test doesn't have to double up.
    ExtractedNft {
        contract_id: NFT_CONTRACT.to_string(),
        token_id: "1".to_string(),
        collection_name: Some("Test".to_string()),
        owner_account: Some(DST_STRKEY.to_string()),
        name: Some("NFT #1".to_string()),
        media_url: None,
        metadata: Some(json!({"rarity": "common"})),
        minted_at_ledger: Some(TEST_LEDGER_SEQ),
        last_seen_ledger: TEST_LEDGER_SEQ,
        created_at: TEST_CLOSED_AT,
    }
}

// ---------------------------------------------------------------------------
// DB setup + row-count helpers
// ---------------------------------------------------------------------------

async fn ensure_default_partitions(pool: &PgPool) {
    // Default partitions catch any rows not covered by a monthly range. In
    // production, `db-partition-mgmt` pre-creates monthly partitions; in the
    // test we rely on these defaults so the per-ledger inserts land somewhere.
    for table in [
        "transactions",
        "operations",
        "transaction_participants",
        "soroban_events",
        "soroban_invocations",
        "nft_ownership",
        "liquidity_pool_snapshots",
        "account_balance_history",
    ] {
        let default_name = format!("{table}_default");
        let ddl = format!("CREATE TABLE IF NOT EXISTS {default_name} PARTITION OF {table} DEFAULT");
        if let Err(err) = sqlx::query(&ddl).execute(pool).await {
            // If the default already exists under a different form, ignore.
            eprintln!("default partition create warning for {table}: {err}");
        }
    }
}

async fn clean_test_ledger(pool: &PgPool) {
    // Children cascade on DELETE FROM transactions via composite FK. Pools,
    // accounts, tokens, nfts etc need explicit cleanup so repeated runs start
    // from zero state for the test fixture's identifiers.
    let sql_stmts = [
        // Delete test-specific leaves first.
        "DELETE FROM lp_positions WHERE pool_id = decode($1, 'hex')",
        "DELETE FROM liquidity_pool_snapshots WHERE pool_id = decode($1, 'hex')",
        "DELETE FROM liquidity_pools WHERE pool_id = decode($1, 'hex')",
    ];
    for sql in sql_stmts {
        let _ = sqlx::query(sql).bind(POOL_ID).execute(pool).await;
    }
    let _ = sqlx::query("DELETE FROM nft_ownership WHERE nft_id IN (SELECT id FROM nfts WHERE contract_id IN ($1, $2))")
        .bind(TOKEN_CONTRACT)
        .bind(NFT_CONTRACT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM nfts WHERE contract_id IN ($1, $2)")
        .bind(TOKEN_CONTRACT)
        .bind(NFT_CONTRACT)
        .execute(pool)
        .await;
    // soroban_events / invocations / operations / participants cascade via FK
    // on (transaction_id, created_at). Deleting the parent transactions wipes
    // them.
    let _ = sqlx::query("DELETE FROM transactions WHERE hash = decode($1, 'hex')")
        .bind(TEST_TX_HASH)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM transaction_hash_index WHERE hash = decode($1, 'hex')")
        .bind(TEST_TX_HASH)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
        .bind(i64::from(TEST_LEDGER_SEQ))
        .execute(pool)
        .await;
    // tokens — delete anything referencing our SAC contract_id to start clean.
    let _ = sqlx::query("DELETE FROM tokens WHERE contract_id IN ($1, $2)")
        .bind(TOKEN_CONTRACT)
        .bind(NFT_CONTRACT)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM tokens WHERE asset_type IN ('classic','sac') AND issuer_id IN (SELECT id FROM accounts WHERE account_id = $1)"
    )
    .bind(ISSUER_STRKEY)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM account_balances_current WHERE account_id IN (SELECT id FROM accounts WHERE account_id = ANY($1))")
        .bind(vec![SRC_STRKEY.to_string(), DST_STRKEY.to_string(), ISSUER_STRKEY.to_string()])
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM account_balance_history WHERE ledger_sequence = $1")
        .bind(i64::from(TEST_LEDGER_SEQ))
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM soroban_contracts WHERE contract_id IN ($1, $2)")
        .bind(TOKEN_CONTRACT)
        .bind(NFT_CONTRACT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = decode($1, 'hex')")
        .bind(WASM_HASH)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = ANY($1)")
        .bind(vec![
            SRC_STRKEY.to_string(),
            DST_STRKEY.to_string(),
            ISSUER_STRKEY.to_string(),
        ])
        .execute(pool)
        .await;
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    ledgers: i64,
    accounts: i64,
    transactions: i64,
    hash_index: i64,
    participants: i64,
    operations: i64,
    events: i64,
    invocations: i64,
    contracts: i64,
    wasm: i64,
    tokens: i64,
    nfts: i64,
    nft_ownership: i64,
    pools: i64,
    pool_snapshots: i64,
    lp_positions: i64,
    balances_current: i64,
    balance_history: i64,
}

async fn test_counts(pool: &PgPool) -> Counts {
    // Restrict counts to rows tied to our fixtures so pre-existing DB content
    // doesn't poison the assertions.
    let ledger = i64::from(TEST_LEDGER_SEQ);
    let row = sqlx::query(
        r#"
        WITH
          l AS (SELECT COUNT(*) AS n FROM ledgers WHERE sequence = $1),
          a AS (SELECT COUNT(*) AS n FROM accounts WHERE account_id = ANY($2)),
          t AS (SELECT COUNT(*) AS n FROM transactions WHERE hash = decode($3, 'hex')),
          hi AS (SELECT COUNT(*) AS n FROM transaction_hash_index WHERE hash = decode($3, 'hex')),
          p AS (SELECT COUNT(*) AS n FROM transaction_participants tp
                   JOIN transactions tx ON tx.id = tp.transaction_id AND tx.created_at = tp.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          o AS (SELECT COUNT(*) AS n FROM operations op
                   JOIN transactions tx ON tx.id = op.transaction_id AND tx.created_at = op.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          e AS (SELECT COUNT(*) AS n FROM soroban_events ev
                   JOIN transactions tx ON tx.id = ev.transaction_id AND tx.created_at = ev.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          iv AS (SELECT COUNT(*) AS n FROM soroban_invocations inv
                   JOIN transactions tx ON tx.id = inv.transaction_id AND tx.created_at = inv.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          c AS (SELECT COUNT(*) AS n FROM soroban_contracts WHERE contract_id = ANY($4)),
          w AS (SELECT COUNT(*) AS n FROM wasm_interface_metadata WHERE wasm_hash = decode($5, 'hex')),
          tk AS (SELECT COUNT(*) AS n FROM tokens WHERE contract_id = ANY($4)),
          n AS (SELECT COUNT(*) AS n FROM nfts WHERE contract_id = ANY($4)),
          no AS (SELECT COUNT(*) AS n FROM nft_ownership no2
                   JOIN nfts nf ON nf.id = no2.nft_id
                  WHERE nf.contract_id = ANY($4)),
          pl AS (SELECT COUNT(*) AS n FROM liquidity_pools WHERE pool_id = decode($6, 'hex')),
          ps AS (SELECT COUNT(*) AS n FROM liquidity_pool_snapshots WHERE pool_id = decode($6, 'hex')),
          lp AS (SELECT COUNT(*) AS n FROM lp_positions WHERE pool_id = decode($6, 'hex')),
          bc AS (SELECT COUNT(*) AS n FROM account_balances_current abc
                   JOIN accounts aa ON aa.id = abc.account_id
                  WHERE aa.account_id = ANY($2)),
          bh AS (SELECT COUNT(*) AS n FROM account_balance_history abh
                   JOIN accounts aa ON aa.id = abh.account_id
                  WHERE aa.account_id = ANY($2) AND abh.ledger_sequence = $1)
        SELECT l.n AS l, a.n AS a, t.n AS t, hi.n AS hi, p.n AS p, o.n AS o,
               e.n AS e, iv.n AS iv, c.n AS c, w.n AS w, tk.n AS tk, n.n AS n,
               no.n AS no, pl.n AS pl, ps.n AS ps, lp.n AS lp, bc.n AS bc, bh.n AS bh
          FROM l, a, t, hi, p, o, e, iv, c, w, tk, n, no, pl, ps, lp, bc, bh
        "#,
    )
    .bind(ledger)
    .bind(vec![
        SRC_STRKEY.to_string(),
        DST_STRKEY.to_string(),
        ISSUER_STRKEY.to_string(),
    ])
    .bind(TEST_TX_HASH)
    .bind(vec![TOKEN_CONTRACT.to_string(), NFT_CONTRACT.to_string()])
    .bind(WASM_HASH)
    .bind(POOL_ID)
    .fetch_one(pool)
    .await
    .expect("counts query");

    Counts {
        ledgers: row.get("l"),
        accounts: row.get("a"),
        transactions: row.get("t"),
        hash_index: row.get("hi"),
        participants: row.get("p"),
        operations: row.get("o"),
        events: row.get("e"),
        invocations: row.get("iv"),
        contracts: row.get("c"),
        wasm: row.get("w"),
        tokens: row.get("tk"),
        nfts: row.get("n"),
        nft_ownership: row.get("no"),
        pools: row.get("pl"),
        pool_snapshots: row.get("ps"),
        lp_positions: row.get("lp"),
        balances_current: row.get("bc"),
        balance_history: row.get("bh"),
    }
}

// Touch DateTime<Utc> so the compiler picks up the chrono dep even if all
// usages become conditional later.
#[allow(dead_code)]
fn _touch(_: DateTime<Utc>) {}
