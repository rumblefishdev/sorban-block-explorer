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

/// Ensure a minimal `soroban_contracts` row exists for the given contract_id.
///
/// Inserts a stub row with only the contract_id if it doesn't exist yet.
/// This satisfies FK constraints when events/invocations reference contracts
/// that were deployed in earlier ledgers (not yet in the database).
pub async fn ensure_contract_exists(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    contract_id: &str,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_contracts (contract_id)
           VALUES ($1)
           ON CONFLICT (contract_id) DO NOTHING"#,
    )
    .bind(contract_id)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Update `transactions.operation_tree` for a given transaction.
pub async fn update_operation_tree(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    transaction_id: i64,
    operation_tree: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(r#"UPDATE transactions SET operation_tree = $1 WHERE id = $2"#)
        .bind(operation_tree)
        .bind(transaction_id)
        .execute(&mut *conn)
        .await?;

    Ok(())
}

/// Upsert contract deployment into `soroban_contracts`.
///
/// Fills deployment fields (deployer_account, deployed_at_ledger, contract_type, is_sac).
/// Merges metadata with any existing data from interface extraction (task 0026).
pub async fn upsert_contract_deployment(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    contract: &SorobanContract,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO soroban_contracts
               (contract_id, wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac, metadata)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (contract_id) DO UPDATE SET
               wasm_hash = COALESCE(soroban_contracts.wasm_hash, EXCLUDED.wasm_hash),
               deployer_account = COALESCE(soroban_contracts.deployer_account, EXCLUDED.deployer_account),
               deployed_at_ledger = COALESCE(soroban_contracts.deployed_at_ledger, EXCLUDED.deployed_at_ledger),
               contract_type = COALESCE(soroban_contracts.contract_type, EXCLUDED.contract_type),
               is_sac = EXCLUDED.is_sac OR soroban_contracts.is_sac,
               metadata = COALESCE(soroban_contracts.metadata, '{}'::jsonb) || COALESCE(EXCLUDED.metadata, '{}'::jsonb)"#,
    )
    .bind(&contract.contract_id)
    .bind(&contract.wasm_hash)
    .bind(&contract.deployer_account)
    .bind(contract.deployed_at_ledger)
    .bind(&contract.contract_type)
    .bind(contract.is_sac.unwrap_or(false))
    .bind(&contract.metadata)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Upsert account state into `accounts`.
///
/// Uses watermark on `last_seen_ledger` to prevent stale backfill overwrites.
/// Sets `first_seen_ledger` only on first insert.
pub async fn upsert_account_state(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    account: &Account,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO accounts
               (account_id, first_seen_ledger, last_seen_ledger, sequence_number, balances, home_domain)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (account_id) DO UPDATE SET
               last_seen_ledger = EXCLUDED.last_seen_ledger,
               sequence_number = EXCLUDED.sequence_number,
               balances = EXCLUDED.balances,
               home_domain = COALESCE(EXCLUDED.home_domain, accounts.home_domain)
           WHERE accounts.last_seen_ledger <= EXCLUDED.last_seen_ledger"#,
    )
    .bind(&account.account_id)
    .bind(account.first_seen_ledger)
    .bind(account.last_seen_ledger)
    .bind(account.sequence_number)
    .bind(&account.balances)
    .bind(&account.home_domain)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Upsert liquidity pool state into `liquidity_pools`.
///
/// Uses watermark on `last_updated_ledger` to prevent stale backfill overwrites.
pub async fn upsert_liquidity_pool(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    lp: &LiquidityPool,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO liquidity_pools
               (pool_id, asset_a, asset_b, fee_bps, reserves, total_shares, tvl, created_at_ledger, last_updated_ledger)
           VALUES ($1, $2, $3, $4, $5, $6::numeric, $7::numeric, $8, $9)
           ON CONFLICT (pool_id) DO UPDATE SET
               reserves = EXCLUDED.reserves,
               total_shares = EXCLUDED.total_shares,
               tvl = EXCLUDED.tvl,
               last_updated_ledger = EXCLUDED.last_updated_ledger
           WHERE liquidity_pools.last_updated_ledger <= EXCLUDED.last_updated_ledger"#,
    )
    .bind(&lp.pool_id)
    .bind(&lp.asset_a)
    .bind(&lp.asset_b)
    .bind(lp.fee_bps)
    .bind(&lp.reserves)
    .bind(&lp.total_shares)
    .bind(lp.tvl.as_deref())
    .bind(lp.created_at_ledger)
    .bind(lp.last_updated_ledger)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Append a liquidity pool snapshot (idempotent — replays are ignored).
pub async fn insert_liquidity_pool_snapshot(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    snapshot: &LiquidityPoolSnapshot,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO liquidity_pool_snapshots
               (pool_id, ledger_sequence, created_at, reserves, total_shares, tvl, volume, fee_revenue)
           VALUES ($1, $2, $3, $4, $5::numeric, $6::numeric, $7::numeric, $8::numeric)
           ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING"#,
    )
    .bind(&snapshot.pool_id)
    .bind(snapshot.ledger_sequence)
    .bind(snapshot.created_at)
    .bind(&snapshot.reserves)
    .bind(&snapshot.total_shares)
    .bind(snapshot.tvl.as_deref())
    .bind(snapshot.volume.as_deref())
    .bind(snapshot.fee_revenue.as_deref())
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Upsert a detected token into `tokens`.
pub async fn upsert_token(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    token: &Token,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO tokens
               (asset_type, asset_code, issuer_address, contract_id, name, total_supply, holder_count)
           VALUES ($1, $2, $3, $4, $5, $6::numeric, $7)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(&token.asset_type)
    .bind(&token.asset_code)
    .bind(&token.issuer_address)
    .bind(&token.contract_id)
    .bind(&token.name)
    .bind(token.total_supply.as_deref())
    .bind(token.holder_count)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Upsert an NFT into `nfts`.
///
/// Uses watermark on `last_seen_ledger` to prevent stale overwrites.
/// Sets `minted_at_ledger` only on first insert.
pub async fn upsert_nft(
    executor: impl Acquire<'_, Database = sqlx::Postgres>,
    nft: &Nft,
) -> Result<(), sqlx::Error> {
    let mut conn = executor.acquire().await?;
    sqlx::query(
        r#"INSERT INTO nfts
               (contract_id, token_id, collection_name, owner_account, name, media_url, metadata, minted_at_ledger, last_seen_ledger)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (contract_id, token_id) DO UPDATE SET
               owner_account = EXCLUDED.owner_account,
               name = COALESCE(EXCLUDED.name, nfts.name),
               media_url = COALESCE(EXCLUDED.media_url, nfts.media_url),
               metadata = COALESCE(EXCLUDED.metadata, nfts.metadata),
               last_seen_ledger = EXCLUDED.last_seen_ledger
           WHERE nfts.last_seen_ledger <= EXCLUDED.last_seen_ledger"#,
    )
    .bind(&nft.contract_id)
    .bind(&nft.token_id)
    .bind(&nft.collection_name)
    .bind(&nft.owner_account)
    .bind(&nft.name)
    .bind(&nft.media_url)
    .bind(&nft.metadata)
    .bind(nft.minted_at_ledger)
    .bind(nft.last_seen_ledger)
    .execute(&mut *conn)
    .await?;

    Ok(())
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
}
