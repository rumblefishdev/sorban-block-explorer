-- Drop JSONB details column from operations.
--
-- The details column stored type-specific operation data as JSONB
-- (~175 KB per ledger). For INVOKE_HOST_FUNCTION operations this included
-- full ScVal-decoded functionArgs and returnValue which dominated size.
--
-- Operation details are deterministically re-derivable from the raw XDR
-- files on S3 (stellar-ledger-data bucket). The API will lazy-fetch and
-- parse them on demand using the thin index (transaction_id,
-- application_order, type) that remains in this table.
--
-- Also drops the GIN index on details which is no longer needed.

DROP INDEX IF EXISTS idx_operations_details;

ALTER TABLE operations DROP COLUMN details;
