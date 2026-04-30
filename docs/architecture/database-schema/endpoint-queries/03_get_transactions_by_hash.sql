-- Endpoint:     GET /transactions/:hash
-- Purpose:      Full transaction detail. Header from DB; raw envelope/result/
--               result_meta XDR + parsed invocation tree fetched from the
--               public Stellar ledger archive at request time per ADR 0029
--               (NOT in DB). Operation list, participants, soroban events
--               and invocation appearances are all DB-side.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.4
-- Schema:       ADR 0037
-- Data sources: DB + Archive XDR (ADR 0029).
--               DB returns: header, operations[], participants[],
--                           soroban_events[] (appearance rows only),
--                           soroban_invocations[] (appearance rows only).
--               Archive returns: envelope_xdr, result_xdr, result_meta_xdr,
--                           parsed operation_tree, full event topics + data.
-- Inputs:
--   $1  :hash     BYTEA(32)  raw 32-byte transaction hash
-- Indexes:      transaction_hash_index PK (hash),
--               transactions uq_transactions_hash_created_at,
--               operations_appearances PK (id, created_at) — accessed via the
--                  composite FK lookup `(transaction_id, created_at)`,
--               transaction_participants PK (account_id, created_at, transaction_id),
--                  reached here via composite FK,
--               soroban_events_appearances PK (contract_id, transaction_id, ledger_sequence, created_at),
--               soroban_invocations_appearances PK (contract_id, transaction_id, ledger_sequence, created_at).
-- Notes:
--   • Six statements. The API runs them sequentially, threading
--     `(transaction_id, created_at)` from statement B into C-F.
--   • Statement A is the partition-pruning shortcut: hash → (ledger_sequence,
--     created_at) via transaction_hash_index. Without this, statement B would
--     scan every partition of `transactions` because there is no global
--     index on `hash` alone (uq_transactions_hash_created_at is per-partition).
--   • All projections that surface a StrKey come from a final join to
--     `accounts.account_id` / `soroban_contracts.contract_id`.
--   • Enum decoding: op_type_name() in projection (statement C),
--     soroban_events_appearances has no event_type column — that field is in
--     the archive XDR, not the appearance index (ADR 0033).

-- ============================================================================
-- A. Resolve hash → (ledger_sequence, created_at) for partition pruning.
-- ============================================================================
SELECT
    ledger_sequence,
    created_at
FROM transaction_hash_index
WHERE hash = $1;

-- @@ split @@

-- ============================================================================
-- B. Transaction header.
--    Inputs: $1 = hash, $2 = created_at (from statement A).
-- ============================================================================
SELECT
    t.id                            AS transaction_id,
    encode(t.hash, 'hex')           AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                  AS source_account,
    t.fee_charged,
    encode(t.inner_tx_hash, 'hex')  AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    t.parse_error,
    t.created_at
    -- not in DB: envelope_xdr, result_xdr, result_meta_xdr, operation_tree
    --           — Archive (.xdr.zst), parsed at request time. ADR 0029.
    --           Bridge for archive lookup: (ledger_sequence, transaction_id, hash).
    --           The archive's per-ledger blob contains all transactions in that
    --           ledger; the API picks ours by hash.
    -- not in DB: memo_type, memo_content
    --           — encoded inside envelope_xdr. Archive overlay. ADR 0029.
    -- not in DB: signatures[] (signer, weight, signature_hex)
    --           — encoded inside envelope_xdr. Archive overlay. ADR 0029.
FROM transactions t
JOIN accounts src ON src.id = t.source_id
WHERE t.hash       = $1
  AND t.created_at = $2;

-- @@ split @@

-- ============================================================================
-- C. Operations (appearance rows for this tx).
--    Inputs: $1 = transaction_id (from B), $2 = created_at (from A/B).
--    The appearance index is read-time-only (ADR 0033/0034 for events/invocations);
--    operations_appearances is the DB-side flat view of distinct op identities.
-- ============================================================================
SELECT
    oa.id                           AS appearance_id,
    op_type_name(oa.type)           AS type_name,
    oa.type                         AS type,
    src.account_id                  AS source_account,
    dst.account_id                  AS destination_account,
    sc.contract_id                  AS contract_id,
    oa.asset_code,
    iss.account_id                  AS asset_issuer,
    encode(oa.pool_id, 'hex')       AS pool_id_hex,
    oa.ledger_sequence,
    oa.created_at
    -- not in DB: per-op stroop amount
    --           — `operations_appearances.amount` is a fold count of
    --             collapsed identity-tuple duplicates (task 0163), NOT a
    --             value. Real per-op amounts are encoded inside
    --             envelope_xdr / result_meta_xdr and surfaced via the
    --             archive overlay below. ADR 0029.
    -- not in DB: raw operation parameters / arguments
    --           — encoded inside envelope_xdr operation entries. Archive
    --             overlay required for Advanced mode (frontend §6.4). ADR 0029.
    -- not in DB: per-operation return values
    --           — encoded inside result_meta_xdr per-operation result entries.
    --             Archive overlay required for Advanced mode. ADR 0029.
    -- Operation ordering: `appearance_id` (oa.id) is a global BIGSERIAL across
    --   all ledgers/partitions, NOT a within-tx index. The result-set order
    --   from `ORDER BY oa.id` is monotone with ingest order, which IS the
    --   operation application order within a tx (operations land sequentially
    --   during a single ingest). Frontend §6.4 Advanced mode "operation IDs"
    --   should display row-position within the result set (1..N), not oa.id.
FROM operations_appearances oa
LEFT JOIN accounts          src ON src.id = oa.source_id
LEFT JOIN accounts          dst ON dst.id = oa.destination_id
LEFT JOIN soroban_contracts sc  ON sc.id  = oa.contract_id
LEFT JOIN accounts          iss ON iss.id = oa.asset_issuer_id
WHERE oa.transaction_id = $1
  AND oa.created_at     = $2
ORDER BY oa.id;

-- @@ split @@

-- ============================================================================
-- D. Participants (accounts touched by this tx).
--    Inputs: $1 = transaction_id, $2 = created_at.
-- ============================================================================
SELECT
    a.account_id
FROM transaction_participants tp
JOIN accounts a ON a.id = tp.account_id
WHERE tp.transaction_id = $1
  AND tp.created_at     = $2
ORDER BY a.account_id;

-- @@ split @@

-- ============================================================================
-- E. Soroban event appearances (index only — full topics/data via archive).
--    Inputs: $1 = transaction_id, $2 = created_at.
-- ============================================================================
SELECT
    sc.contract_id,
    sea.ledger_sequence,
    sea.amount,
    sea.created_at,
    -- These two columns travel with the row purely as an archive bridge:
    -- the API uses (transaction_id, contract_id, ledger_sequence) to find
    -- the matching events inside the parsed transaction's events[] array.
    sea.contract_id                 AS contract_pk_for_archive_bridge,
    sea.transaction_id              AS transaction_id_for_archive_bridge
    -- not in DB: event_type, topics, data — Archive XDR (ADR 0029, ADR 0033).
    --           Bridge: (transaction_id, created_at, contract_id, ledger_sequence).
    --           The same envelope_xdr fetched for E3 statement B carries the
    --           events; the API filters them by contract_id within the tx.
FROM soroban_events_appearances sea
JOIN soroban_contracts sc ON sc.id = sea.contract_id
WHERE sea.transaction_id = $1
  AND sea.created_at     = $2
ORDER BY sea.ledger_sequence, sc.contract_id;

-- @@ split @@

-- ============================================================================
-- F. Soroban invocation appearances (per ADR 0034 + task 0183).
--    Inputs: $1 = transaction_id, $2 = created_at.
--
--    Coverage note (task 0183, 2026-04-30): rows now reflect the
--    **execution** call graph reconstructed from `fn_call`/`fn_return`
--    diagnostic events, not just the auth tree. Auth-less DeFi router
--    sub-calls (multi-hop swaps where the user signs only the outer
--    call) are populated; previously these tx had zero rows here.
--    Caller is split across `caller_id` (G/M account) and
--    `caller_contract_id` (C-prefix contract) — the latter is set when
--    the trio has no G/M caller in the depth-first emit order, i.e.
--    when the contract was reached only as a sub-invocation.
-- ============================================================================
SELECT
    sc.contract_id,
    caller.account_id          AS caller_account,
    caller_contract.contract_id AS caller_contract,
    sia.ledger_sequence,
    sia.amount,
    sia.created_at
    -- not in DB: function_name, args, return_value
    --           — Archive XDR. Bridge: (transaction_id, created_at, contract_id).
    --           ADR 0029, ADR 0034.
    -- not in DB: invocation_index / parent_invocation_id (call hierarchy)
    --           — must be reconstructed by parsing the envelope_xdr's
    --             invocation tree. The DB does not pre-compute the tree per
    --             ADR 0034. Frontend §6.4 Normal-mode "contract-to-contract
    --             hierarchy" requires the API to parse the full tree once and
    --             stitch it back into this row set.
FROM soroban_invocations_appearances sia
JOIN soroban_contracts sc                 ON sc.id     = sia.contract_id
LEFT JOIN accounts          caller          ON caller.id = sia.caller_id
LEFT JOIN soroban_contracts caller_contract ON caller_contract.id = sia.caller_contract_id
WHERE sia.transaction_id = $1
  AND sia.created_at     = $2
ORDER BY sia.ledger_sequence, sc.contract_id;
