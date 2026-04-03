-- Derived from: domain::soroban::SorobanContract (crates/domain/src/soroban.rs)
CREATE TABLE soroban_contracts (
    contract_id        VARCHAR(56) PRIMARY KEY,          -- SorobanContract.contract_id: String
    wasm_hash          VARCHAR(64),                      -- SorobanContract.wasm_hash: Option<String>
    deployer_account   VARCHAR(56),                      -- SorobanContract.deployer_account: Option<String>
    deployed_at_ledger BIGINT REFERENCES ledgers(sequence), -- SorobanContract.deployed_at_ledger: Option<i64>
    contract_type      VARCHAR(50),                      -- SorobanContract.contract_type: Option<String>
    is_sac             BOOLEAN NOT NULL DEFAULT FALSE,    -- SorobanContract.is_sac: Option<bool> (DEFAULT for upsert safety)
    metadata           JSONB,                            -- SorobanContract.metadata: Option<Value>
    -- DB-only: generated column excluded from domain struct
    search_vector      TSVECTOR GENERATED ALWAYS AS (
                           to_tsvector('english', coalesce(metadata->>'name', ''))
                       ) STORED
);

CREATE INDEX idx_contracts_type ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
