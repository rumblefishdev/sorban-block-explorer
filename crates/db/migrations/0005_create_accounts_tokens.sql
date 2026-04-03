-- Derived from: domain::account::Account (crates/domain/src/account.rs)
CREATE TABLE accounts (
    account_id        VARCHAR(56) PRIMARY KEY,          -- Account.account_id: String
    first_seen_ledger BIGINT NOT NULL,                  -- Account.first_seen_ledger: i64
    last_seen_ledger  BIGINT NOT NULL,                  -- Account.last_seen_ledger: i64 (watermark)
    sequence_number   BIGINT NOT NULL,                  -- Account.sequence_number: i64
    balances          JSONB NOT NULL DEFAULT '[]'::jsonb, -- Account.balances: serde_json::Value
    home_domain       VARCHAR(256)                      -- Account.home_domain: Option<String>
);

CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);

-- Derived from: domain::token::Token (crates/domain/src/token.rs)
CREATE TABLE tokens (
    id               SERIAL PRIMARY KEY,                -- Token.id: i32
    asset_type       VARCHAR(20) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban')), -- Token.asset_type: String
    asset_code       VARCHAR(12),                       -- Token.asset_code: Option<String>
    issuer_address   VARCHAR(56),                       -- Token.issuer_address: Option<String>
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id), -- Token.contract_id: Option<String>
    name             VARCHAR(256),                      -- Token.name: Option<String>
    total_supply     NUMERIC(28, 7),                    -- Token.total_supply: Option<String> (NUMERIC as String)
    holder_count     INTEGER,                           -- Token.holder_count: Option<i32>
    metadata         JSONB                              -- Token.metadata: Option<Value>
);

CREATE UNIQUE INDEX idx_tokens_classic ON tokens (asset_code, issuer_address) WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX idx_tokens_soroban ON tokens (contract_id) WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac ON tokens (contract_id) WHERE asset_type = 'sac';
CREATE INDEX idx_tokens_type ON tokens (asset_type);
