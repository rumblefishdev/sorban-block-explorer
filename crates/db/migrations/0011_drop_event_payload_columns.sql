-- Drop JSONB payload columns from soroban_events.
--
-- The topics and data columns stored full ScVal-decoded JSON for every
-- contract event (~560 KB per ledger, ~4,500 events/ledger). These columns
-- dominated both storage and insert time (~65% of persist pipeline).
--
-- Event payloads are deterministically re-derivable from the raw XDR files
-- on S3 (stellar-ledger-data bucket). The API will lazy-fetch and parse
-- them on demand using the thin index (contract_id, ledger_sequence,
-- event_index) that remains in this table.
--
-- Also drops the GIN index on topics which is no longer needed.

DROP INDEX IF EXISTS idx_events_topics;

ALTER TABLE soroban_events DROP COLUMN topics;
ALTER TABLE soroban_events DROP COLUMN data;
