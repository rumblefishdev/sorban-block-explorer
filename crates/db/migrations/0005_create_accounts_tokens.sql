CREATE TABLE accounts (
    account_id         VARCHAR(56) PRIMARY KEY,
    first_seen_ledger  BIGINT NOT NULL,
    last_seen_ledger   BIGINT NOT NULL,
    sequence_number    BIGINT NOT NULL,
    balances           JSONB NOT NULL DEFAULT '[]'::jsonb,
    home_domain        VARCHAR(256)
);

CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);

CREATE TABLE tokens (
    id               SERIAL PRIMARY KEY,
    asset_type       VARCHAR(20) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban')),
    asset_code       VARCHAR(12),
    issuer_address   VARCHAR(56),
    contract_id      VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name             VARCHAR(256),
    total_supply     NUMERIC,
    holder_count     INTEGER,
    metadata         JSONB
);

CREATE UNIQUE INDEX idx_tokens_classic ON tokens (asset_code, issuer_address) WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX idx_tokens_soroban ON tokens (contract_id) WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac ON tokens (contract_id) WHERE asset_type = 'sac';
CREATE INDEX idx_tokens_type ON tokens (asset_type);
