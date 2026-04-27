-- Endpoint:     GET /contracts/:contract_id/events
-- Purpose:      Paginated list of recent events emitted by a contract.
--               Default ordering: most recent first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.10
-- Schema:       ADR 0037
-- Data sources: DB index + Archive XDR.
--               DB returns: appearance rows (one per
--                  (contract, tx, ledger_sequence) tuple) + the
--                  `(ledger_sequence, transaction_id, created_at)` triple
--                  the API uses to fan out to the archive.
--               Archive returns: full event topics + data + event_type
--                  via the .xdr.zst for each ledger
--                  (ADR 0029, ADR 0033 — appearance index is read-time-only).
-- Inputs:
--   $1  :contract_strkey       VARCHAR(56)  C-form contract ID
--   $2  :limit                 INT          page size
--   $3  :cursor_ledger         BIGINT       NULL on first page
--   $4  :cursor_tx_id          BIGINT       NULL on first page
--   $5  :cursor_created_at     TIMESTAMPTZ  NULL on first page
-- Indexes:      soroban_contracts UNIQUE (contract_id),
--               idx_sea_contract_ledger
--                  ON (contract_id, ledger_sequence DESC, created_at DESC),
--               soroban_events_appearances PK
--                  (contract_id, transaction_id, ledger_sequence, created_at),
--               transactions PK (id, created_at).
-- Notes:
--   • Per ADR 0033 this table is the appearance index only. The DB knows
--     "contract X emitted N events in tx T at ledger L" — it does NOT know
--     the topics or data. The API must overlay those from the archive.
--   • The cursor matches the ordering hierarchy of `idx_sea_contract_ledger`:
--     `(contract_id =, ledger_sequence DESC, created_at DESC)`. Adding
--     transaction_id to the cursor breaks ties when two txs in the same
--     ledger emit events for the same contract.
--   • We surface `tx_hash` and `successful` from the join so the table view
--     can render them without a second round-trip.

WITH ct AS (
    SELECT id FROM soroban_contracts WHERE contract_id = $1
)
SELECT
    sea.ledger_sequence,
    sea.transaction_id,
    encode(t.hash, 'hex')   AS transaction_hash_hex,
    t.successful,
    sea.amount,
    sea.created_at
    -- not in DB: event_type, topics, data — Archive XDR (ADR 0029, ADR 0033).
    --   Bridge to archive: (ledger_sequence, transaction_id, created_at)
    --   identifies the transaction's `.xdr.zst` slice; the contract_id (input
    --   $1) plus the order of events within the tx-meta selects the right
    --   event entries from that decoded payload. The API must filter the
    --   parsed events[] by this contract_id; the DB cannot tell which
    --   events in the meta belong to this contract beyond the appearance.
FROM ct
JOIN soroban_events_appearances sea
       ON sea.contract_id = ct.id
JOIN transactions t
       ON t.id         = sea.transaction_id
      AND t.created_at = sea.created_at
WHERE
    ($3::bigint IS NULL OR (sea.ledger_sequence, sea.transaction_id, sea.created_at) < ($3, $4, $5))
ORDER BY sea.ledger_sequence DESC, sea.transaction_id DESC, sea.created_at DESC
LIMIT $2;
