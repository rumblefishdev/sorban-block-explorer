//! `nfts` — natural key `(contract_strkey, token_id)`.
//! Surrogate `id` excluded; `contract_id`, `current_owner_id` resolved.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sc.contract_id || '|' || n.token_id AS sk,
           sc.contract_id || '|' ||
           n.token_id || '|' ||
           COALESCE(n.collection_name, 'NULL') || '|' ||
           COALESCE(n.name, 'NULL') || '|' ||
           COALESCE(n.media_url, 'NULL') || '|' ||
           COALESCE(n.metadata::text, 'NULL') || '|' ||
           COALESCE(n.minted_at_ledger::text, 'NULL') || '|' ||
           COALESCE(a.account_id, 'NULL') || '|' ||
           COALESCE(n.current_owner_ledger::text, 'NULL') AS canonical
      FROM nfts n
      JOIN soroban_contracts sc ON sc.id = n.contract_id
      LEFT JOIN accounts a ON a.id = n.current_owner_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
