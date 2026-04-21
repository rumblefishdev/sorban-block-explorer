ALTER TABLE transactions
    DROP CONSTRAINT IF EXISTS uq_transactions_hash_created_at;
