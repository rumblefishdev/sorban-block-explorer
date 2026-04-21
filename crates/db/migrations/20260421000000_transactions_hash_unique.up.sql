-- Add UNIQUE (hash, created_at) to transactions.
--
-- ADR 0027 did not declare uniqueness on transactions.hash (the separate
-- transaction_hash_index.hash PK was the look-up path). The ADR 0027
-- write-path (task 0149) relies on `ON CONFLICT … DO UPDATE … RETURNING id`
-- to recover the surrogate id on replay in O(1); that requires a UNIQUE
-- constraint on the natural key. Partitioned tables require all partition
-- key columns to appear in UNIQUE constraints, so we use (hash, created_at)
-- — created_at is deterministic per ledger (parent close time), so replays
-- hit the same key and the upsert is a no-op on the row.

ALTER TABLE transactions
    ADD CONSTRAINT uq_transactions_hash_created_at UNIQUE (hash, created_at);
