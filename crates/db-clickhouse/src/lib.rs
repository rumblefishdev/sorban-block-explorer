//! ClickHouse pilot — connection layer for the parallel store.
//!
//! See ADR 0044 (`lore/2-adrs/0044_clickhouse-pilot-parallel-store.md`) for
//! the design and task 0204 for the implementation context. This crate is
//! deliberately self-contained: no other crate in the workspace depends on
//! it, and it does not depend on `crates/db`.

use clickhouse::Client;

pub mod persist;

#[cfg(feature = "aws-mtls")]
pub mod mtls;

/// Schema embedded at compile time. Applied verbatim by `db-clickhouse-init`
/// and re-exported here so callers can apply it programmatically (e.g. from
/// integration tests).
pub const INIT_SQL: &str = include_str!("../schema/init.sql");

/// Default ClickHouse HTTP endpoint when `CLICKHOUSE_URL` is not set.
pub const DEFAULT_URL: &str = "http://localhost:8123";

/// Default ClickHouse user when `CLICKHOUSE_USER` is not set.
///
/// Local-dev posture only: no production secrets in the compose file.
pub const DEFAULT_USER: &str = "default";

/// Production CH database — `schema/init.sql` creates every table here.
/// Shared so the api / indexer / backfill-runner Lambdas all reach the
/// same logical store under mTLS.
pub const PROD_DATABASE: &str = "default";

/// Configuration for a ClickHouse client, sourced from environment variables
/// with safe local-dev defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Config {
    /// Read from `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
    /// `CLICKHOUSE_DATABASE`. Each falls back to the `DEFAULT_*` constants.
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
            user: std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| DEFAULT_USER.to_string()),
            password: std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_default(),
            database: std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "default".into()),
        }
    }
}

/// Build a `clickhouse::Client` from a `Config`.
///
/// Clients are cheap to clone — clone instead of rebuilding so the underlying
/// hyper connection pool is reused.
pub fn client(cfg: &Config) -> Client {
    Client::default()
        .with_url(&cfg.url)
        .with_user(&cfg.user)
        .with_password(&cfg.password)
        .with_database(&cfg.database)
}

/// Errors that can happen while applying the schema or persisting a ledger.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// Underlying CH transport / driver failure (`clickhouse::error::Error`
    /// wraps HTTP, serialization, and server-reported issues).
    #[error("clickhouse query failed: {0}")]
    Query(#[from] clickhouse::error::Error),
    /// Pre-write staging failure (decode error, overflow, malformed
    /// parser output, …) — surfaced before any row hits ClickHouse.
    #[error("clickhouse staging failed: {0}")]
    Staging(String),
}

/// Apply `init.sql` to the given client. Idempotent: every statement is a
/// `CREATE … IF NOT EXISTS`, so running twice is a no-op.
///
/// The HTTP query endpoint accepts one statement per request, so we split
/// the file on `;` and submit each statement individually. The DDL has no
/// embedded semicolons (no string literals, no functions), so naive
/// splitting is safe — keep the schema in that style.
pub async fn apply_init_sql(client: &Client) -> Result<(), SchemaError> {
    for stmt in split_statements(INIT_SQL) {
        client.query(&stmt).execute().await?;
    }
    Ok(())
}

/// Split a multi-statement SQL string into individual statements.
///
/// Strips line comments (`-- …`) and empty statements. Does not handle
/// quoted strings or block comments — the caller must keep `init.sql`
/// free of either.
fn split_statements(sql: &str) -> Vec<String> {
    let stripped: String = sql
        .lines()
        .map(|line| match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    stripped
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_statements_drops_line_comments_and_empty_chunks() {
        let sql = "-- top comment\n\
                   CREATE TABLE a (x Int64) ENGINE = MergeTree ORDER BY x;\n\
                   -- mid comment\n\
                   CREATE TABLE b (y Int64) ENGINE = MergeTree ORDER BY y;\n\
                   ;\n";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE a"));
        assert!(stmts[1].starts_with("CREATE TABLE b"));
    }

    #[test]
    fn init_sql_parses_into_statements() {
        let stmts = split_statements(INIT_SQL);
        // Current: 25 CREATE TABLE + 1 CREATE MATERIALIZED VIEW + 1 CREATE DICTIONARY
        // = 27 (matches the assertion below). Historical derivation:
        // 17-table base; task 0217 added `nfts_pending` + `nft_ownership_pending`
        // as schema-only landing zones (the CH writer does NOT yet stage/INSERT
        // into either — follow-up to PR #180); task 0231 (ADR 0050) added the
        // `asset_enrichment` + `nft_enrichment` side tables; lore-0293 added the
        // `asset_aggregates` table + its refreshable `asset_aggregates_mv` for
        // pre-computed asset aggregates (`assets.total_supply` / `holder_count`
        // now served from `asset_aggregates`; the dead columns were dropped in
        // task 0310); task 0297 added the `soroban_contract_metadata` side
        // table (on-chain token name/symbol/decimals); ADR 0051 / task 0339 added
        // the `asset_sac` SAC-facet side table; task 0331 FIRST added an INTERIM
        // `soroban_token_balances` per-holder side table (asset_type=3 balances
        // from ContractData `Balance(Address)` ledger state), then
        // task 0331 Option C: unified `balances` + `balance_aggregates`
        // (+ its refreshable MV) REPLACED that interim `soroban_token_balances` +
        // `soroban_asset_aggregates` (+ MV) — net one fewer table, same MV count.
        // (The interim `addresses` resolution table was dropped — holder→StrKey
        // resolves via `accounts` (G) / `soroban_contracts` (C), no dimension.)
        // task 0331 simplification: DROPPED the legacy `asset_aggregates` table +
        // `asset_aggregates_mv` (classic supply over `account_balances_current`) —
        // superseded by `balance_aggregates` over `balances`. 29 → 27.
        // task 0331 Option A: DROPPED `soroban_token_supply` (the per-token
        // `TotalSupply` key read) — `balance_aggregates` (Σ amount) is the sole
        // supply source; one universal method, no seed-only staleness. 27 → 26.
        // ADR 0051 / task 0339 (merged): added the `asset_sac` SAC-facet side table
        // (SAC-ness of a classic/native asset, not a distinct row). 26 → 27.
        // task 0359: added `operation_asset_appearances` — per-(asset, tx) presence
        // index (asset-leading key; native first-class). 27 → 28.
        // task 0365: added `operation_pools` — per-(pool, tx) presence index
        // (pool-leading key; the pool-dimension twin of the above). 28 → 29.
        // task 0385: added `accounts_recent` (last_seen_ledger-ordered read-model for
        // the acclist browse) + its refreshable `accounts_recent_mv`. 29 → 31.
        // task 0279: added `lp_operation_amounts` — per-(op, pool, asset) amounts
        // behind the pool page's "Amount" column (issue #371). 31 → 32.
        // task 0463: added `account_entry_state` — signers + thresholds side table
        // (issue #377); side table, not columns on `accounts`, because that
        // table takes whole-row writes from more than one path. 33 → 34:
        // pool_instance_state (task 0374 step 15, the asset_sac-pattern side
        // table: what a pool declares about itself) and pool_state_changes
        // (step 7, plane-state reserves).
        // 34 → 35.
        // task 0540 / 0541: added `asset_transfers` (one row per token
        // movement, the lossless replacement for `net_settled`),
        // `transaction_memos` (memo per transaction, from the envelope) and
        // `soroban_event_ops` (operation attribution per event, the side table
        // `soroban_events` cannot take). 35 → 38.
        assert_eq!(
            stmts.len(),
            38,
            "expected 34 tables + 2 materialized views + 1 dictionary, got {}",
            stmts.len()
        );
    }
}
