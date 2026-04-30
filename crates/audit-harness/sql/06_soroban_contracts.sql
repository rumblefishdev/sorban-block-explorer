-- ============================================================================
-- soroban_contracts — surrogate BIGSERIAL id + natural contract_id (StrKey).
-- Columns: id, contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id,
--          deployed_at_ledger, contract_type, is_sac, metadata, search_vector
-- ============================================================================
\echo '## soroban_contracts'

\echo '### I1 — contract_id matches StrKey shape (56 chars, prefix C, base32)'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(contract_id) FROM (
           SELECT contract_id FROM soroban_contracts
           WHERE NOT (length(contract_id) = 56
                     AND contract_id LIKE 'C%'
                     AND contract_id ~ '^[A-Z2-7]+$')
           ORDER BY id LIMIT 5
       ) s) AS sample
FROM soroban_contracts
WHERE NOT (length(contract_id) = 56
          AND contract_id LIKE 'C%'
          AND contract_id ~ '^[A-Z2-7]+$');

\echo '### I2 — contract_id UNIQUE'
SELECT COUNT(*) AS violations
FROM (SELECT contract_id FROM soroban_contracts GROUP BY contract_id HAVING COUNT(*) > 1) d;

\echo '### I3 — deployer_id FK valid where set'
SELECT COUNT(*) AS violations
FROM soroban_contracts c
LEFT JOIN accounts a ON a.id = c.deployer_id
WHERE c.deployer_id IS NOT NULL AND a.id IS NULL;

\echo '### I4 — wasm_hash (when set) → wasm_interface_metadata.wasm_hash'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(c.wasm_hash,'hex')) FROM (
           SELECT c.wasm_hash FROM soroban_contracts c
           LEFT JOIN wasm_interface_metadata w ON w.wasm_hash = c.wasm_hash
           WHERE c.wasm_hash IS NOT NULL AND w.wasm_hash IS NULL
           ORDER BY c.id LIMIT 5
       ) s) AS sample
FROM soroban_contracts c
LEFT JOIN wasm_interface_metadata w ON w.wasm_hash = c.wasm_hash
WHERE c.wasm_hash IS NOT NULL AND w.wasm_hash IS NULL;

\echo '### I5 — contract_type SMALLINT in known range (per ADR 0031 + ADR 0036)'
-- Known: 0=unknown, 1=token (legacy?), 2=NFT, 3=fungible, 4=SAC, others=schema drift
SELECT COUNT(*) AS violations
FROM soroban_contracts
WHERE contract_type < 0 OR contract_type > 10;

\echo '### I6 — wasm_hash exactly 32 bytes when set'
SELECT COUNT(*) AS violations
FROM soroban_contracts
WHERE wasm_hash IS NOT NULL AND octet_length(wasm_hash) <> 32;
