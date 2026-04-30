-- ============================================================================
-- wasm_interface_metadata — unpartitioned. Keyed by wasm_hash (PK).
-- Columns: wasm_hash, metadata
-- ============================================================================
\echo '## wasm_interface_metadata'

\echo '### I1 — wasm_hash UNIQUE (PK)'
SELECT COUNT(*) AS violations
FROM (SELECT wasm_hash FROM wasm_interface_metadata GROUP BY wasm_hash HAVING COUNT(*) > 1) d;

\echo '### I2 — wasm_hash exactly 32 bytes'
SELECT COUNT(*) AS violations
FROM wasm_interface_metadata
WHERE octet_length(wasm_hash) <> 32;

\echo '### I3 — metadata is valid JSONB object (not NULL, not array, not scalar)'
SELECT COUNT(*) AS violations
FROM wasm_interface_metadata
WHERE metadata IS NULL OR jsonb_typeof(metadata) <> 'object';
