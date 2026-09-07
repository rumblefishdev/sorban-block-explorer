//! End-to-end smoke test for the ClickHouse pilot schema.
//!
//! Connects to a ClickHouse instance reachable at `CLICKHOUSE_URL`,
//! applies `init.sql` (idempotent — no-op if already applied), then for
//! each of the 17 mirrored tables: inserts one row, reads it back,
//! verifies the round-trip, and deletes the test data via partition
//! drop or `ALTER TABLE … DELETE`.
//!
//! Also exercises the `transaction_hash_dict` Dictionary: inserts a row
//! into `transaction_hash_index`, reloads the dictionary, and checks
//! `dictGet(...)` returns the expected `ledger_sequence`.
//!
//! Gated on `CLICKHOUSE_URL` env: skipped cleanly if unset, so CI
//! without a ClickHouse instance is green.

use db_clickhouse::{Config, apply_init_sql, client};

const SMOKE_LEDGER: i64 = 99_999_001; // out-of-band sentinel to avoid colliding with real data

/// Skip the test cleanly when no ClickHouse is reachable.
fn ch_url() -> Option<String> {
    std::env::var("CLICKHOUSE_URL").ok()
}

#[tokio::test]
async fn smoke_inserts_and_reads_each_table() {
    let Some(url) = ch_url() else {
        eprintln!("CLICKHOUSE_URL not set — skipping smoke test");
        return;
    };

    let cfg = Config {
        url,
        ..Config::from_env()
    };
    let client = client(&cfg);

    apply_init_sql(&client)
        .await
        .expect("apply init.sql must succeed (idempotent)");

    cleanup(&client).await;

    // ----- ledgers (immutable lookup, partitioned) -----
    client
        .query(
            "INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee) \
             VALUES (?, unhex('0000000000000000000000000000000000000000000000000000000000000001'), now64(3), 22, 1, 100)",
        )
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert ledgers");
    assert_count(&client, "ledgers", &format!("sequence = {SMOKE_LEDGER}"), 1).await;

    // ----- accounts (state) -----
    client
        .query(
            "INSERT INTO accounts (id, account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain) \
             VALUES (?, 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', ?, ?, 0, NULL)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert accounts");
    assert_count(&client, "accounts", &format!("id = {SMOKE_LEDGER}"), 1).await;

    // ----- assets (state) — composite PK, no surrogate `id` -----
    client
        .query(
            "INSERT INTO assets (asset_type, asset_code, issuer_id, contract_id) \
             VALUES (1, 'USDC', ?, 0)",
        )
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert assets");
    assert_count(
        &client,
        "assets",
        &format!("asset_code = 'USDC' AND issuer_id = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- account_balances_current (state) -----
    client
        .query(
            "INSERT INTO account_balances_current (account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger) \
             VALUES (?, 1, 'USDC', ?, toDecimal128('123.4567890', 7), ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert account_balances_current");
    assert_count(
        &client,
        "account_balances_current",
        &format!("account_id = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- balances (unified per-holder model, task 0331 Option C) -----
    client
        .query(
            "INSERT INTO balances (holder_id, asset_id, amount, last_updated_ledger) \
             VALUES (?, ?, 123456789, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert balances");
    assert_count(
        &client,
        "balances",
        &format!("holder_id = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // (`balance_aggregates` is filled by `balance_aggregates_mv`, a REFRESH EVERY
    // 2 MINUTE view — not synchronously assertable in a smoke test. The legacy
    // `asset_aggregates` refresh smoke was dropped with that table in the task 0331
    // simplification; classic supply now flows via `balance_aggregates` over `balances`.)

    // ----- soroban_contracts (state) -----
    client
        .query(
            "INSERT INTO soroban_contracts (id, contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, deployed_at_ledger, contract_type, is_sac) \
             VALUES (?, 'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', unhex('0000000000000000000000000000000000000000000000000000000000000002'), ?, NULL, NULL, NULL, false)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert soroban_contracts");
    assert_count(
        &client,
        "soroban_contracts",
        &format!("id = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- wasm_interface_metadata (immutable lookup) -----
    client
        .query(
            "INSERT INTO wasm_interface_metadata (wasm_hash, metadata) \
             VALUES (unhex('0000000000000000000000000000000000000000000000000000000000000099'), '{\"functions\":[]}')",
        )
        .execute()
        .await
        .expect("insert wasm_interface_metadata");
    assert_count(
        &client,
        "wasm_interface_metadata",
        "hex(wasm_hash) = '0000000000000000000000000000000000000000000000000000000000000099'",
        1,
    )
    .await;

    // ----- transactions (append-only fact, partitioned) -----
    client
        .query(
            "INSERT INTO transactions (id, hash, ledger_sequence, application_order, source_id, fee_charged, inner_tx_hash, successful, operation_count, has_soroban, parse_error) \
             VALUES (?, unhex('0000000000000000000000000000000000000000000000000000000000000003'), ?, 1, ?, 100, NULL, true, 1, false, false)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert transactions");
    assert_count(
        &client,
        "transactions",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- transaction_hash_index (append-only fact, source for Dictionary) -----
    client
        .query(
            "INSERT INTO transaction_hash_index (hash, ledger_sequence) \
             VALUES (unhex('00000000000000000000000000000000000000000000000000000000000000aa'), ?)",
        )
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert transaction_hash_index");
    assert_count(
        &client,
        "transaction_hash_index",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- operations_appearances (append-only fact) — no surrogate `id` -----
    client
        .query(
            "INSERT INTO operations_appearances (transaction_id, application_order, type, source_id, destination_id, contract_id, asset_code, asset_issuer_id, pool_ids, amount, ledger_sequence) \
             VALUES (?, 1, 1, ?, NULL, NULL, '', NULL, [], 100, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert operations_appearances");
    assert_count(
        &client,
        "operations_appearances",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- transaction_participants (append-only fact) -----
    client
        .query(
            "INSERT INTO transaction_participants (account_id, ledger_sequence, transaction_id) \
             VALUES (?, ?, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert transaction_participants");
    assert_count(
        &client,
        "transaction_participants",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- soroban_events (append-only fact, full-content; the v3 design) -----
    client
        .query(
            "INSERT INTO soroban_events (contract_id, transaction_id, ledger_sequence, event_index, event_type, signature, topics_xdr, data_xdr) \
             VALUES (?, ?, ?, 0, 1, 'transfer', 'topics', 'data')",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert soroban_events");
    assert_count(
        &client,
        "soroban_events",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- soroban_invocations_appearances (append-only fact) -----
    client
        .query(
            "INSERT INTO soroban_invocations_appearances (contract_id, transaction_id, ledger_sequence, caller_id, caller_contract_id, amount) \
             VALUES (?, ?, ?, ?, NULL, 1)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert soroban_invocations_appearances");
    assert_count(
        &client,
        "soroban_invocations_appearances",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- nfts (state) — composite PK, no surrogate `id` -----
    client
        .query(
            "INSERT INTO nfts (contract_id, token_id, collection_name, name, media_url, minted_at_ledger, current_owner_id, current_owner_ledger) \
             VALUES (?, 'tok-1', NULL, NULL, NULL, ?, ?, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert nfts");
    assert_count(
        &client,
        "nfts",
        &format!("contract_id = {SMOKE_LEDGER} AND token_id = 'tok-1'"),
        1,
    )
    .await;

    // ----- nft_ownership (append-only fact) — composite PK, no `nft_id` -----
    client
        .query(
            "INSERT INTO nft_ownership (contract_id, token_id, ledger_sequence, event_order, transaction_id, owner_id, event_type) \
             VALUES (?, 'tok-1', ?, 0, ?, ?, 0)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert nft_ownership");
    assert_count(
        &client,
        "nft_ownership",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- liquidity_pools (state) — version=last_updated_ledger (task 0208) -----
    client
        .query(
            "INSERT INTO liquidity_pools (pool_id, asset_a_type, asset_a_code, asset_a_issuer_id, asset_b_type, asset_b_code, asset_b_issuer_id, fee_bps, last_updated_ledger) \
             VALUES (unhex('00000000000000000000000000000000000000000000000000000000000000bb'), 0, '', 0, 1, 'USDC', ?, 30, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert liquidity_pools");
    assert_count(
        &client,
        "liquidity_pools",
        "hex(pool_id) = '00000000000000000000000000000000000000000000000000000000000000BB'",
        1,
    )
    .await;

    // ----- liquidity_pool_snapshots (append-only fact) — no surrogate `id` -----
    client
        .query(
            "INSERT INTO liquidity_pool_snapshots (pool_id, ledger_sequence, reserve_a, reserve_b, total_shares) \
             VALUES (unhex('00000000000000000000000000000000000000000000000000000000000000bb'), ?, toDecimal128('100.0', 7), toDecimal128('200.0', 7), toDecimal128('150.0', 7))",
        )
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert liquidity_pool_snapshots");
    assert_count(
        &client,
        "liquidity_pool_snapshots",
        &format!("ledger_sequence = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- lp_positions (state) -----
    client
        .query(
            "INSERT INTO lp_positions (pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger) \
             VALUES (unhex('00000000000000000000000000000000000000000000000000000000000000bb'), ?, toDecimal128('50.0', 7), ?, ?)",
        )
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .bind(SMOKE_LEDGER)
        .execute()
        .await
        .expect("insert lp_positions");
    assert_count(
        &client,
        "lp_positions",
        &format!("account_id = {SMOKE_LEDGER}"),
        1,
    )
    .await;

    // ----- transaction_hash_dict Dictionary -----
    // Force the cache to refresh against the row we just inserted.
    client
        .query("SYSTEM RELOAD DICTIONARY transaction_hash_dict")
        .execute()
        .await
        .expect("reload dict");

    let resolved: i64 = client
        .query(
            "SELECT dictGet('transaction_hash_dict', 'ledger_sequence', \
             tuple(toString(unhex('00000000000000000000000000000000000000000000000000000000000000aa'))))",
        )
        .fetch_one()
        .await
        .expect("dictGet");
    assert_eq!(
        resolved, SMOKE_LEDGER,
        "dictGet must resolve hash → ledger_sequence"
    );

    cleanup(&client).await;
}

/// Count rows in `table` matching `where_clause`.
///
/// No `FINAL`: plain `MergeTree` tables reject it, and the smoke test only
/// inserts one row per (table, sentinel) combination so background
/// `ReplacingMergeTree` dedup never hides anything.
async fn count_rows(client: &clickhouse::Client, table: &str, where_clause: &str) -> u64 {
    let q = format!("SELECT count() FROM {table} WHERE {where_clause}");
    client
        .query(&q)
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("count from {table}: {e}"))
}

/// Assert there are exactly `expected` rows in `table` matching `where_clause`.
/// The `balance_aggregates_mv` refreshable MV is the source of every user-facing
/// `total_supply` / `holder_count`, and the main smoke test can't assert it (2-minute
/// cadence). This proves it sums correctly: it runs in a THROWAWAY database (the MV
/// does a FULL recompute of `balances` into `balance_aggregates`, replacing the whole
/// target — must never touch shared data), forces an immediate refresh, and checks the
/// result. Gated on `CLICKHOUSE_URL`.
#[tokio::test]
async fn balance_aggregates_mv_sums_supply_and_holders() {
    let Some(url) = ch_url() else {
        eprintln!("CLICKHOUSE_URL not set — skipping balance_aggregates MV test");
        return;
    };
    let db = "ch_test_0331_balance_agg_mv";
    let base = client(&Config {
        url: url.clone(),
        ..Config::from_env()
    });
    base.query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop pre-existing throwaway db");
    base.query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .expect("create throwaway db");
    let cl = client(&Config {
        url,
        database: db.to_string(),
        ..Config::from_env()
    });
    apply_init_sql(&cl).await.expect("apply init schema");

    // One asset, three holders: 100 + 50 positive, 1 zeroed → supply 150, holders 2.
    let asset: i64 = 424_242;
    cl.query(
        "INSERT INTO balances (holder_id, asset_id, amount, last_updated_ledger) VALUES \
         (1, ?, 100, 10), (2, ?, 50, 10), (3, ?, 0, 10)",
    )
    .bind(asset)
    .bind(asset)
    .bind(asset)
    .execute()
    .await
    .expect("insert balances");

    // Force the refreshable MV to recompute NOW (vs REFRESH EVERY 2 MINUTE) and wait.
    cl.query("SYSTEM REFRESH VIEW balance_aggregates_mv")
        .execute()
        .await
        .expect("refresh mv");
    cl.query("SYSTEM WAIT VIEW balance_aggregates_mv")
        .execute()
        .await
        .expect("wait mv");

    #[derive(clickhouse::Row, serde::Deserialize)]
    struct Agg {
        total_supply: Option<i128>,
        holder_count: Option<i32>,
    }
    let agg = cl
        .query("SELECT total_supply, holder_count FROM balance_aggregates WHERE asset_id = ?")
        .bind(asset)
        .fetch_one::<Agg>()
        .await
        .expect("aggregate row");
    assert_eq!(
        agg.total_supply,
        Some(150),
        "sum(amount) over the 3 balance rows"
    );
    assert_eq!(agg.holder_count, Some(2), "countIf(amount > 0)");

    base.query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("cleanup throwaway db");
}

async fn assert_count(client: &clickhouse::Client, table: &str, where_clause: &str, expected: u64) {
    let actual = count_rows(client, table, where_clause).await;
    assert_eq!(
        actual, expected,
        "table {table} expected {expected} row(s) for `{where_clause}`, got {actual}"
    );
}

/// Wipe all sentinel rows. Run before and after the test so a previous
/// failed run doesn't poison the next attempt.
async fn cleanup(client: &clickhouse::Client) {
    let l = SMOKE_LEDGER;
    let stmts = [
        format!("ALTER TABLE ledgers DELETE WHERE sequence = {l}"),
        format!("ALTER TABLE accounts DELETE WHERE id = {l}"),
        format!(
            "ALTER TABLE assets DELETE WHERE asset_type = 1 AND asset_code = 'USDC' \
             AND issuer_id = {l} AND contract_id = 0"
        ),
        format!("ALTER TABLE account_balances_current DELETE WHERE account_id = {l}"),
        format!("ALTER TABLE soroban_contracts DELETE WHERE id = {l}"),
        "ALTER TABLE wasm_interface_metadata DELETE WHERE hex(wasm_hash) = '0000000000000000000000000000000000000000000000000000000000000099'".into(),
        format!("ALTER TABLE transactions DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE transaction_hash_index DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE operations_appearances DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE transaction_participants DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE soroban_events DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE soroban_invocations_appearances DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE nfts DELETE WHERE contract_id = {l} AND token_id = 'tok-1'"),
        format!("ALTER TABLE nft_ownership DELETE WHERE ledger_sequence = {l}"),
        "ALTER TABLE liquidity_pools DELETE WHERE hex(pool_id) = '00000000000000000000000000000000000000000000000000000000000000BB'".into(),
        format!("ALTER TABLE liquidity_pool_snapshots DELETE WHERE ledger_sequence = {l}"),
        format!("ALTER TABLE lp_positions DELETE WHERE account_id = {l}"),
        // `balances` was missing here — a run failing after its insert left
        // sentinel rows that poisoned the next run's count (lore-0392).
        format!("ALTER TABLE balances DELETE WHERE holder_id = {l}"),
    ];
    for s in stmts {
        // Ignore errors: cleanup runs on a freshly-created schema where the
        // tables exist but rows might not. ALTER … DELETE is async and
        // never errors on no-match.
        let _ = client
            .query(&s)
            .with_setting("mutations_sync", "1")
            .execute()
            .await;
    }
}
