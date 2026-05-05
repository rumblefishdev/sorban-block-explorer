//! `soroban_invocations_appearances` — PK
//! `(contract_id, transaction_id, ledger_sequence, created_at)`.
//! `caller_id` (account, nullable) and `caller_contract_id` (contract,
//! nullable) resolved.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sc.contract_id || '|' || encode(t.hash, 'hex') || '|' ||
           sia.ledger_sequence::text || '|' || sia.created_at::text AS sk,
           sc.contract_id || '|' ||
           encode(t.hash, 'hex') || '|' ||
           sia.ledger_sequence::text || '|' ||
           COALESCE(ca.account_id, 'NULL') || '|' ||
           COALESCE(ccc.contract_id, 'NULL') || '|' ||
           sia.amount::text || '|' ||
           sia.created_at::text AS canonical
      FROM soroban_invocations_appearances sia
      JOIN soroban_contracts sc ON sc.id = sia.contract_id
      JOIN transactions t ON t.id = sia.transaction_id AND t.created_at = sia.created_at
      LEFT JOIN accounts ca ON ca.id = sia.caller_id
      LEFT JOIN soroban_contracts ccc ON ccc.id = sia.caller_contract_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
