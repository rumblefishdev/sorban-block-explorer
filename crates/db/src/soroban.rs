//! Persistence functions for Soroban events, invocations, and contracts.
//!
//! All inserts use parameterized queries via sqlx. Inserts currently
//! execute one INSERT per row directly on the connection pool.

use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use tracing::warn;
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedContractInterface,
    ExtractedEvent, ExtractedInvocation, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedNft, ExtractedToken,
};

/// Insert extracted events into `soroban_events`.
///
/// Requires the parent `transaction_id` (BIGSERIAL from the transactions table).
/// Events are inserted in order; `created_at` determines the target partition.
pub async fn insert_events(
    pool: &PgPool,
    events: &[ExtractedEvent],
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    for event in events {
        let created_at = unix_to_datetime(event.created_at)?;
        sqlx::query(
            r#"INSERT INTO soroban_events
               (transaction_id, contract_id, event_type, topics, data, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(transaction_id)
        .bind(&event.contract_id)
        .bind(&event.event_type)
        .bind(&event.topics)
        .bind(&event.data)
        .bind(event.ledger_sequence as i64)
        .bind(created_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Insert extracted invocations into `soroban_invocations`.
///
/// Requires the parent `transaction_id` (BIGSERIAL from the transactions table).
pub async fn insert_invocations(
    pool: &PgPool,
    invocations: &[ExtractedInvocation],
    transaction_id: i64,
) -> Result<(), sqlx::Error> {
    for inv in invocations {
        let created_at = unix_to_datetime(inv.created_at)?;
        sqlx::query(
            r#"INSERT INTO soroban_invocations
               (transaction_id, contract_id, caller_account, function_name,
                function_args, return_value, successful, ledger_sequence, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(transaction_id)
        .bind(&inv.contract_id)
        .bind(&inv.caller_account)
        .bind(&inv.function_name)
        .bind(&inv.function_args)
        .bind(&inv.return_value)
        .bind(inv.successful)
        .bind(inv.ledger_sequence as i64)
        .bind(created_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Upsert contract interface metadata into `soroban_contracts`.
///
/// If the contract row already exists (created by task 0027), this updates
/// only the `metadata` column. If it doesn't exist yet, inserts a minimal
/// row with just `wasm_hash` and `metadata` — task 0027 will fill the rest.
pub async fn upsert_contract_metadata(
    pool: &PgPool,
    interface: &ExtractedContractInterface,
    contract_id: &str,
) -> Result<(), sqlx::Error> {
    let metadata = match serde_json::to_value(&interface.functions) {
        Ok(v) => serde_json::json!({
            "interface": { "functions": v },
            "wasm_hash": interface.wasm_hash,
            "wasm_byte_len": interface.wasm_byte_len,
        }),
        Err(e) => {
            warn!(contract_id, "failed to serialize contract interface: {e}");
            return Ok(());
        }
    };

    sqlx::query(
        r#"INSERT INTO soroban_contracts (contract_id, wasm_hash, metadata)
           VALUES ($1, $2, $3)
           ON CONFLICT (contract_id) DO UPDATE SET
               metadata = COALESCE(soroban_contracts.metadata, '{}'::jsonb) || EXCLUDED.metadata,
               wasm_hash = COALESCE(soroban_contracts.wasm_hash, EXCLUDED.wasm_hash)"#,
    )
    .bind(contract_id)
    .bind(&interface.wasm_hash)
    .bind(&metadata)
    .execute(pool)
    .await?;

    Ok(())
}

/// Update `transactions.operation_tree` for a given transaction.
pub async fn update_operation_tree(
    pool: &PgPool,
    transaction_id: i64,
    operation_tree: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE transactions SET operation_tree = $1 WHERE id = $2"#,
    )
    .bind(operation_tree)
    .bind(transaction_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert contract deployment into `soroban_contracts`.
///
/// Fills deployment fields (deployer_account, deployed_at_ledger, contract_type, is_sac).
/// Merges metadata with any existing data from interface extraction (task 0026).
pub async fn upsert_contract_deployment(
    pool: &PgPool,
    deployment: &ExtractedContractDeployment,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO soroban_contracts
               (contract_id, wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac, metadata)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (contract_id) DO UPDATE SET
               wasm_hash = COALESCE(soroban_contracts.wasm_hash, EXCLUDED.wasm_hash),
               deployer_account = COALESCE(soroban_contracts.deployer_account, EXCLUDED.deployer_account),
               deployed_at_ledger = COALESCE(soroban_contracts.deployed_at_ledger, EXCLUDED.deployed_at_ledger),
               contract_type = COALESCE(EXCLUDED.contract_type, soroban_contracts.contract_type),
               is_sac = EXCLUDED.is_sac OR soroban_contracts.is_sac,
               metadata = COALESCE(soroban_contracts.metadata, '{}'::jsonb) || EXCLUDED.metadata"#,
    )
    .bind(&deployment.contract_id)
    .bind(&deployment.wasm_hash)
    .bind(&deployment.deployer_account)
    .bind(deployment.deployed_at_ledger as i64)
    .bind(&deployment.contract_type)
    .bind(deployment.is_sac)
    .bind(&deployment.metadata)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert account state into `accounts`.
///
/// Uses watermark on `last_seen_ledger` to prevent stale backfill overwrites.
/// Sets `first_seen_ledger` only on first insert.
pub async fn upsert_account_state(
    pool: &PgPool,
    account: &ExtractedAccountState,
) -> Result<(), sqlx::Error> {
    let first_seen = account
        .first_seen_ledger
        .unwrap_or(account.last_seen_ledger) as i64;

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
    .bind(first_seen)
    .bind(account.last_seen_ledger as i64)
    .bind(account.sequence_number)
    .bind(&account.balances)
    .bind(&account.home_domain)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert liquidity pool state into `liquidity_pools`.
///
/// Uses watermark on `last_updated_ledger` to prevent stale backfill overwrites.
pub async fn upsert_liquidity_pool(
    pool: &PgPool,
    lp: &ExtractedLiquidityPool,
) -> Result<(), sqlx::Error> {
    let created_at_ledger = lp
        .created_at_ledger
        .unwrap_or(lp.last_updated_ledger) as i64;

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
    .bind(created_at_ledger)
    .bind(lp.last_updated_ledger as i64)
    .execute(pool)
    .await?;

    Ok(())
}

/// Append a liquidity pool snapshot (idempotent — replays are ignored).
pub async fn insert_liquidity_pool_snapshot(
    pool: &PgPool,
    snapshot: &ExtractedLiquidityPoolSnapshot,
) -> Result<(), sqlx::Error> {
    let created_at = unix_to_datetime(snapshot.created_at)?;

    sqlx::query(
        r#"INSERT INTO liquidity_pool_snapshots
               (pool_id, ledger_sequence, created_at, reserves, total_shares, tvl, volume, fee_revenue)
           VALUES ($1, $2, $3, $4, $5::numeric, $6::numeric, $7::numeric, $8::numeric)
           ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING"#,
    )
    .bind(&snapshot.pool_id)
    .bind(snapshot.ledger_sequence as i64)
    .bind(created_at)
    .bind(&snapshot.reserves)
    .bind(&snapshot.total_shares)
    .bind(snapshot.tvl.as_deref())
    .bind(snapshot.volume.as_deref())
    .bind(snapshot.fee_revenue.as_deref())
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert a detected token into `tokens`.
pub async fn upsert_token(
    pool: &PgPool,
    token: &ExtractedToken,
) -> Result<(), sqlx::Error> {
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
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert an NFT into `nfts`.
///
/// Uses watermark on `last_seen_ledger` to prevent stale overwrites.
/// Sets `minted_at_ledger` only on first insert.
pub async fn upsert_nft(
    pool: &PgPool,
    nft: &ExtractedNft,
) -> Result<(), sqlx::Error> {
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
    .bind(nft.minted_at_ledger.map(|v| v as i64))
    .bind(nft.last_seen_ledger as i64)
    .execute(pool)
    .await?;

    Ok(())
}

fn unix_to_datetime(unix_secs: i64) -> Result<DateTime<Utc>, sqlx::Error> {
    Utc.timestamp_opt(unix_secs, 0)
        .single()
        .ok_or_else(|| {
            sqlx::Error::Protocol(format!(
                "invalid unix timestamp {unix_secs} — cannot resolve to partition"
            ))
        })
}
