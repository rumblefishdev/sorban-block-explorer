-- Endpoint:     GET /contracts/:contract_id/interface
-- Purpose:      Contract interface: list of public functions with parameter
--               names and types, sourced from the WASM interface metadata
--               attached at deploy time. This powers the "Contract interface"
--               panel on the contract page (frontend §6.10) — readable
--               function signatures without reading the WASM source.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.10
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :contract_strkey  VARCHAR(56)  C-form contract ID
-- Indexes:      soroban_contracts UNIQUE (contract_id),
--               wasm_interface_metadata PK (wasm_hash).
-- Notes:
--   • Single statement. We resolve the contract → wasm_hash → metadata in
--     one query because both lookups are PK / UNIQUE-key probes.
--   • The metadata JSONB is the source of truth; the API decides which
--     subset to surface. We project it whole — there is no benefit to
--     extracting individual paths server-side because the row is already
--     small (one WASM blob's interface) and we save a JSON serialization
--     pass on the client.
--   • A contract can be SAC (no WASM hash) — the LEFT JOIN yields NULL
--     metadata in that case, which the API translates to "no interface
--     declared" or to a synthesized SAC interface stub.
--   • SCHEMA-DOC GAP: the JSONB shape of `wasm_interface_metadata.metadata`
--     is set by the indexer at ingest time (parsed from the WASM custom
--     section). It is NOT documented in ADR 0037 or anywhere else in
--     `docs/architecture/**`. Frontend §6.10 needs to render
--     "list of public functions with parameter names and types" — that
--     requires a known JSONB shape (e.g. `{ "functions": [{ "name": ...,
--     "params": [...], "returns": ... }] }`). Producing that shape is the
--     indexer's responsibility; documenting it is a follow-up. Until the
--     shape is locked, the API returns the JSONB verbatim and frontend is
--     expected to be defensive about missing keys.

SELECT
    sc.contract_id,
    encode(sc.wasm_hash, 'hex')   AS wasm_hash_hex,
    wim.metadata                  AS interface_metadata
FROM soroban_contracts sc
LEFT JOIN wasm_interface_metadata wim ON wim.wasm_hash = sc.wasm_hash
WHERE sc.contract_id = $1;
