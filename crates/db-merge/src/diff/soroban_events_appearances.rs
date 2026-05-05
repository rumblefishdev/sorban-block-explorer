//! `soroban_events_appearances` — PK
//! `(contract_id, transaction_id, ledger_sequence, created_at)`.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sc.contract_id || '|' || encode(t.hash, 'hex') || '|' ||
           sea.ledger_sequence::text || '|' || sea.created_at::text AS sk,
           sc.contract_id || '|' ||
           encode(t.hash, 'hex') || '|' ||
           sea.ledger_sequence::text || '|' ||
           sea.amount::text || '|' ||
           sea.created_at::text AS canonical
      FROM soroban_events_appearances sea
      JOIN soroban_contracts sc ON sc.id = sea.contract_id
      JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
