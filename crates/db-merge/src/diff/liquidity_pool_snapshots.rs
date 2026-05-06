//! `liquidity_pool_snapshots` — natural key `uq_lp_snapshots_pool_ledger`
//! `(pool_id, ledger_sequence)`. Surrogate `id` excluded.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(pool_id, 'hex') || '|' || ledger_sequence::text AS sk,
           encode(pool_id, 'hex') || '|' ||
           ledger_sequence::text || '|' ||
           reserve_a::text || '|' ||
           reserve_b::text || '|' ||
           total_shares::text || '|' ||
           COALESCE(tvl::text, 'NULL') || '|' ||
           COALESCE(volume::text, 'NULL') || '|' ||
           COALESCE(fee_revenue::text, 'NULL') || '|' ||
           created_at::text AS canonical
      FROM liquidity_pool_snapshots
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
