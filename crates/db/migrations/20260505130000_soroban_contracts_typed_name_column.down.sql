-- Reverse of 20260505130000_soroban_contracts_typed_name_column.up.sql.
-- Restores the `metadata JSONB` column and rewrites the `search_vector`
-- generated expression to read `metadata->>'name'`. The data round-trip
-- (`UPDATE … SET metadata = jsonb_build_object('name', name)`) preserves
-- previously-extracted names so up/down/up cycles on a populated DB do
-- not lose data — important for migration test scenarios that exercise
-- the round-trip explicitly (task 0156 Step 10 idempotency test).

DROP INDEX idx_contracts_search;

ALTER TABLE soroban_contracts DROP COLUMN search_vector;

ALTER TABLE soroban_contracts ADD COLUMN metadata JSONB;

UPDATE soroban_contracts
   SET metadata = jsonb_build_object('name', name)
 WHERE name IS NOT NULL;

ALTER TABLE soroban_contracts DROP COLUMN name;

ALTER TABLE soroban_contracts
    ADD COLUMN search_vector TSVECTOR
    GENERATED ALWAYS AS (
        to_tsvector('simple', COALESCE(metadata->>'name', '') || ' ' || contract_id)
    ) STORED;

CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
