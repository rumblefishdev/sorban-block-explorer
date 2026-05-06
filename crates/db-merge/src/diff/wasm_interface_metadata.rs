//! `wasm_interface_metadata` — natural key `wasm_hash` (BYTEA → hex).

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(wasm_hash, 'hex') AS sk,
           encode(wasm_hash, 'hex') || '|' ||
           metadata::text AS canonical
      FROM wasm_interface_metadata
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
