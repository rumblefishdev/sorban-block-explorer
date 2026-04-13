//! Persistence functions for Soroban derived-state tables.
//!
//! - `accounts`, `nfts`, `liquidity_pools`: watermark-guarded upserts (last_seen/last_updated_ledger)
//! - `soroban_contracts`: COALESCE-merge upserts (no watermark; deployment fields are immutable once set)
//! - `tokens`: insert-or-ignore (effectively immutable once discovered)
//! - `liquidity_pool_snapshots`: append-only inserts (ON CONFLICT DO NOTHING)
//!
//! For immutable-table inserts (ledgers, transactions, operations, events,
//! invocations), see the `persistence` module.

use domain::{
    account::Account,
    nft::Nft,
    pool::{LiquidityPool, LiquidityPoolSnapshot},
    soroban::SorobanContract,
    token::Token,
};
use sqlx::Acquire;

/// Ensure minimal `soroban_contracts` rows exist for all given contract_ids.
/// Uses UNNEST to insert all stubs in a single round trip.
pub async fn ensure_contracts_exist_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    contract_ids: &[&str],
) -> Result<(), sqlx::Error> {
    if contract_ids.is_empty() {
        return Ok(());
    }
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_contracts (contract_id)
           SELECT * FROM unnest($1::text[])
           ON CONFLICT (contract_id) DO NOTHING"#,
    )
    .bind(contract_ids)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch update `transactions.operation_tree` for multiple transactions.
/// Uses UPDATE FROM UNNEST for a single round trip.
pub async fn update_operation_trees_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    transaction_ids: &[i64],
    operation_trees: &[&serde_json::Value],
) -> Result<(), sqlx::Error> {
    if transaction_ids.is_empty() {
        return Ok(());
    }
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"UPDATE transactions t
           SET operation_tree = u.tree
           FROM unnest($1::bigint[], $2::jsonb[]) AS u(id, tree)
           WHERE t.id = u.id"#,
    )
    .bind(transaction_ids)
    .bind(operation_trees)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch upsert contract deployments into `soroban_contracts`.
/// Uses UNNEST for a single round trip.
pub async fn upsert_contract_deployments_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    contracts: &[SorobanContract],
) -> Result<(), sqlx::Error> {
    if contracts.is_empty() {
        return Ok(());
    }
    let len = contracts.len();
    let mut contract_ids = Vec::with_capacity(len);
    let mut wasm_hashes: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut deployer_accounts: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut deployed_at_ledgers: Vec<Option<i64>> = Vec::with_capacity(len);
    let mut contract_types: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut is_sacs = Vec::with_capacity(len);
    let mut metadatas: Vec<Option<&serde_json::Value>> = Vec::with_capacity(len);

    for c in contracts {
        contract_ids.push(c.contract_id.as_str());
        wasm_hashes.push(c.wasm_hash.as_deref());
        deployer_accounts.push(c.deployer_account.as_deref());
        deployed_at_ledgers.push(c.deployed_at_ledger);
        contract_types.push(c.contract_type.as_deref());
        is_sacs.push(c.is_sac.unwrap_or(false));
        metadatas.push(c.metadata.as_ref());
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_contracts
               (contract_id, wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac, metadata)
           SELECT * FROM unnest(
               $1::text[], $2::text[], $3::text[], $4::bigint[], $5::text[], $6::bool[], $7::jsonb[]
           )
           ON CONFLICT (contract_id) DO UPDATE SET
               wasm_hash = COALESCE(soroban_contracts.wasm_hash, EXCLUDED.wasm_hash),
               deployer_account = COALESCE(soroban_contracts.deployer_account, EXCLUDED.deployer_account),
               deployed_at_ledger = COALESCE(soroban_contracts.deployed_at_ledger, EXCLUDED.deployed_at_ledger),
               contract_type = COALESCE(soroban_contracts.contract_type, EXCLUDED.contract_type),
               is_sac = EXCLUDED.is_sac OR soroban_contracts.is_sac,
               metadata = COALESCE(soroban_contracts.metadata, '{}'::jsonb) || COALESCE(EXCLUDED.metadata, '{}'::jsonb)"#,
    )
    .bind(&contract_ids)
    .bind(&wasm_hashes)
    .bind(&deployer_accounts)
    .bind(&deployed_at_ledgers)
    .bind(&contract_types)
    .bind(&is_sacs)
    .bind(&metadatas)
    .execute(&mut *conn)
    .await?;

    // Apply any pre-staged interface metadata for contracts with known wasm_hash.
    // This handles the 2-ledger deploy pattern: WASM uploaded in ledger A (interface
    // extracted and staged), contract deployed in ledger B (metadata applied here).
    let cids_with_wasm: Vec<&str> = contracts
        .iter()
        .filter(|c| c.wasm_hash.is_some())
        .map(|c| c.contract_id.as_str())
        .collect();
    if !cids_with_wasm.is_empty() {
        sqlx::query(
            r#"UPDATE soroban_contracts sc
               SET metadata = COALESCE(sc.metadata, '{}'::jsonb) || wim.metadata
               FROM wasm_interface_metadata wim
               WHERE sc.contract_id = ANY($1)
                 AND sc.wasm_hash = wim.wasm_hash"#,
        )
        .bind(&cids_with_wasm)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

/// Upsert WASM interface metadata into the staging table, keyed by `wasm_hash`.
///
/// This persists function signatures so they can be applied to contracts that are
/// deployed in a later ledger (2-ledger install-then-deploy pattern). Idempotent:
/// re-processing the same WASM overwrites with the same data.
pub async fn upsert_wasm_interface_metadata(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    wasm_hash: &str,
    metadata: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO wasm_interface_metadata (wasm_hash, metadata)
           VALUES ($1, $2)
           ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata"#,
    )
    .bind(wasm_hash)
    .bind(metadata)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Update `metadata` JSONB on all contracts matching the given `wasm_hash`.
///
/// Merges interface data (function signatures, wasm byte length) into any existing
/// metadata. Must run **after** `upsert_contract_deployment` so that `wasm_hash`
/// is populated on contract rows.
///
/// Returns the number of rows updated (0 if no contracts have this wasm_hash yet).
pub async fn update_contract_interfaces_by_wasm_hash(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    wasm_hash: &str,
    interface_metadata: &serde_json::Value,
) -> Result<u64, sqlx::Error> {
    let mut conn = executor.acquire().await?;
    let result = sqlx::query(
        r#"UPDATE soroban_contracts
           SET metadata = COALESCE(metadata, '{}'::jsonb) || $1
           WHERE wasm_hash = $2"#,
    )
    .bind(interface_metadata)
    .bind(wasm_hash)
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}

/// Batch upsert account states into `accounts`.
/// Uses UNNEST for a single round trip. Watermark on `last_seen_ledger`.
pub async fn upsert_account_states_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    accounts: &[Account],
) -> Result<(), sqlx::Error> {
    if accounts.is_empty() {
        return Ok(());
    }
    let len = accounts.len();
    let mut account_ids = Vec::with_capacity(len);
    let mut first_seen_ledgers = Vec::with_capacity(len);
    let mut last_seen_ledgers = Vec::with_capacity(len);
    let mut sequence_numbers = Vec::with_capacity(len);
    let mut balances = Vec::with_capacity(len);
    let mut home_domains: Vec<Option<&str>> = Vec::with_capacity(len);

    for a in accounts {
        account_ids.push(a.account_id.as_str());
        first_seen_ledgers.push(a.first_seen_ledger);
        last_seen_ledgers.push(a.last_seen_ledger);
        sequence_numbers.push(a.sequence_number);
        balances.push(&a.balances);
        home_domains.push(a.home_domain.as_deref());
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO accounts
               (account_id, first_seen_ledger, last_seen_ledger, sequence_number, balances, home_domain)
           SELECT * FROM unnest(
               $1::text[], $2::bigint[], $3::bigint[], $4::bigint[], $5::jsonb[], $6::text[]
           )
           ON CONFLICT (account_id) DO UPDATE SET
               last_seen_ledger = EXCLUDED.last_seen_ledger,
               sequence_number = EXCLUDED.sequence_number,
               balances = EXCLUDED.balances,
               home_domain = COALESCE(EXCLUDED.home_domain, accounts.home_domain)
           WHERE accounts.last_seen_ledger <= EXCLUDED.last_seen_ledger"#,
    )
    .bind(&account_ids)
    .bind(&first_seen_ledgers)
    .bind(&last_seen_ledgers)
    .bind(&sequence_numbers)
    .bind(&balances)
    .bind(&home_domains)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch upsert liquidity pool states into `liquidity_pools`.
/// Uses UNNEST for a single round trip. Watermark on `last_updated_ledger`.
pub async fn upsert_liquidity_pools_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    pools: &[LiquidityPool],
) -> Result<(), sqlx::Error> {
    if pools.is_empty() {
        return Ok(());
    }
    let len = pools.len();
    let mut pool_ids = Vec::with_capacity(len);
    let mut assets_a = Vec::with_capacity(len);
    let mut assets_b = Vec::with_capacity(len);
    let mut fee_bps_list = Vec::with_capacity(len);
    let mut reserves = Vec::with_capacity(len);
    let mut total_shares = Vec::with_capacity(len);
    let mut tvls: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut created_at_ledgers = Vec::with_capacity(len);
    let mut last_updated_ledgers = Vec::with_capacity(len);

    for lp in pools {
        pool_ids.push(lp.pool_id.as_str());
        assets_a.push(&lp.asset_a);
        assets_b.push(&lp.asset_b);
        fee_bps_list.push(lp.fee_bps);
        reserves.push(&lp.reserves);
        total_shares.push(lp.total_shares.as_str());
        tvls.push(lp.tvl.as_deref());
        created_at_ledgers.push(lp.created_at_ledger);
        last_updated_ledgers.push(lp.last_updated_ledger);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO liquidity_pools
               (pool_id, asset_a, asset_b, fee_bps, reserves, total_shares, tvl, created_at_ledger, last_updated_ledger)
           SELECT pool_id, asset_a, asset_b, fee_bps, reserves, total_shares::numeric, tvl::numeric, created_at_ledger, last_updated_ledger
           FROM unnest(
               $1::text[], $2::jsonb[], $3::jsonb[], $4::int[], $5::jsonb[], $6::text[], $7::text[], $8::bigint[], $9::bigint[]
           ) AS t(pool_id, asset_a, asset_b, fee_bps, reserves, total_shares, tvl, created_at_ledger, last_updated_ledger)
           ON CONFLICT (pool_id) DO UPDATE SET
               reserves = EXCLUDED.reserves,
               total_shares = EXCLUDED.total_shares,
               tvl = EXCLUDED.tvl,
               last_updated_ledger = EXCLUDED.last_updated_ledger
           WHERE liquidity_pools.last_updated_ledger <= EXCLUDED.last_updated_ledger"#,
    )
    .bind(&pool_ids)
    .bind(&assets_a)
    .bind(&assets_b)
    .bind(&fee_bps_list)
    .bind(&reserves)
    .bind(&total_shares)
    .bind(&tvls)
    .bind(&created_at_ledgers)
    .bind(&last_updated_ledgers)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch append liquidity pool snapshots (idempotent — replays are ignored).
/// Uses UNNEST for a single round trip.
pub async fn insert_liquidity_pool_snapshots_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    snapshots: &[LiquidityPoolSnapshot],
) -> Result<(), sqlx::Error> {
    if snapshots.is_empty() {
        return Ok(());
    }
    let len = snapshots.len();
    let mut pool_ids = Vec::with_capacity(len);
    let mut ledger_sequences = Vec::with_capacity(len);
    let mut created_ats = Vec::with_capacity(len);
    let mut reserves = Vec::with_capacity(len);
    let mut total_shares = Vec::with_capacity(len);
    let mut tvls: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut volumes: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut fee_revenues: Vec<Option<&str>> = Vec::with_capacity(len);

    for s in snapshots {
        pool_ids.push(s.pool_id.as_str());
        ledger_sequences.push(s.ledger_sequence);
        created_ats.push(s.created_at);
        reserves.push(&s.reserves);
        total_shares.push(s.total_shares.as_str());
        tvls.push(s.tvl.as_deref());
        volumes.push(s.volume.as_deref());
        fee_revenues.push(s.fee_revenue.as_deref());
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO liquidity_pool_snapshots
               (pool_id, ledger_sequence, created_at, reserves, total_shares, tvl, volume, fee_revenue)
           SELECT pool_id, ledger_sequence, created_at, reserves, total_shares::numeric, tvl::numeric, volume::numeric, fee_revenue::numeric
           FROM unnest(
               $1::text[], $2::bigint[], $3::timestamptz[], $4::jsonb[], $5::text[], $6::text[], $7::text[], $8::text[]
           ) AS t(pool_id, ledger_sequence, created_at, reserves, total_shares, tvl, volume, fee_revenue)
           ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING"#,
    )
    .bind(&pool_ids)
    .bind(&ledger_sequences)
    .bind(&created_ats)
    .bind(&reserves)
    .bind(&total_shares)
    .bind(&tvls)
    .bind(&volumes)
    .bind(&fee_revenues)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch upsert detected tokens into `tokens`.
/// Uses UNNEST for a single round trip.
pub async fn upsert_tokens_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    tokens: &[Token],
) -> Result<(), sqlx::Error> {
    if tokens.is_empty() {
        return Ok(());
    }
    let len = tokens.len();
    let mut asset_types = Vec::with_capacity(len);
    let mut asset_codes: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut issuer_addresses: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut contract_ids: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut names: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut total_supplies: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut holder_counts: Vec<Option<i32>> = Vec::with_capacity(len);

    for t in tokens {
        asset_types.push(t.asset_type.as_str());
        asset_codes.push(t.asset_code.as_deref());
        issuer_addresses.push(t.issuer_address.as_deref());
        contract_ids.push(t.contract_id.as_deref());
        names.push(t.name.as_deref());
        total_supplies.push(t.total_supply.as_deref());
        holder_counts.push(t.holder_count);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO tokens
               (asset_type, asset_code, issuer_address, contract_id, name, total_supply, holder_count)
           SELECT asset_type, asset_code, issuer_address, contract_id, name, total_supply::numeric, holder_count
           FROM unnest(
               $1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::int[]
           ) AS t(asset_type, asset_code, issuer_address, contract_id, name, total_supply, holder_count)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(&asset_types)
    .bind(&asset_codes)
    .bind(&issuer_addresses)
    .bind(&contract_ids)
    .bind(&names)
    .bind(&total_supplies)
    .bind(&holder_counts)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Batch upsert NFTs into `nfts`.
/// Uses UNNEST for a single round trip. Watermark on `last_seen_ledger`.
pub async fn upsert_nfts_batch(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    nfts: &[Nft],
) -> Result<(), sqlx::Error> {
    if nfts.is_empty() {
        return Ok(());
    }
    let len = nfts.len();
    let mut contract_ids = Vec::with_capacity(len);
    let mut token_ids = Vec::with_capacity(len);
    let mut collection_names: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut owner_accounts: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut names: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut media_urls: Vec<Option<&str>> = Vec::with_capacity(len);
    let mut metadatas: Vec<Option<&serde_json::Value>> = Vec::with_capacity(len);
    let mut minted_at_ledgers: Vec<Option<i64>> = Vec::with_capacity(len);
    let mut last_seen_ledgers = Vec::with_capacity(len);

    for n in nfts {
        contract_ids.push(n.contract_id.as_str());
        token_ids.push(n.token_id.as_str());
        collection_names.push(n.collection_name.as_deref());
        owner_accounts.push(n.owner_account.as_deref());
        names.push(n.name.as_deref());
        media_urls.push(n.media_url.as_deref());
        metadatas.push(n.metadata.as_ref());
        minted_at_ledgers.push(n.minted_at_ledger);
        last_seen_ledgers.push(n.last_seen_ledger);
    }

    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO nfts
               (contract_id, token_id, collection_name, owner_account, name, media_url, metadata, minted_at_ledger, last_seen_ledger)
           SELECT * FROM unnest(
               $1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::jsonb[], $8::bigint[], $9::bigint[]
           )
           ON CONFLICT (contract_id, token_id) DO UPDATE SET
               owner_account = EXCLUDED.owner_account,
               name = COALESCE(EXCLUDED.name, nfts.name),
               media_url = COALESCE(EXCLUDED.media_url, nfts.media_url),
               metadata = COALESCE(EXCLUDED.metadata, nfts.metadata),
               last_seen_ledger = EXCLUDED.last_seen_ledger
           WHERE nfts.last_seen_ledger <= EXCLUDED.last_seen_ledger"#,
    )
    .bind(&contract_ids)
    .bind(&token_ids)
    .bind(&collection_names)
    .bind(&owner_accounts)
    .bind(&names)
    .bind(&media_urls)
    .bind(&metadatas)
    .bind(&minted_at_ledgers)
    .bind(&last_seen_ledgers)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

// ── Single-row wrappers (used by tests) ────────────────────────────────

#[cfg(test)]
async fn upsert_account_state(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    account: &Account,
) -> Result<(), sqlx::Error> {
    upsert_account_states_batch(executor, std::slice::from_ref(account)).await
}

#[cfg(test)]
async fn upsert_nft(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    nft: &Nft,
) -> Result<(), sqlx::Error> {
    upsert_nfts_batch(executor, std::slice::from_ref(nft)).await
}

#[cfg(test)]
async fn upsert_liquidity_pool(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    lp: &LiquidityPool,
) -> Result<(), sqlx::Error> {
    upsert_liquidity_pools_batch(executor, std::slice::from_ref(lp)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{account::Account, nft::Nft, pool::LiquidityPool};
    use serde_json::json;

    async fn test_pool() -> Option<sqlx::PgPool> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!(
                "SKIP: DATABASE_URL not set — integration tests require a running PostgreSQL instance"
            );
            return None;
        };
        Some(
            sqlx::PgPool::connect(&url)
                .await
                .expect("failed to connect to DATABASE_URL"),
        )
    }

    fn test_account(id: &str, last_seen: i64) -> Account {
        Account {
            account_id: id.to_string(),
            first_seen_ledger: last_seen,
            last_seen_ledger: last_seen,
            sequence_number: 1,
            balances: json!([{"asset_type": "native", "balance": 1000}]),
            home_domain: None,
        }
    }

    fn test_nft(contract: &str, token: &str, last_seen: i64) -> Nft {
        Nft {
            contract_id: contract.to_string(),
            token_id: token.to_string(),
            collection_name: None,
            owner_account: Some("GOWNER".to_string()),
            name: None,
            media_url: None,
            metadata: None,
            minted_at_ledger: Some(last_seen),
            last_seen_ledger: last_seen,
        }
    }

    fn test_pool_entry(pool_id: &str, last_updated: i64) -> LiquidityPool {
        LiquidityPool {
            pool_id: pool_id.to_string(),
            asset_a: json!("native"),
            asset_b: json!({"type": "credit_alphanum4", "code": "USDC"}),
            fee_bps: 30,
            reserves: json!({"a": 1000, "b": 2000}),
            total_shares: "100".to_string(),
            tvl: None,
            created_at_ledger: last_updated,
            last_updated_ledger: last_updated,
        }
    }

    /// AC 13: Upserting the same account twice produces exactly one row.
    #[tokio::test]
    async fn account_upsert_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let id = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1";
        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let account = test_account(id, 100);
        upsert_account_state(&pool, &account).await.unwrap();
        upsert_account_state(&pool, &account).await.unwrap();

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE account_id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);

        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC 14: Older account data does not overwrite newer derived state.
    #[tokio::test]
    async fn account_watermark_blocks_stale_write() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let id = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA2";
        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let newer = Account {
            last_seen_ledger: 200,
            sequence_number: 99,
            ..test_account(id, 200)
        };
        let older = Account {
            last_seen_ledger: 50,
            sequence_number: 1,
            ..test_account(id, 50)
        };

        upsert_account_state(&pool, &newer).await.unwrap();
        upsert_account_state(&pool, &older).await.unwrap();

        let (seq, last): (i64, i64) = sqlx::query_as(
            "SELECT sequence_number, last_seen_ledger FROM accounts WHERE account_id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(last, 200, "last_seen_ledger should remain 200");
        assert_eq!(seq, 99, "sequence_number should remain from ledger 200");

        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC 13: first_seen_ledger is never overwritten after creation.
    #[tokio::test]
    async fn account_first_seen_ledger_immutable() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let id = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA3";
        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        upsert_account_state(&pool, &test_account(id, 100))
            .await
            .unwrap();
        upsert_account_state(
            &pool,
            &Account {
                first_seen_ledger: 999,
                last_seen_ledger: 200,
                ..test_account(id, 200)
            },
        )
        .await
        .unwrap();

        let (first,): (i64,) =
            sqlx::query_as("SELECT first_seen_ledger FROM accounts WHERE account_id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(first, 100, "first_seen_ledger must not be overwritten");

        sqlx::query("DELETE FROM accounts WHERE account_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC 14: Older NFT data does not overwrite newer last_seen_ledger.
    #[tokio::test]
    async fn nft_watermark_blocks_stale_write() {
        let Some(pool) = test_pool().await else {
            return;
        };

        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = 'CNFT_TEST_0028'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO soroban_contracts (contract_id) VALUES ('CNFT_TEST_0028') ON CONFLICT DO NOTHING",
        ).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM nfts WHERE contract_id = 'CNFT_TEST_0028' AND token_id = 'tok1'")
            .execute(&pool)
            .await
            .unwrap();

        let newer = Nft {
            owner_account: Some("GNEW".to_string()),
            last_seen_ledger: 300,
            ..test_nft("CNFT_TEST_0028", "tok1", 300)
        };
        let older = Nft {
            owner_account: Some("GOLD".to_string()),
            last_seen_ledger: 50,
            ..test_nft("CNFT_TEST_0028", "tok1", 50)
        };

        upsert_nft(&pool, &newer).await.unwrap();
        upsert_nft(&pool, &older).await.unwrap();

        let (owner, last): (Option<String>, i64) =
            sqlx::query_as("SELECT owner_account, last_seen_ledger FROM nfts WHERE contract_id = 'CNFT_TEST_0028' AND token_id = 'tok1'")
                .fetch_one(&pool).await.unwrap();
        assert_eq!(last, 300, "last_seen_ledger should remain 300");
        assert_eq!(
            owner.as_deref(),
            Some("GNEW"),
            "owner should remain from ledger 300"
        );

        sqlx::query("DELETE FROM nfts WHERE contract_id = 'CNFT_TEST_0028'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = 'CNFT_TEST_0028'")
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC 14: Older pool data does not overwrite newer last_updated_ledger.
    #[tokio::test]
    async fn pool_watermark_blocks_stale_write() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let pid = "aabbccdd0028test00000000000000000000000000000000000000000000aabb";
        sqlx::query("DELETE FROM liquidity_pools WHERE pool_id = $1")
            .bind(pid)
            .execute(&pool)
            .await
            .unwrap();

        let newer = LiquidityPool {
            reserves: json!({"a": 9000, "b": 8000}),
            last_updated_ledger: 500,
            ..test_pool_entry(pid, 500)
        };
        let older = LiquidityPool {
            reserves: json!({"a": 100, "b": 200}),
            last_updated_ledger: 10,
            ..test_pool_entry(pid, 10)
        };

        upsert_liquidity_pool(&pool, &newer).await.unwrap();
        upsert_liquidity_pool(&pool, &older).await.unwrap();

        let (last, reserves): (i64, serde_json::Value) = sqlx::query_as(
            "SELECT last_updated_ledger, reserves FROM liquidity_pools WHERE pool_id = $1",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(last, 500, "last_updated_ledger should remain 500");
        assert_eq!(
            reserves["a"], 9000,
            "reserves should remain from ledger 500"
        );

        sqlx::query("DELETE FROM liquidity_pools WHERE pool_id = $1")
            .bind(pid)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC: contract interface metadata propagates to all contracts sharing the same wasm_hash.
    #[tokio::test]
    async fn contract_interface_metadata_propagates_to_all_matching_contracts() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let wasm = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let cid_a = "CIFACE_TEST_0104_A_CONTRACT_ID_PLACEHOLDER_0000000000000";
        let cid_b = "CIFACE_TEST_0104_B_CONTRACT_ID_PLACEHOLDER_0000000000000";

        // Clean up
        for cid in &[cid_a, cid_b] {
            sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
                .bind(cid)
                .execute(&pool)
                .await
                .unwrap();
        }

        // Insert two contracts sharing the same wasm_hash
        for cid in &[cid_a, cid_b] {
            sqlx::query("INSERT INTO soroban_contracts (contract_id, wasm_hash) VALUES ($1, $2)")
                .bind(cid)
                .bind(wasm)
                .execute(&pool)
                .await
                .unwrap();
        }

        let metadata = serde_json::json!({
            "functions": [{"name": "transfer", "doc": "", "inputs": [], "outputs": []}],
            "wasm_byte_len": 4096,
        });
        let updated = update_contract_interfaces_by_wasm_hash(&pool, wasm, &metadata)
            .await
            .unwrap();
        assert_eq!(
            updated, 2,
            "both contracts sharing wasm_hash should be updated"
        );

        for cid in &[cid_a, cid_b] {
            let (meta,): (serde_json::Value,) =
                sqlx::query_as("SELECT metadata FROM soroban_contracts WHERE contract_id = $1")
                    .bind(cid)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                meta["functions"][0]["name"], "transfer",
                "contract {cid} should have interface metadata"
            );
            assert_eq!(meta["wasm_byte_len"], 4096);
        }

        for cid in &[cid_a, cid_b] {
            sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
                .bind(cid)
                .execute(&pool)
                .await
                .unwrap();
        }
    }

    /// AC: re-processing the same wasm_hash does not corrupt metadata (idempotent).
    #[tokio::test]
    async fn contract_interface_metadata_replay_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let wasm = "beef5678beef5678beef5678beef5678beef5678beef5678beef5678beef5678";
        let cid = "CIFACE_TEST_0104_REPLAY_CONTRACT_ID_PLACEHOLDER_00000000";

        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO soroban_contracts (contract_id, wasm_hash) VALUES ($1, $2)")
            .bind(cid)
            .bind(wasm)
            .execute(&pool)
            .await
            .unwrap();

        let metadata = serde_json::json!({
            "functions": [{"name": "mint", "doc": "", "inputs": [], "outputs": []}],
            "wasm_byte_len": 2048,
        });

        // Apply twice (simulating ledger replay)
        update_contract_interfaces_by_wasm_hash(&pool, wasm, &metadata)
            .await
            .unwrap();
        update_contract_interfaces_by_wasm_hash(&pool, wasm, &metadata)
            .await
            .unwrap();

        let (meta,): (serde_json::Value,) =
            sqlx::query_as("SELECT metadata FROM soroban_contracts WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            meta["functions"][0]["name"], "mint",
            "metadata should not be corrupted after replay"
        );
        assert_eq!(
            meta["wasm_byte_len"], 2048,
            "wasm_byte_len should not be duplicated or corrupted"
        );

        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// AC: 2-ledger deploy pattern — interface staged before contract exists, applied on deployment.
    ///
    /// Simulates:
    ///   Ledger A: WASM uploaded → upsert_wasm_interface_metadata (no contract row yet)
    ///   Ledger B: contract deployed → upsert_contract_deployment picks up staged metadata
    #[tokio::test]
    async fn staged_interface_metadata_applied_on_contract_deployment() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let wasm = "cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000cafe0000";
        let cid = "CIFACE_TEST_0104_STAGED_CONTRACT_ID_PLACEHOLDER_00000000";

        // Clean up both tables before test
        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = $1")
            .bind(wasm)
            .execute(&pool)
            .await
            .unwrap();

        // Ledger A: WASM uploaded, no contract row exists yet
        let interface_meta = serde_json::json!({
            "functions": [{"name": "swap", "doc": "", "inputs": [], "outputs": []}],
            "wasm_byte_len": 8192,
        });
        upsert_wasm_interface_metadata(&pool, wasm, &interface_meta)
            .await
            .unwrap();

        // Confirm no contract row yet
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM soroban_contracts WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "contract should not exist before deployment");

        // Ledger B: contract deployed with matching wasm_hash
        let contract = domain::soroban::SorobanContract {
            contract_id: cid.to_string(),
            wasm_hash: Some(wasm.to_string()),
            deployer_account: None,
            deployed_at_ledger: None,
            contract_type: None,
            is_sac: None,
            metadata: None,
        };
        upsert_contract_deployments_batch(&pool, &[contract])
            .await
            .unwrap();

        // Staged interface metadata should have been applied automatically
        let (meta,): (serde_json::Value,) =
            sqlx::query_as("SELECT metadata FROM soroban_contracts WHERE contract_id = $1")
                .bind(cid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            meta["functions"][0]["name"], "swap",
            "staged interface metadata should be applied at deployment time"
        );
        assert_eq!(meta["wasm_byte_len"], 8192);

        // Cleanup
        sqlx::query("DELETE FROM soroban_contracts WHERE contract_id = $1")
            .bind(cid)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wasm_interface_metadata WHERE wasm_hash = $1")
            .bind(wasm)
            .execute(&pool)
            .await
            .unwrap();
    }
}
