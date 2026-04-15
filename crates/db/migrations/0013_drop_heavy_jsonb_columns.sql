-- Drop remaining heavy JSONB columns from transactions and soroban_invocations.
--
-- transactions.operation_tree (~422 bytes avg per row) stored a pre-computed
-- Soroban invocation call tree. This was the only UPDATE-able column in
-- transactions (step 6 in persist pipeline). Removing it eliminates both
-- the storage cost and the UPDATE round-trip.
--
-- soroban_invocations.function_args (~164 bytes avg) and return_value
-- (~48 bytes avg) stored ScVal-decoded JSON payloads.
--
-- All data is deterministically re-derivable from raw XDR files on S3
-- and will be served via lazy-fetch from a parsed JSON cache on S3.

ALTER TABLE transactions DROP COLUMN operation_tree;

ALTER TABLE soroban_invocations DROP COLUMN function_args;
ALTER TABLE soroban_invocations DROP COLUMN return_value;
