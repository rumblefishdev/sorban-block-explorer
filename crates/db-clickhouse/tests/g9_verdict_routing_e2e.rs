//! Task 0392 regression — G9 cross-ledger verdict routing end-to-end.
//!
//! On prod the G9 lookup (`soroban_contracts` verdict read-back for contracts
//! that emit NFT rows/events but were deployed in an earlier ledger) failed on
//! EVERY row-returning call for 2+ weeks: `ContractVerdictRow.contract_type`
//! was a bare `i16` against the `Nullable(Int16)` column, and clickhouse 0.15
//! RBWNAT validation rejects that — the fail-open contract then routed every
//! event to the `*_pending` quarantine (~6.5k rows/day, 91% fungible
//! false-positives). A unit test cannot catch this class: the mismatch only
//! exists on the wire against a real server. Hence e2e.
//!
//! Asserts the full routing contract of `route_for` via the production persist
//! wrapper:
//!   * `Fungible`-verdict contract → events/rows DROPPED (neither hot nor
//!     pending),
//!   * `Nft`-verdict contract → events/rows land HOT (`nfts` /
//!     `nft_ownership`),
//!   * unclassified contract (no verdict row) → quarantine (`nfts_pending` /
//!     `nft_ownership_pending`) — the fail-open path must stay intact.
//!
//! Gated on `CLICKHOUSE_URL` (skips cleanly when unset — same pattern as
//! `persist_e2e.rs`). Run locally:
//!
//!   docker compose up -d clickhouse
//!   CLICKHOUSE_URL=http://localhost:8123 \
//!       cargo test -p db-clickhouse --test g9_verdict_routing_e2e

use db_clickhouse::persist::{ClassificationCache, ids, persist_ledger_clickhouse};
use db_clickhouse::{Config, apply_init_sql, client};
use domain::{ContractEventType, NftEventType};
use xdr_parser::types::{
    EventSource, ExtractedEvent, ExtractedLedger, ExtractedNft, ExtractedNftEvent,
    ExtractedTransaction,
};

/// Out-of-band sentinel — distinct from `smoke.rs` (99_999_001) and
/// `persist_e2e.rs` (99_999_002).
const E2E_LEDGER: u32 = 99_999_003;

/// Valid StrKey shapes (`C`/`G` + 55 chars) so surrogate-id derivation works.
fn contract(tag: char) -> String {
    "C".to_string() + &tag.to_string().repeat(55)
}

fn owner_account() -> String {
    "G".to_string() + &"B".repeat(55)
}

fn tx_hash() -> String {
    let mut bytes = vec![0u8; 32];
    bytes[31] = 0x92;
    hex::encode(&bytes)
}

fn fixture_ledger() -> ExtractedLedger {
    ExtractedLedger {
        sequence: E2E_LEDGER,
        hash: "cd".repeat(32),
        closed_at: 1_700_000_000,
        protocol_version: 22,
        transaction_count: 1,
        base_fee: 100,
    }
}

fn fixture_tx() -> ExtractedTransaction {
    ExtractedTransaction {
        hash: tx_hash(),
        inner_tx_hash: None,
        ledger_sequence: E2E_LEDGER,
        source_account: owner_account(),
        fee_source: None,
        fee_charged: 100,
        successful: true,
        result_code: "txSuccess".into(),
        envelope_xdr: String::new(),
        result_xdr: String::new(),
        result_meta_xdr: None,
        operation_tree: None,
        memo_type: None,
        memo: None,
        created_at: 1_700_000_000,
        parse_error: false,
        ledger_deltas: vec![],
    }
}

fn fixture_nft(contract_id: &str, token: &str) -> ExtractedNft {
    ExtractedNft {
        contract_id: contract_id.to_string(),
        token_id: token.to_string(),
        collection_name: None,
        owner_account: Some(owner_account()),
        name: None,
        media_url: None,
        minted_at_ledger: Some(E2E_LEDGER),
        last_seen_ledger: E2E_LEDGER,
        created_at: 1_700_000_000,
    }
}

/// 32×`0xEE`, base64 — the post-upgrade WASM hash for the 0320 scenario.
const NEW_WASM_B64: &str = "7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u4=";

/// Synthetic host-emitted `executable_update` SYSTEM event (task 0320 path).
/// Topic shape mirrors mainnet: `[Symbol("executable_update"), <old>, <new>]`
/// with `<new>` = `vec[Symbol("Wasm"), Bytes(hash)]`.
fn fixture_upgrade_event(contract_id: &str) -> ExtractedEvent {
    ExtractedEvent {
        transaction_hash: tx_hash(),
        event_type: ContractEventType::System,
        source: EventSource::TxLevel,
        contract_id: Some(contract_id.to_string()),
        topics: serde_json::json!([
            {"type": "symbol", "value": "executable_update"},
            {"type": "vec", "value": [{"type": "symbol", "value": "Wasm"},
                                      {"type": "bytes", "value": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}]},
            {"type": "vec", "value": [{"type": "symbol", "value": "Wasm"},
                                      {"type": "bytes", "value": NEW_WASM_B64}]},
        ]),
        data: serde_json::Value::Null,
        event_index: 0,
        op_index: None,
        stage: None,
        ledger_sequence: E2E_LEDGER,
        created_at: 1_700_000_000,
    }
}

fn fixture_event(contract_id: &str, token: &str, order: u16) -> ExtractedNftEvent {
    ExtractedNftEvent {
        transaction_hash: tx_hash(),
        contract_id: contract_id.to_string(),
        token_id: token.to_string(),
        event_type: NftEventType::Transfer,
        owner_account: Some(owner_account()),
        event_order: order,
        ledger_sequence: E2E_LEDGER,
        created_at: 1_700_000_000,
    }
}

async fn cleanup(cl: &clickhouse::Client, contracts: &[&str]) {
    for c in contracts {
        let id = ids::contract_id(c);
        for stmt in [
            format!("ALTER TABLE soroban_contracts DELETE WHERE contract_id = '{c}'"),
            format!("ALTER TABLE nfts DELETE WHERE contract_id = {id}"),
            format!("ALTER TABLE nfts_pending DELETE WHERE contract_id = {id}"),
            format!("ALTER TABLE nft_ownership DELETE WHERE contract_id = {id}"),
            format!("ALTER TABLE nft_ownership_pending DELETE WHERE contract_id = {id}"),
        ] {
            let _ = cl.query(&stmt).execute().await;
        }
    }
    for stmt in [
        format!("ALTER TABLE ledgers DELETE WHERE sequence = {E2E_LEDGER}"),
        format!("ALTER TABLE transactions DELETE WHERE ledger_sequence = {E2E_LEDGER}"),
        format!("ALTER TABLE transaction_hash_index DELETE WHERE ledger_sequence = {E2E_LEDGER}"),
        format!("ALTER TABLE transaction_participants DELETE WHERE ledger_sequence = {E2E_LEDGER}"),
        format!("ALTER TABLE operations_appearances DELETE WHERE ledger_sequence = {E2E_LEDGER}"),
        format!("ALTER TABLE soroban_events DELETE WHERE ledger_sequence = {E2E_LEDGER}"),
        format!(
            "ALTER TABLE accounts DELETE WHERE account_id = '{}'",
            owner_account()
        ),
    ] {
        let _ = cl.query(&stmt).execute().await;
    }
}

async fn count(cl: &clickhouse::Client, table: &str, contract: &str) -> u64 {
    let id = ids::contract_id(contract);
    cl.query(&format!(
        "SELECT count() FROM {table} WHERE contract_id = {id}"
    ))
    .fetch_one()
    .await
    .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

#[tokio::test]
async fn g9_cross_ledger_verdict_routes_nft_events() {
    let Some(url) = std::env::var("CLICKHOUSE_URL").ok() else {
        eprintln!("CLICKHOUSE_URL not set — skipping G9 routing e2e test");
        return;
    };
    let cfg = Config {
        url,
        ..Config::from_env()
    };
    let cl = client(&cfg);
    apply_init_sql(&cl).await.expect("apply init.sql");

    let fungible = contract('F');
    let nft = contract('N');
    let unknown = contract('U');
    let upgraded = contract('W');
    cleanup(&cl, &[&fungible, &nft, &unknown, &upgraded]).await;

    // Seed prior-ledger verdicts the way deploy-time G1 writes them
    // (Fungible = 3, Nft = 2). `unknown` gets NO row — must quarantine.
    // `upgraded` (also Fungible) is the 0320 subject: its prior row must be
    // read back and carried forward when an `executable_update` fires here.
    for (c, ty) in [(&fungible, 3i16), (&nft, 2i16), (&upgraded, 3i16)] {
        cl.query(&format!(
            "INSERT INTO soroban_contracts \
             (id, contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, \
              deployed_at_ledger, contract_type, is_sac) \
             VALUES ({}, '{c}', NULL, 1, NULL, 1, {ty}, false)",
            ids::contract_id(c)
        ))
        .execute()
        .await
        .expect("seed soroban_contracts verdict");
    }

    // One ledger: an NFT row + ownership event from each contract, none of
    // which deploys here → routing MUST come from the G9 read-back.
    let ledger = fixture_ledger();
    let txs = vec![fixture_tx()];
    let nfts = vec![
        fixture_nft(&fungible, "1"),
        fixture_nft(&nft, "1"),
        fixture_nft(&unknown, "1"),
    ];
    let events = vec![
        fixture_event(&fungible, "1", 0),
        fixture_event(&nft, "1", 1),
        fixture_event(&unknown, "1", 2),
    ];
    let contract_events = vec![(tx_hash(), vec![fixture_upgrade_event(&upgraded)])];
    persist_ledger_clickhouse(
        &cl,
        &ledger,
        &txs,
        &[],
        &contract_events,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &nfts,
        &events,
        &[],
        &[],
        &[],
        &[],
        &[],
        &ClassificationCache::new(),
    )
    .await
    .expect("persist must succeed");

    // Fungible verdict → DROP: nothing anywhere.
    for table in [
        "nfts",
        "nfts_pending",
        "nft_ownership",
        "nft_ownership_pending",
    ] {
        assert_eq!(
            count(&cl, table, &fungible).await,
            0,
            "fungible-verdict contract must be dropped from {table}"
        );
    }

    // Nft verdict → HOT: rows in `nfts` + `nft_ownership`, nothing pending.
    assert_eq!(count(&cl, "nfts", &nft).await, 1, "nft row lands hot");
    assert_eq!(
        count(&cl, "nft_ownership", &nft).await,
        1,
        "ownership event lands hot"
    );
    assert_eq!(count(&cl, "nfts_pending", &nft).await, 0);
    assert_eq!(count(&cl, "nft_ownership_pending", &nft).await, 0);

    // No verdict → PENDING: quarantine intact for the genuinely-unknown.
    assert_eq!(
        count(&cl, "nfts_pending", &unknown).await,
        1,
        "unclassified contract quarantines"
    );
    assert_eq!(count(&cl, "nft_ownership_pending", &unknown).await, 1);
    assert_eq!(count(&cl, "nfts", &unknown).await, 0);
    assert_eq!(count(&cl, "nft_ownership", &unknown).await, 0);

    // Task 0320: the prior-row prefetch must succeed (pre-fix it SELECTed the
    // dropped `name` column → Code 47 → no upgrade row ever written) and the
    // upgrade row must override wasm_hash while carrying identity forward.
    // `wasm_hash` is Nullable, so `hex()` yields Nullable(String) — the same
    // wire-type strictness this whole test exists to guard.
    let (wasm_hex, version, ty): (Option<String>, i64, Option<i16>) = cl
        .query(
            "SELECT lower(hex(wasm_hash)), wasm_uploaded_at_ledger, contract_type \
             FROM soroban_contracts FINAL WHERE contract_id = ?",
        )
        .bind(&upgraded)
        .fetch_one()
        .await
        .expect("upgraded contract row");
    assert_eq!(
        wasm_hex.as_deref(),
        Some("ee".repeat(32).as_str()),
        "upgrade rewrites wasm_hash"
    );
    assert_eq!(
        version,
        i64::from(E2E_LEDGER),
        "RMT version = upgrade ledger (wins the merge)"
    );
    assert_eq!(ty, Some(3), "contract_type carried forward on upgrade");

    cleanup(&cl, &[&fungible, &nft, &unknown, &upgraded]).await;
}
