CREATE TABLE soroban_contracts (
    contract_id        VARCHAR(56) PRIMARY KEY,
    wasm_hash          VARCHAR(64),
    deployer_account   VARCHAR(56),
    deployed_at_ledger BIGINT REFERENCES ledgers(sequence),
    contract_type      VARCHAR(50),
    is_sac             BOOLEAN DEFAULT FALSE,
    metadata           JSONB,
    search_vector      TSVECTOR GENERATED ALWAYS AS (
                           to_tsvector('english', coalesce(metadata->>'name', ''))
                       ) STORED
);

CREATE INDEX idx_contracts_type ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
