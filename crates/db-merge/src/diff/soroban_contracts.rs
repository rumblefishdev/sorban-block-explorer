//! `soroban_contracts` — natural key `contract_id` (StrKey).
//! Surrogate `id` excluded; `search_vector` excluded (GENERATED ALWAYS).
//! `deployer_id` resolved to account StrKey.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sc.contract_id AS sk,
           sc.contract_id || '|' ||
           COALESCE(encode(sc.wasm_hash, 'hex'), 'NULL') || '|' ||
           COALESCE(sc.wasm_uploaded_at_ledger::text, 'NULL') || '|' ||
           COALESCE(a.account_id, 'NULL') || '|' ||
           COALESCE(sc.deployed_at_ledger::text, 'NULL') || '|' ||
           COALESCE(sc.contract_type::text, 'NULL') || '|' ||
           sc.is_sac::text || '|' ||
           COALESCE(sc.metadata::text, 'NULL') AS canonical
      FROM soroban_contracts sc
      LEFT JOIN accounts a ON a.id = sc.deployer_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
