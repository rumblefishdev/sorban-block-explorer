-- Drop raw XDR columns from the transactions table.
--
-- These columns stored full base64-encoded XDR blobs (envelope, result,
-- result metadata) which are already parsed into dedicated tables
-- (operations, soroban_events, soroban_invocations, etc.).
-- The raw XDR is also retained on S3 (stellar-ledger-data bucket)
-- for reprocessing if ever needed.
--
-- Removing these columns significantly reduces database storage
-- requirements — they dominated table size at ~60-80% of total.

ALTER TABLE transactions DROP COLUMN envelope_xdr;
ALTER TABLE transactions DROP COLUMN result_xdr;
ALTER TABLE transactions DROP COLUMN result_meta_xdr;
