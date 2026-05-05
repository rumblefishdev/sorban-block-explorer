//! `assets` — natural key is path-dependent: native uses `asset_type`,
//! classic uses `(asset_code, issuer_strkey)`, contract-keyed uses
//! `contract_strkey`. Combined sort key concatenates all distinguishing
//! fields with NULL substitutes.
//! Surrogate `id` excluded; FKs resolved to natural keys.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT a.asset_type::text || '|' ||
           COALESCE(a.asset_code, '') || '|' ||
           COALESCE(acc.account_id, '') || '|' ||
           COALESCE(sc.contract_id, '') AS sk,
           a.asset_type::text || '|' ||
           COALESCE(a.asset_code, 'NULL') || '|' ||
           COALESCE(acc.account_id, 'NULL') || '|' ||
           COALESCE(sc.contract_id, 'NULL') || '|' ||
           COALESCE(a.name, 'NULL') || '|' ||
           COALESCE(a.total_supply::text, 'NULL') || '|' ||
           COALESCE(a.holder_count::text, 'NULL') AS canonical
      FROM assets a
      LEFT JOIN accounts acc ON acc.id = a.issuer_id
      LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
