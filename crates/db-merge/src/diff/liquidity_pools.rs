//! `liquidity_pools` — natural key `pool_id` (BYTEA → hex).
//! Issuer FKs resolved to account StrKeys.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(lp.pool_id, 'hex') AS sk,
           encode(lp.pool_id, 'hex') || '|' ||
           lp.asset_a_type::text || '|' ||
           COALESCE(lp.asset_a_code, 'NULL') || '|' ||
           COALESCE(a.account_id, 'NULL') || '|' ||
           lp.asset_b_type::text || '|' ||
           COALESCE(lp.asset_b_code, 'NULL') || '|' ||
           COALESCE(b.account_id, 'NULL') || '|' ||
           lp.fee_bps::text || '|' ||
           lp.created_at_ledger::text AS canonical
      FROM liquidity_pools lp
      LEFT JOIN accounts a ON a.id = lp.asset_a_issuer_id
      LEFT JOIN accounts b ON b.id = lp.asset_b_issuer_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
