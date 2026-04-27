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
use domain::{
    AssetType, ContractEventType, ContractType, NftEventType, OperationType, TokenAssetType,
};
use indexer::handler::persist::{ClassificationCache, persist_ledger};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use xdr_parser::types::{
    ContractFunction, ExtractedAccountState, ExtractedAsset, ExtractedContractDeployment,
    ExtractedContractInterface, ExtractedEvent, ExtractedInvocation, ExtractedLedger,
    ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedLpPosition, ExtractedNft,
    ExtractedNftEvent, ExtractedOperation, ExtractedTransaction,
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
    let assets = vec![make_sac_asset()];
    let nfts = vec![make_nft()];
    let nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let classification_cache = ClassificationCache::new();

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
        &assets,
        &nfts,
        &nft_events,
        &lp_positions,
        &inner_tx_hashes,
        &classification_cache,
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
    assert_eq!(
        counts_first.events, 1,
        "soroban_events_appearances row count — one (contract, tx, ledger) trio"
    );
    assert_eq!(
        counts_first.events_amount_sum, 1,
        "SUM(amount) must equal the ingested non-diagnostic event count (ADR 0033)"
    );
    assert_eq!(
        counts_first.invocations, 1,
        "soroban_invocations_appearances row count — one (contract, tx, ledger) trio"
    );
    assert_eq!(
        counts_first.invocations_amount_sum, 1,
        "SUM(amount) must equal the ingested invocation tree-node count (ADR 0034)"
    );
    assert!(counts_first.contracts >= 1, "contracts row count");
    assert_eq!(counts_first.wasm, 1, "wasm_interface_metadata row count");
    assert_eq!(counts_first.assets, 1, "assets row count");
    assert_eq!(counts_first.nfts, 1, "nfts row count");

    // Task 0160 regression: SAC row must now carry the wrapped classic
    // asset's code + issuer_id + contract_id — previously all three
    // landed NULL / missing because `upsert_assets_classic_like`
    // silently dropped SAC rows lacking code/issuer.
    let sac_identity: (String, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT a.asset_code,
               a.issuer_id,
               a.contract_id
          FROM assets a
          JOIN soroban_contracts sc ON sc.id = a.contract_id
         WHERE sc.contract_id = $1
           AND a.asset_type = $2
        "#,
    )
    .bind(TOKEN_CONTRACT)
    .bind(TokenAssetType::Sac)
    .fetch_one(&pool)
    .await
    .expect("SAC row exists with asset_type = Sac");
    assert_eq!(sac_identity.0, "USDC", "SAC asset_code populated");
    assert!(
        sac_identity.1.is_some(),
        "SAC issuer_id resolved to accounts.id"
    );
    assert!(
        sac_identity.2.is_some(),
        "SAC contract_id resolved to soroban_contracts.id"
    );
    assert_eq!(counts_first.pools, 1, "liquidity_pools row count");
    assert_eq!(
        counts_first.pool_snapshots, 1,
        "liquidity_pool_snapshots row count"
    );
    assert!(
        counts_first.balances_current >= 1,
        "account_balances_current row count"
    );

    // Parser does not yet produce these today.
    assert_eq!(
        counts_first.nft_ownership, 0,
        "nft_ownership expected empty"
    );
    assert_eq!(counts_first.lp_positions, 0, "lp_positions expected empty");

    // ADR 0031 round-trip — operations.type SMALLINT decodes back to the
    // typed enum, and the SQL helper renders the same canonical label as
    // OperationType::as_str(). Closes the Rust ↔ SQL drift gap on every run.
    let ops: Vec<(OperationType, String)> = sqlx::query_as(
        r#"
        SELECT type, op_type_name(type)
          FROM operations
         WHERE ledger_sequence = $1
         ORDER BY application_order
        "#,
    )
    .bind(i64::from(TEST_LEDGER_SEQ))
    .fetch_all(&pool)
    .await
    .expect("fetch operations as typed enum");
    assert_eq!(ops.len(), 2, "two ops inserted by the fixture");
    assert_eq!(ops[0].0, OperationType::Payment);
    assert_eq!(ops[0].1, "PAYMENT");
    assert_eq!(ops[1].0, OperationType::InvokeHostFunction);
    assert_eq!(ops[1].1, "INVOKE_HOST_FUNCTION");

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
        &assets,
        &nfts,
        &nft_events,
        &lp_positions,
        &inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("replay persist_ledger failed");

    let counts_replay = test_counts(&pool).await;
    assert_eq!(counts_replay, counts_first, "replay must be idempotent");
}

/// ADR 0031 — every Rust `#[repr(i16)]` enum variant must agree with the
/// matching `xxx_name(SMALLINT)` SQL helper from migration 0008. Without
/// this guard a new Rust variant would silently render NULL in psql / BI
/// dashboards until a human noticed.
#[tokio::test]
async fn enum_label_helpers_match_rust_as_str() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping enum-helper drift test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping enum-helper drift test");
            return;
        }
    };

    // Per-enum check: bind every VARIANTS element as SMALLINT, fetch the
    // label rendered by the SQL helper, compare to Rust `as_str()`.
    macro_rules! check_all {
        ($pool:expr, $sql_fn:expr, $enum:ty) => {
            for v in <$enum>::VARIANTS {
                let i: i16 = *v as i16;
                let label: Option<String> = sqlx::query_scalar(&format!("SELECT {}($1)", $sql_fn))
                    .bind(i)
                    .fetch_one($pool)
                    .await
                    .unwrap_or_else(|err| panic!("{}({}) query failed: {err}", $sql_fn, i));
                let label = label.unwrap_or_else(|| {
                    panic!(
                        "{}({}) returned NULL — Rust ↔ SQL drift on variant {}",
                        $sql_fn,
                        i,
                        v.as_str()
                    )
                });
                assert_eq!(
                    label,
                    v.as_str(),
                    "{}({}) = {:?}; Rust {}::{:?}.as_str() = {:?}",
                    $sql_fn,
                    i,
                    label,
                    stringify!($enum),
                    v,
                    v.as_str()
                );
            }
        };
    }

    check_all!(&pool, "op_type_name", OperationType);
    check_all!(&pool, "asset_type_name", AssetType);
    check_all!(&pool, "token_asset_type_name", TokenAssetType);
    // ADR 0033: soroban_events.event_type no longer exists; no event_type_name helper.
    check_all!(&pool, "nft_event_type_name", NftEventType);
    check_all!(&pool, "contract_type_name", ContractType);
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
        op_type: OperationType::Payment,
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
        op_type: OperationType::InvokeHostFunction,
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
        event_type: ContractEventType::Contract,
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
        contract_type: ContractType::Token,
        is_sac: true,
        metadata: json!({"name": "TEST"}),
        // Task 0160: match the SAC asset row fixture (make_sac_asset) so
        // integration tests exercise a complete SAC identity end-to-end.
        sac_asset: Some(xdr_parser::types::SacAssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: ISSUER_STRKEY.to_string(),
        }),
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

fn make_sac_asset() -> ExtractedAsset {
    ExtractedAsset {
        asset_type: TokenAssetType::Sac,
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
        "soroban_events_appearances",
        "soroban_invocations_appearances",
        "nft_ownership",
        "liquidity_pool_snapshots",
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
    // accounts, assets, nfts etc need explicit cleanup so repeated runs start
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
    // ADR 0030: assets/nfts/nft_ownership.contract_id is now BIGINT → join via
    // soroban_contracts to filter by StrKey.
    let _ = sqlx::query(
        "DELETE FROM nft_ownership WHERE nft_id IN (
            SELECT n.id FROM nfts n
              JOIN soroban_contracts sc ON sc.id = n.contract_id
             WHERE sc.contract_id = ANY($1)
         )",
    )
    .bind(vec![TOKEN_CONTRACT.to_string(), NFT_CONTRACT.to_string()])
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM nfts WHERE contract_id IN (
            SELECT id FROM soroban_contracts WHERE contract_id = ANY($1)
         )",
    )
    .bind(vec![TOKEN_CONTRACT.to_string(), NFT_CONTRACT.to_string()])
    .execute(pool)
    .await;
    // soroban_events_appearances / invocations / operations / participants
    // cascade via FK on (transaction_id, created_at). Deleting the parent
    // transactions wipes them.
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
    // assets — delete anything referencing our SAC/soroban contract_id to start clean.
    // ADR 0030: assets.contract_id is BIGINT; resolve StrKey → id first.
    let _ = sqlx::query(
        "DELETE FROM assets WHERE contract_id IN (
            SELECT id FROM soroban_contracts WHERE contract_id = ANY($1)
         )",
    )
    .bind(vec![TOKEN_CONTRACT.to_string(), NFT_CONTRACT.to_string()])
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM assets WHERE asset_type IN (1, 2) AND issuer_id IN (SELECT id FROM accounts WHERE account_id = $1)"
    )
    .bind(ISSUER_STRKEY)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM account_balances_current WHERE account_id IN (SELECT id FROM accounts WHERE account_id = ANY($1))")
        .bind(vec![SRC_STRKEY.to_string(), DST_STRKEY.to_string(), ISSUER_STRKEY.to_string()])
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
    events_amount_sum: i64,
    invocations: i64,
    invocations_amount_sum: i64,
    contracts: i64,
    wasm: i64,
    assets: i64,
    nfts: i64,
    nft_ownership: i64,
    pools: i64,
    pool_snapshots: i64,
    lp_positions: i64,
    balances_current: i64,
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
          e AS (SELECT COUNT(*) AS n FROM soroban_events_appearances ev
                   JOIN transactions tx ON tx.id = ev.transaction_id AND tx.created_at = ev.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          es AS (SELECT COALESCE(SUM(ev.amount), 0)::BIGINT AS n
                   FROM soroban_events_appearances ev
                   JOIN transactions tx ON tx.id = ev.transaction_id AND tx.created_at = ev.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          iv AS (SELECT COUNT(*) AS n FROM soroban_invocations_appearances inv
                   JOIN transactions tx ON tx.id = inv.transaction_id AND tx.created_at = inv.created_at
                  WHERE tx.hash = decode($3, 'hex')),
          ivs AS (SELECT COALESCE(SUM(inv.amount), 0)::BIGINT AS n
                    FROM soroban_invocations_appearances inv
                    JOIN transactions tx ON tx.id = inv.transaction_id AND tx.created_at = inv.created_at
                   WHERE tx.hash = decode($3, 'hex')),
          c AS (SELECT COUNT(*) AS n FROM soroban_contracts WHERE contract_id = ANY($4)),
          w AS (SELECT COUNT(*) AS n FROM wasm_interface_metadata WHERE wasm_hash = decode($5, 'hex')),
          -- ADR 0030: assets/nfts.contract_id is BIGINT → join soroban_contracts
          -- to filter by StrKey.
          ast AS (SELECT COUNT(*) AS n FROM assets ast
                   JOIN soroban_contracts sc ON sc.id = ast.contract_id
                  WHERE sc.contract_id = ANY($4)),
          n AS (SELECT COUNT(*) AS n FROM nfts n
                   JOIN soroban_contracts sc ON sc.id = n.contract_id
                  WHERE sc.contract_id = ANY($4)),
          no AS (SELECT COUNT(*) AS n FROM nft_ownership no2
                   JOIN nfts nf ON nf.id = no2.nft_id
                   JOIN soroban_contracts sc ON sc.id = nf.contract_id
                  WHERE sc.contract_id = ANY($4)),
          pl AS (SELECT COUNT(*) AS n FROM liquidity_pools WHERE pool_id = decode($6, 'hex')),
          ps AS (SELECT COUNT(*) AS n FROM liquidity_pool_snapshots WHERE pool_id = decode($6, 'hex')),
          lp AS (SELECT COUNT(*) AS n FROM lp_positions WHERE pool_id = decode($6, 'hex')),
          bc AS (SELECT COUNT(*) AS n FROM account_balances_current abc
                   JOIN accounts aa ON aa.id = abc.account_id
                  WHERE aa.account_id = ANY($2))
        SELECT l.n AS l, a.n AS a, t.n AS t, hi.n AS hi, p.n AS p, o.n AS o,
               e.n AS e, es.n AS es, iv.n AS iv, ivs.n AS ivs, c.n AS c, w.n AS w, ast.n AS ast, n.n AS n,
               no.n AS no, pl.n AS pl, ps.n AS ps, lp.n AS lp, bc.n AS bc
          FROM l, a, t, hi, p, o, e, es, iv, ivs, c, w, ast, n, no, pl, ps, lp, bc
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
        events_amount_sum: row.get("es"),
        invocations: row.get("iv"),
        invocations_amount_sum: row.get("ivs"),
        contracts: row.get("c"),
        wasm: row.get("w"),
        assets: row.get("ast"),
        nfts: row.get("n"),
        nft_ownership: row.get("no"),
        pools: row.get("pl"),
        pool_snapshots: row.get("ps"),
        lp_positions: row.get("lp"),
        balances_current: row.get("bc"),
    }
}

// Touch DateTime<Utc> so the compiler picks up the chrono dep even if all
// usages become conditional later.
#[allow(dead_code)]
fn _touch(_: DateTime<Utc>) {}

// ---------------------------------------------------------------------------
// Task 0153 — mid-stream backfill: stub wasm_interface_metadata when a
// contract deployed in-window references a WASM uploaded before the window.
// ---------------------------------------------------------------------------

const STUB_LEDGER_SEQ: u32 = 90_000_101;
const STUB_LEDGER_SEQ_2: u32 = 90_000_102;
/// 2026-04-21 12:01:40 UTC / 12:03:20 UTC — distinct from the idempotency test.
const STUB_CLOSED_AT: i64 = 1_777_118_500;
const STUB_CLOSED_AT_2: i64 = 1_777_118_600;
const STUB_TX_HASH: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const STUB_TX_HASH_2: &str = "6666666666666666666666666666666666666666666666666666666666666666";
const STUB_LEDGER_HASH: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const STUB_LEDGER_HASH_2: &str = "8888888888888888888888888888888888888888888888888888888888888888";
const STUB_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAASTUB";
const STUB_WASM_HASH: &str = "9999999999999999999999999999999999999999999999999999999999999999";

#[tokio::test]
async fn stub_wasm_unblocks_unknown_hash_and_real_upload_upgrades_it() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping stub-wasm test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping stub-wasm test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_stub_test(&pool).await;

    // --- Ledger 1: deployment references a WASM whose interface is NOT in
    // this ledger (simulating mid-stream backfill where the upload happened
    // before the backfill window).
    let ledger1 = ExtractedLedger {
        sequence: STUB_LEDGER_SEQ,
        hash: STUB_LEDGER_HASH.to_string(),
        closed_at: STUB_CLOSED_AT,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx1 = ExtractedTransaction {
        hash: STUB_TX_HASH.to_string(),
        ledger_sequence: STUB_LEDGER_SEQ,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: STUB_CLOSED_AT,
        parse_error: false,
    };
    let dep = ExtractedContractDeployment {
        contract_id: STUB_CONTRACT.to_string(),
        wasm_hash: Some(STUB_WASM_HASH.to_string()),
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: STUB_LEDGER_SEQ,
        contract_type: ContractType::Other,
        is_sac: false,
        metadata: json!({}),
        sac_asset: None,
    };

    let empty_operations: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_interfaces: Vec<ExtractedContractInterface> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_assets: Vec<ExtractedAsset> = Vec::new();
    let no_nfts: Vec<ExtractedNft> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let classification_cache = ClassificationCache::new();

    persist_ledger(
        &pool,
        &ledger1,
        &[tx1],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &no_interfaces,
        &[dep],
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("persist_ledger with unknown wasm_hash must succeed (stub path)");

    // Stub row exists, metadata is empty JSON, soroban_contracts carries the FK.
    let stub_metadata: Value = sqlx::query_scalar(
        "SELECT metadata FROM wasm_interface_metadata WHERE wasm_hash = decode($1, 'hex')",
    )
    .bind(STUB_WASM_HASH)
    .fetch_one(&pool)
    .await
    .expect("stub wasm_interface_metadata row must exist");
    assert_eq!(stub_metadata, json!({}), "stub metadata is empty JSON");

    let contract_wasm: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT wasm_hash FROM soroban_contracts WHERE contract_id = $1")
            .bind(STUB_CONTRACT)
            .fetch_one(&pool)
            .await
            .expect("soroban_contracts row inserted under stub FK");
    let expected_bytes = hex::decode(STUB_WASM_HASH).expect("decode STUB_WASM_HASH");
    assert_eq!(
        contract_wasm,
        Some(expected_bytes),
        "soroban_contracts.wasm_hash points at the stubbed WASM"
    );

    // --- Ledger 2: the real WASM upload is observed (contract_interface
    // carries the hash). Stub metadata must be overwritten in place.
    let ledger2 = ExtractedLedger {
        sequence: STUB_LEDGER_SEQ_2,
        hash: STUB_LEDGER_HASH_2.to_string(),
        closed_at: STUB_CLOSED_AT_2,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx2 = ExtractedTransaction {
        hash: STUB_TX_HASH_2.to_string(),
        ledger_sequence: STUB_LEDGER_SEQ_2,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: STUB_CLOSED_AT_2,
        parse_error: false,
    };
    let iface = ExtractedContractInterface {
        wasm_hash: STUB_WASM_HASH.to_string(),
        functions: Vec::new(),
        wasm_byte_len: 512,
    };
    let no_deployments: Vec<ExtractedContractDeployment> = Vec::new();

    persist_ledger(
        &pool,
        &ledger2,
        &[tx2],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &[iface],
        &no_deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("follow-up persist_ledger with the real WASM upload must succeed");

    let upgraded_metadata: Value = sqlx::query_scalar(
        "SELECT metadata FROM wasm_interface_metadata WHERE wasm_hash = decode($1, 'hex')",
    )
    .bind(STUB_WASM_HASH)
    .fetch_one(&pool)
    .await
    .expect("wasm_interface_metadata row still present");
    assert_eq!(
        upgraded_metadata,
        json!({"functions": [], "wasm_byte_len": 512}),
        "stub metadata must be upgraded in place to the real ABI"
    );

    clean_stub_test(&pool).await;
}

// ---------------------------------------------------------------------------
// Task 0118 Phase 2 — fungible-transfer NFT filter
// ---------------------------------------------------------------------------

const FILTER_LEDGER_SEQ: u32 = 90_000_201;
/// 2026-04-21 12:10:00 UTC — distinct from the other tests' ledger windows.
const FILTER_CLOSED_AT: i64 = 1_777_119_000;
const FILTER_TX_HASH: &str = "aaaa111111111111111111111111111111111111111111111111111111111111";
const FILTER_LEDGER_HASH: &str = "bbbb111111111111111111111111111111111111111111111111111111111111";
const NFT_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFLTRNFT";
const FUN_ID: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFLTRFUN";
const NFT_WASM_HASH: &str = "cccc111111111111111111111111111111111111111111111111111111111111";
const FUN_WASM_HASH: &str = "dddd111111111111111111111111111111111111111111111111111111111111";

/// End-to-end check of the task 0118 Phase 2 NFT insert filter.
///
/// Both contracts receive an NFT-candidate row with an `i128`-shaped
/// token id; only the contract classified as `Nft` should land in the
/// `nfts` table after persist. The `Fungible` contract's row must be
/// dropped by the filter — that is exactly the USDC-in-`nfts`
/// regression from audit finding F9.
#[tokio::test]
async fn nft_filter_drops_fungible_classified_contract() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping NFT filter test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping NFT filter test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_filter_test(&pool).await;

    let ledger = ExtractedLedger {
        sequence: FILTER_LEDGER_SEQ,
        hash: FILTER_LEDGER_HASH.to_string(),
        closed_at: FILTER_CLOSED_AT,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx = ExtractedTransaction {
        hash: FILTER_TX_HASH.to_string(),
        ledger_sequence: FILTER_LEDGER_SEQ,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: FILTER_CLOSED_AT,
        parse_error: false,
    };

    let interfaces = vec![
        iface_with(NFT_WASM_HASH, &["owner_of", "transfer"]),
        iface_with(FUN_WASM_HASH, &["decimals", "allowance", "transfer"]),
    ];
    let deployments = vec![
        deploy_with(NFT_ID, NFT_WASM_HASH),
        deploy_with(FUN_ID, FUN_WASM_HASH),
    ];
    let nfts = vec![
        nft_row(NFT_ID, "1"),
        nft_row(FUN_ID, "2"), // fungible-transfer false-positive
    ];

    let empty_operations: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_assets: Vec<ExtractedAsset> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let classification_cache = ClassificationCache::new();

    persist_ledger(
        &pool,
        &ledger,
        &[tx],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &interfaces,
        &deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("persist_ledger must succeed under the NFT filter path");

    // ── contract_type column was written per classification ──
    let nft_ty: Option<i16> =
        sqlx::query_scalar("SELECT contract_type FROM soroban_contracts WHERE contract_id = $1")
            .bind(NFT_ID)
            .fetch_one(&pool)
            .await
            .expect("NFT contract row must exist");
    let fun_ty: Option<i16> =
        sqlx::query_scalar("SELECT contract_type FROM soroban_contracts WHERE contract_id = $1")
            .bind(FUN_ID)
            .fetch_one(&pool)
            .await
            .expect("fungible contract row must exist");
    assert_eq!(
        nft_ty.and_then(|v| ContractType::try_from(v).ok()),
        Some(ContractType::Nft),
        "NFT contract_type persisted",
    );
    assert_eq!(
        fun_ty.and_then(|v| ContractType::try_from(v).ok()),
        Some(ContractType::Fungible),
        "fungible contract_type persisted",
    );

    // ── nfts filter verdict: NFT row kept, fungible row dropped ──
    let nft_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nfts n
           JOIN soroban_contracts sc ON sc.id = n.contract_id
          WHERE sc.contract_id = $1",
    )
    .bind(NFT_ID)
    .fetch_one(&pool)
    .await
    .expect("count nfts for NFT contract");
    let fun_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nfts n
           JOIN soroban_contracts sc ON sc.id = n.contract_id
          WHERE sc.contract_id = $1",
    )
    .bind(FUN_ID)
    .fetch_one(&pool)
    .await
    .expect("count nfts for fungible contract");
    assert_eq!(nft_count, 1, "NFT contract row survives the filter");
    assert_eq!(
        fun_count, 0,
        "fungible-classified contract row dropped at filter",
    );

    // ── cache hydrated for both deployments (definitive verdicts only) ──
    assert_eq!(
        classification_cache.get(NFT_ID),
        Some(ContractType::Nft),
        "per-worker cache holds the NFT verdict",
    );
    assert_eq!(
        classification_cache.get(FUN_ID),
        Some(ContractType::Fungible),
        "per-worker cache holds the fungible verdict",
    );

    clean_filter_test(&pool).await;
}

fn iface_with(wasm_hash: &str, fn_names: &[&str]) -> ExtractedContractInterface {
    ExtractedContractInterface {
        wasm_hash: wasm_hash.to_string(),
        functions: fn_names
            .iter()
            .map(|n| ContractFunction {
                name: (*n).to_string(),
                doc: String::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            })
            .collect(),
        wasm_byte_len: 1024,
    }
}

fn deploy_with(contract_id: &str, wasm_hash: &str) -> ExtractedContractDeployment {
    ExtractedContractDeployment {
        contract_id: contract_id.to_string(),
        wasm_hash: Some(wasm_hash.to_string()),
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: FILTER_LEDGER_SEQ,
        contract_type: ContractType::Other, // parser default; staging overrides
        is_sac: false,
        metadata: json!({}),
        sac_asset: None,
    }
}

fn nft_row(contract_id: &str, token_id: &str) -> ExtractedNft {
    ExtractedNft {
        contract_id: contract_id.to_string(),
        token_id: token_id.to_string(),
        collection_name: None,
        owner_account: Some(DST_STRKEY.to_string()),
        name: None,
        media_url: None,
        metadata: None,
        minted_at_ledger: Some(FILTER_LEDGER_SEQ),
        last_seen_ledger: FILTER_LEDGER_SEQ,
        created_at: FILTER_CLOSED_AT,
    }
}

async fn clean_filter_test(pool: &PgPool) {
    let contracts = vec![NFT_ID.to_string(), FUN_ID.to_string()];
    // nfts → soroban_contracts join is the only ref path into nfts for the
    // filter test fixture. Drop children first, then the contracts, then
    // the wasm rows behind them.
    let _ = sqlx::query(
        "DELETE FROM nfts WHERE contract_id IN (
            SELECT id FROM soroban_contracts WHERE contract_id = ANY($1)
         )",
    )
    .bind(&contracts)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM transactions WHERE hash = decode($1, 'hex')")
        .bind(FILTER_TX_HASH)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM transaction_hash_index WHERE hash = decode($1, 'hex')")
        .bind(FILTER_TX_HASH)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
        .bind(i64::from(FILTER_LEDGER_SEQ))
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = ANY($1)")
        .bind(&contracts)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = ANY($1::BYTEA[])")
        .bind(vec![
            hex::decode(NFT_WASM_HASH).unwrap(),
            hex::decode(FUN_WASM_HASH).unwrap(),
        ])
        .execute(pool)
        .await;
}

// ---------------------------------------------------------------------------
// Task 0120 — Soroban-native token detection + late-WASM bridge
// ---------------------------------------------------------------------------

const TK_LEDGER_SEQ_1: u32 = 90_000_301;
const TK_LEDGER_SEQ_2: u32 = 90_000_302;
/// 2026-04-22 12:20:00 UTC
const TK_CLOSED_AT_1: i64 = 1_777_205_400;
const TK_CLOSED_AT_2: i64 = TK_CLOSED_AT_1 + 6;
const TK_LEDGER_HASH_1: &str = "eeee111111111111111111111111111111111111111111111111111111111111";
const TK_LEDGER_HASH_2: &str = "eeee222222222222222222222222222222222222222222222222222222222222";
const TK_TX_HASH_1: &str = "eeee333333333333333333333333333333333333333333333333333333333333";
const TK_TX_HASH_2: &str = "eeee444444444444444444444444444444444444444444444444444444444444";
const TK_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAATKNSOR";
const TK_WASM_HASH: &str = "eeee555555555555555555555555555555555555555555555555555555555555";

/// End-to-end check of task 0120's same-ledger detection path.
///
/// A WASM deployment classified as `Fungible` (SEP-0041 surface) lands in
/// the `assets` table with `asset_type = Soroban` and `contract_id` set
/// to the surrogate bigint id of the deployed contract.
#[tokio::test]
async fn soroban_fungible_contract_produces_assets_row() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0120 same-ledger test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0120 same-ledger test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_tk_test(&pool).await;

    let ledger = ExtractedLedger {
        sequence: TK_LEDGER_SEQ_1,
        hash: TK_LEDGER_HASH_1.to_string(),
        closed_at: TK_CLOSED_AT_1,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx = ExtractedTransaction {
        hash: TK_TX_HASH_1.to_string(),
        ledger_sequence: TK_LEDGER_SEQ_1,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: TK_CLOSED_AT_1,
        parse_error: false,
    };

    // SEP-0041 surface (decimals is a fungible discriminator).
    let interfaces = vec![iface_with(
        TK_WASM_HASH,
        &["transfer", "balance", "decimals", "name", "symbol"],
    )];
    let deployments = vec![ExtractedContractDeployment {
        contract_id: TK_CONTRACT.to_string(),
        wasm_hash: Some(TK_WASM_HASH.to_string()),
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: TK_LEDGER_SEQ_1,
        contract_type: ContractType::Other, // staging overrides via classifier
        is_sac: false,
        metadata: json!({}),
        sac_asset: None,
    }];
    // Drive the real parser → persist wiring end-to-end so a regression in
    // detect_assets signature/behaviour fails this test, not just an
    // isolated unit test.
    let assets = xdr_parser::detect_assets(&deployments, &interfaces);
    assert_eq!(
        assets.len(),
        1,
        "parser must emit exactly one Soroban asset for this deploy"
    );
    assert_eq!(assets[0].asset_type, TokenAssetType::Soroban);

    let empty_operations: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_nfts: Vec<ExtractedNft> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let classification_cache = ClassificationCache::new();

    persist_ledger(
        &pool,
        &ledger,
        &[tx],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &interfaces,
        &deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("persist_ledger for 0120 same-ledger path must succeed");

    // Contract row classified as Fungible.
    let fun_ty: Option<i16> =
        sqlx::query_scalar("SELECT contract_type FROM soroban_contracts WHERE contract_id = $1")
            .bind(TK_CONTRACT)
            .fetch_one(&pool)
            .await
            .expect("soroban_contracts row exists");
    assert_eq!(
        fun_ty.and_then(|v| ContractType::try_from(v).ok()),
        Some(ContractType::Fungible),
        "contract_type must be Fungible"
    );

    // Exactly one Soroban asset row for this contract.
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM assets t
             JOIN soroban_contracts sc ON sc.id = t.contract_id
            WHERE sc.contract_id = $1
              AND t.asset_type = $2"#,
    )
    .bind(TK_CONTRACT)
    .bind(TokenAssetType::Soroban)
    .fetch_one(&pool)
    .await
    .expect("assets count query succeeds");
    assert_eq!(
        count, 1,
        "exactly one Soroban assets row per Fungible contract"
    );

    clean_tk_test(&pool).await;
}

/// End-to-end check of task 0120's late-WASM bridge path.
///
/// Two-ledger pattern: contract deploys in L1 referencing a wasm_hash
/// whose interface is not in L1. `detect_assets` skips it. `stub_wasm`
/// path leaves `soroban_contracts.contract_type = Other`. In L2 the real
/// WASM upload arrives with SEP-0041 discriminators;
/// `reclassify_contracts_from_wasm` promotes contract_type to Fungible,
/// and `insert_assets_from_reclassified_contracts` backfills the missing
/// assets row.
#[tokio::test]
async fn late_wasm_upload_backfills_assets_row() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0120 late-WASM test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0120 late-WASM test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_tk_test(&pool).await;

    // ── L1: deploy without the WASM upload. Parser emits no asset row. ──
    let ledger1 = ExtractedLedger {
        sequence: TK_LEDGER_SEQ_1,
        hash: TK_LEDGER_HASH_1.to_string(),
        closed_at: TK_CLOSED_AT_1,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx1 = ExtractedTransaction {
        hash: TK_TX_HASH_1.to_string(),
        ledger_sequence: TK_LEDGER_SEQ_1,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: TK_CLOSED_AT_1,
        parse_error: false,
    };
    let deployments = vec![ExtractedContractDeployment {
        contract_id: TK_CONTRACT.to_string(),
        wasm_hash: Some(TK_WASM_HASH.to_string()),
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: TK_LEDGER_SEQ_1,
        contract_type: ContractType::Other,
        is_sac: false,
        metadata: json!({}),
        sac_asset: None,
    }];

    let empty_operations: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_interfaces: Vec<ExtractedContractInterface> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_assets: Vec<ExtractedAsset> = Vec::new();
    let no_nfts: Vec<ExtractedNft> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let classification_cache = ClassificationCache::new();

    persist_ledger(
        &pool,
        &ledger1,
        &[tx1],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &no_interfaces,
        &deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("L1 persist_ledger (no-WASM deploy) must succeed");

    // After L1: contract exists with contract_type = Other, no assets row.
    let count_before: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM assets t
             JOIN soroban_contracts sc ON sc.id = t.contract_id
            WHERE sc.contract_id = $1
              AND t.asset_type = $2"#,
    )
    .bind(TK_CONTRACT)
    .bind(TokenAssetType::Soroban)
    .fetch_one(&pool)
    .await
    .expect("assets count succeeds");
    assert_eq!(count_before, 0, "no assets row yet (WASM not observed)");

    // ── L2: WASM upload arrives. Interface has SEP-0041 surface.
    //   Reclassify promotes Other → Fungible; bridge inserts assets row.
    let ledger2 = ExtractedLedger {
        sequence: TK_LEDGER_SEQ_2,
        hash: TK_LEDGER_HASH_2.to_string(),
        closed_at: TK_CLOSED_AT_2,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx2 = ExtractedTransaction {
        hash: TK_TX_HASH_2.to_string(),
        ledger_sequence: TK_LEDGER_SEQ_2,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: TK_CLOSED_AT_2,
        parse_error: false,
    };
    let interfaces = vec![iface_with(
        TK_WASM_HASH,
        &["transfer", "balance", "decimals", "name", "symbol"],
    )];
    let no_deployments: Vec<ExtractedContractDeployment> = Vec::new();

    persist_ledger(
        &pool,
        &ledger2,
        &[tx2],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &interfaces,
        &no_deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache,
    )
    .await
    .expect("L2 persist_ledger (late-WASM upload) must succeed");

    // After L2: contract promoted to Fungible, assets row inserted.
    let fun_ty: Option<i16> =
        sqlx::query_scalar("SELECT contract_type FROM soroban_contracts WHERE contract_id = $1")
            .bind(TK_CONTRACT)
            .fetch_one(&pool)
            .await
            .expect("soroban_contracts row exists");
    assert_eq!(
        fun_ty.and_then(|v| ContractType::try_from(v).ok()),
        Some(ContractType::Fungible),
        "contract_type promoted Other → Fungible"
    );

    let count_after: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM assets t
             JOIN soroban_contracts sc ON sc.id = t.contract_id
            WHERE sc.contract_id = $1
              AND t.asset_type = $2"#,
    )
    .bind(TK_CONTRACT)
    .bind(TokenAssetType::Soroban)
    .fetch_one(&pool)
    .await
    .expect("assets count succeeds");
    assert_eq!(count_after, 1, "bridge inserted Soroban assets row");

    // Re-run the same ledger (replay) — must be idempotent, still exactly one row.
    let classification_cache2 = ClassificationCache::new();
    persist_ledger(
        &pool,
        &ledger2,
        &[ExtractedTransaction {
            hash: TK_TX_HASH_2.to_string(),
            ledger_sequence: TK_LEDGER_SEQ_2,
            source_account: SRC_STRKEY.to_string(),
            fee_charged: 100,
            successful: true,
            result_code: "txSuccess".to_string(),
            envelope_xdr: "AAAAAA...".to_string(),
            result_xdr: "AAAAAA...".to_string(),
            result_meta_xdr: None,
            operation_tree: None,
            memo_type: None,
            memo: None,
            created_at: TK_CLOSED_AT_2,
            parse_error: false,
        }],
        &empty_operations,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &interfaces,
        &no_deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &no_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &classification_cache2,
    )
    .await
    .expect("L2 replay must be idempotent");

    let count_replay: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM assets t
             JOIN soroban_contracts sc ON sc.id = t.contract_id
            WHERE sc.contract_id = $1
              AND t.asset_type = $2"#,
    )
    .bind(TK_CONTRACT)
    .bind(TokenAssetType::Soroban)
    .fetch_one(&pool)
    .await
    .expect("assets count succeeds");
    assert_eq!(count_replay, 1, "replay does not duplicate assets row");

    clean_tk_test(&pool).await;
}

async fn clean_tk_test(pool: &PgPool) {
    let tx_hashes = vec![
        hex::decode(TK_TX_HASH_1).unwrap(),
        hex::decode(TK_TX_HASH_2).unwrap(),
    ];
    let _ = sqlx::query(
        "DELETE FROM assets
          WHERE contract_id IN (SELECT id FROM soroban_contracts WHERE contract_id = $1)",
    )
    .bind(TK_CONTRACT)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM transactions WHERE hash = ANY($1)")
        .bind(&tx_hashes)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM transaction_hash_index WHERE hash = ANY($1)")
        .bind(&tx_hashes)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM ledgers WHERE sequence = ANY($1)")
        .bind(vec![i64::from(TK_LEDGER_SEQ_1), i64::from(TK_LEDGER_SEQ_2)])
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
        .bind(TK_CONTRACT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = decode($1, 'hex')")
        .bind(TK_WASM_HASH)
        .execute(pool)
        .await;
}

async fn clean_stub_test(pool: &PgPool) {
    // Wipe leaves first so the wasm_interface_metadata delete isn't blocked
    // by the soroban_contracts FK.
    let _ = sqlx::query("DELETE FROM transactions WHERE hash = ANY($1)")
        .bind(
            vec![STUB_TX_HASH, STUB_TX_HASH_2]
                .into_iter()
                .map(|h| hex::decode(h).unwrap())
                .collect::<Vec<_>>(),
        )
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM transaction_hash_index WHERE hash = ANY($1)")
        .bind(
            vec![STUB_TX_HASH, STUB_TX_HASH_2]
                .into_iter()
                .map(|h| hex::decode(h).unwrap())
                .collect::<Vec<_>>(),
        )
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM ledgers WHERE sequence = ANY($1)")
        .bind(vec![
            i64::from(STUB_LEDGER_SEQ),
            i64::from(STUB_LEDGER_SEQ_2),
        ])
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
        .bind(STUB_CONTRACT)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = decode($1, 'hex')")
        .bind(STUB_WASM_HASH)
        .execute(pool)
        .await;
}

// ---------------------------------------------------------------------------
// Task 0160 — SAC underlying asset identity extraction
// ---------------------------------------------------------------------------

const SAC160_LEDGER_SEQ: u32 = 90_000_401;
const SAC160_CLOSED_AT: i64 = 1_777_212_000;
const SAC160_LEDGER_HASH: &str = "ddd0000000000000000000000000000000000000000000000000000000000160";
const SAC160_TX_HASH: &str = "ddd0160000000000000000000000000000000000000000000000000000000001";
const SAC160_XLM_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAXLMSAC";
const SAC160_CREDIT_CONTRACT: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAUSDSAC";

/// Native XLM-SAC deployment (Asset::Native preimage) → `assets` row lands
/// with NULL `asset_code` + NULL `issuer_id` + populated `contract_id`.
/// Verifies the 0160 schema loosening (ck_assets_identity allows this
/// shape for asset_type=Sac) end-to-end against a real Postgres.
#[tokio::test]
async fn xlm_sac_deployment_lands_with_null_identity() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0160 XLM-SAC test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0160 XLM-SAC test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_sac160_test(&pool).await;

    let ledger = ExtractedLedger {
        sequence: SAC160_LEDGER_SEQ,
        hash: SAC160_LEDGER_HASH.to_string(),
        closed_at: SAC160_CLOSED_AT,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx = ExtractedTransaction {
        hash: SAC160_TX_HASH.to_string(),
        ledger_sequence: SAC160_LEDGER_SEQ,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: SAC160_CLOSED_AT,
        parse_error: false,
    };
    let deployments = vec![ExtractedContractDeployment {
        contract_id: SAC160_XLM_CONTRACT.to_string(),
        wasm_hash: None,
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: SAC160_LEDGER_SEQ,
        contract_type: ContractType::Token,
        is_sac: true,
        metadata: json!({}),
        sac_asset: Some(xdr_parser::types::SacAssetIdentity::Native),
    }];
    let assets = vec![ExtractedAsset {
        asset_type: TokenAssetType::Sac,
        asset_code: None,
        issuer_address: None,
        contract_id: Some(SAC160_XLM_CONTRACT.to_string()),
        name: None,
        total_supply: None,
        holder_count: None,
    }];

    let empty_ops: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_interfaces: Vec<ExtractedContractInterface> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_nfts: Vec<ExtractedNft> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();
    let cache = ClassificationCache::new();

    persist_ledger(
        &pool,
        &ledger,
        &[tx],
        &empty_ops,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &no_interfaces,
        &deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &cache,
    )
    .await
    .expect("XLM-SAC persist_ledger must succeed");

    let row: (Option<String>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT a.asset_code, a.issuer_id
          FROM assets a
          JOIN soroban_contracts sc ON sc.id = a.contract_id
         WHERE sc.contract_id = $1
           AND a.asset_type = $2
        "#,
    )
    .bind(SAC160_XLM_CONTRACT)
    .bind(TokenAssetType::Sac)
    .fetch_one(&pool)
    .await
    .expect("XLM-SAC row must land with NULL identity + contract_id FK");
    assert!(
        row.0.is_none(),
        "native XLM-SAC must persist with NULL asset_code"
    );
    assert!(
        row.1.is_none(),
        "native XLM-SAC must persist with NULL issuer_id"
    );

    clean_sac160_test(&pool).await;
}

/// GREATEST promotion — a ClassicCredit(1) write arriving after a SAC(2)
/// write for the same (asset_code, issuer) MUST NOT downgrade asset_type
/// back to 1. Parallel-backfill safety: order-independent final state.
#[tokio::test]
async fn classic_to_sac_greatest_promotion_is_monotonic() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset — skipping 0160 GREATEST test");
        return;
    };
    let pool = match PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("DATABASE_URL unreachable ({err}) — skipping 0160 GREATEST test");
            return;
        }
    };

    ensure_default_partitions(&pool).await;
    clean_sac160_test(&pool).await;

    let ledger = ExtractedLedger {
        sequence: SAC160_LEDGER_SEQ,
        hash: SAC160_LEDGER_HASH.to_string(),
        closed_at: SAC160_CLOSED_AT,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    };
    let tx = ExtractedTransaction {
        hash: SAC160_TX_HASH.to_string(),
        ledger_sequence: SAC160_LEDGER_SEQ,
        source_account: SRC_STRKEY.to_string(),
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".to_string(),
        envelope_xdr: "AAAAAA...".to_string(),
        result_xdr: "AAAAAA...".to_string(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: SAC160_CLOSED_AT,
        parse_error: false,
    };

    let empty_ops: Vec<(String, Vec<ExtractedOperation>)> = Vec::new();
    let empty_events: Vec<(String, Vec<ExtractedEvent>)> = Vec::new();
    let empty_invocations: Vec<(String, Vec<ExtractedInvocation>)> = Vec::new();
    let empty_trees: Vec<(String, serde_json::Value)> = Vec::new();
    let no_interfaces: Vec<ExtractedContractInterface> = Vec::new();
    let no_account_states: Vec<ExtractedAccountState> = Vec::new();
    let no_pools: Vec<ExtractedLiquidityPool> = Vec::new();
    let no_snapshots: Vec<ExtractedLiquidityPoolSnapshot> = Vec::new();
    let no_nfts: Vec<ExtractedNft> = Vec::new();
    let no_nft_events: Vec<ExtractedNftEvent> = Vec::new();
    let no_lp_positions: Vec<ExtractedLpPosition> = Vec::new();
    let no_inner_tx_hashes: HashMap<String, Option<String>> = HashMap::new();

    // ---- Phase 1: SAC(type=2) lands first with a populated contract_id.
    let sac_deployments = vec![ExtractedContractDeployment {
        contract_id: SAC160_CREDIT_CONTRACT.to_string(),
        wasm_hash: None,
        deployer_account: Some(SRC_STRKEY.to_string()),
        deployed_at_ledger: SAC160_LEDGER_SEQ,
        contract_type: ContractType::Token,
        is_sac: true,
        metadata: json!({}),
        sac_asset: Some(xdr_parser::types::SacAssetIdentity::Credit {
            code: "USDC".to_string(),
            issuer: ISSUER_STRKEY.to_string(),
        }),
    }];
    let sac_assets = vec![ExtractedAsset {
        asset_type: TokenAssetType::Sac,
        asset_code: Some("USDC".to_string()),
        issuer_address: Some(ISSUER_STRKEY.to_string()),
        contract_id: Some(SAC160_CREDIT_CONTRACT.to_string()),
        name: None,
        total_supply: None,
        holder_count: None,
    }];
    let cache = ClassificationCache::new();
    persist_ledger(
        &pool,
        &ledger,
        std::slice::from_ref(&tx),
        &empty_ops,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &no_interfaces,
        &sac_deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &sac_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &cache,
    )
    .await
    .expect("Phase 1 (SAC first) must succeed");

    // ---- Phase 2: ClassicCredit(type=1) arrives second for the same
    //      (code, issuer). Would-be downgrade blocked by GREATEST.
    //      Replay the same ledger — idempotent write shape.
    let classic_assets = vec![ExtractedAsset {
        asset_type: TokenAssetType::ClassicCredit,
        asset_code: Some("USDC".to_string()),
        issuer_address: Some(ISSUER_STRKEY.to_string()),
        contract_id: None,
        name: None,
        total_supply: None,
        holder_count: None,
    }];
    let no_deployments: Vec<ExtractedContractDeployment> = Vec::new();
    let cache2 = ClassificationCache::new();
    persist_ledger(
        &pool,
        &ledger,
        &[tx],
        &empty_ops,
        &empty_events,
        &empty_invocations,
        &empty_trees,
        &no_interfaces,
        &no_deployments,
        &no_account_states,
        &no_pools,
        &no_snapshots,
        &classic_assets,
        &no_nfts,
        &no_nft_events,
        &no_lp_positions,
        &no_inner_tx_hashes,
        &cache2,
    )
    .await
    .expect("Phase 2 (classic second) must succeed — ck_assets_identity holds");

    // Final row — asset_type stayed Sac(2), contract_id preserved.
    let final_type: i16 = sqlx::query_scalar(
        r#"
        SELECT a.asset_type
          FROM assets a
          JOIN accounts acc ON acc.id = a.issuer_id
         WHERE a.asset_code = $1 AND acc.account_id = $2
        "#,
    )
    .bind("USDC")
    .bind(ISSUER_STRKEY)
    .fetch_one(&pool)
    .await
    .expect("classic/SAC row exists post order-swap");
    assert_eq!(
        final_type,
        TokenAssetType::Sac as i16,
        "GREATEST pinned asset_type at Sac(2) — no downgrade"
    );

    let contract_id_after: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT a.contract_id
          FROM assets a
          JOIN accounts acc ON acc.id = a.issuer_id
         WHERE a.asset_code = $1 AND acc.account_id = $2
        "#,
    )
    .bind("USDC")
    .bind(ISSUER_STRKEY)
    .fetch_one(&pool)
    .await
    .expect("fetch contract_id");
    assert!(
        contract_id_after.is_some(),
        "contract_id preserved (COALESCE kept SAC's value through the classic write)"
    );

    clean_sac160_test(&pool).await;
}

async fn clean_sac160_test(pool: &PgPool) {
    let _ = sqlx::query(
        "DELETE FROM assets
          WHERE contract_id IN (
                 SELECT id FROM soroban_contracts WHERE contract_id = ANY($1))",
    )
    .bind(vec![
        SAC160_XLM_CONTRACT.to_string(),
        SAC160_CREDIT_CONTRACT.to_string(),
    ])
    .execute(pool)
    .await;
    // Classic/SAC share (code, issuer) unique — also clean by issuer.
    let _ = sqlx::query(
        "DELETE FROM assets
          WHERE asset_type IN (1, 2)
            AND issuer_id IN (SELECT id FROM accounts WHERE account_id = $1)
            AND asset_code = 'USDC'",
    )
    .bind(ISSUER_STRKEY)
    .execute(pool)
    .await;
    let tx_hash = hex::decode(SAC160_TX_HASH).unwrap();
    let _ = sqlx::query("DELETE FROM transactions WHERE hash = $1")
        .bind(&tx_hash)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM transaction_hash_index WHERE hash = $1")
        .bind(&tx_hash)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM ledgers WHERE sequence = $1")
        .bind(i64::from(SAC160_LEDGER_SEQ))
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = ANY($1)")
        .bind(vec![
            SAC160_XLM_CONTRACT.to_string(),
            SAC160_CREDIT_CONTRACT.to_string(),
        ])
        .execute(pool)
        .await;
}
