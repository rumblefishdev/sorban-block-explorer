//! `operations_appearances` — natural key `uq_ops_app_identity`
//! reduced to natural-key form. Surrogate `id` excluded.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(t.hash, 'hex') || '|' || oa.type::text || '|' ||
           COALESCE(s.account_id, '') || '|' ||
           COALESCE(d.account_id, '') || '|' ||
           COALESCE(sc.contract_id, '') || '|' ||
           COALESCE(oa.asset_code, '') || '|' ||
           COALESCE(ai.account_id, '') || '|' ||
           COALESCE(encode(oa.pool_id, 'hex'), '') || '|' ||
           oa.ledger_sequence::text || '|' || oa.created_at::text AS sk,
           encode(t.hash, 'hex') || '|' ||
           oa.type::text || '|' ||
           COALESCE(s.account_id, 'NULL') || '|' ||
           COALESCE(d.account_id, 'NULL') || '|' ||
           COALESCE(sc.contract_id, 'NULL') || '|' ||
           COALESCE(oa.asset_code, 'NULL') || '|' ||
           COALESCE(ai.account_id, 'NULL') || '|' ||
           COALESCE(encode(oa.pool_id, 'hex'), 'NULL') || '|' ||
           oa.amount::text || '|' ||
           oa.ledger_sequence::text || '|' ||
           oa.created_at::text AS canonical
      FROM operations_appearances oa
      JOIN transactions t ON t.id = oa.transaction_id AND t.created_at = oa.created_at
      LEFT JOIN accounts s ON s.id = oa.source_id
      LEFT JOIN accounts d ON d.id = oa.destination_id
      LEFT JOIN soroban_contracts sc ON sc.id = oa.contract_id
      LEFT JOIN accounts ai ON ai.id = oa.asset_issuer_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
