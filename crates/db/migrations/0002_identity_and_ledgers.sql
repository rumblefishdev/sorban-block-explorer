-- ADR 0027 — initial schema, step 2/7: identity hubs and ledgers
-- Tables (FK-safe creation order):
--   1. ledgers               — chain-head anchor, referenced implicitly via ledger_sequence
--   2. accounts              — surrogate BIGSERIAL PK (ADR 0026), all account FKs target accounts.id
--   3. wasm_interface_metadata — ABI keyed by wasm_hash, referenced by soroban_contracts
--   4. soroban_contracts     — contract identity + metadata, referenced by operations/events/tokens/nfts

-- 1. ledgers
CREATE TABLE ledgers (
    sequence          BIGINT      PRIMARY KEY,
    hash              BYTEA       NOT NULL UNIQUE,
    closed_at         TIMESTAMPTZ NOT NULL,
    protocol_version  INTEGER     NOT NULL,
    transaction_count INTEGER     NOT NULL,
    base_fee          BIGINT      NOT NULL,
    CONSTRAINT ck_ledgers_hash_len CHECK (octet_length(hash) = 32)
);
CREATE INDEX idx_ledgers_closed_at ON ledgers (closed_at DESC);

-- 2. accounts (ADR 0026 — surrogate PK)
CREATE TABLE accounts (
    id                BIGSERIAL    PRIMARY KEY,
    account_id        VARCHAR(56)  NOT NULL UNIQUE,
    first_seen_ledger BIGINT       NOT NULL,
    last_seen_ledger  BIGINT       NOT NULL,
    sequence_number   BIGINT       NOT NULL,
    home_domain       VARCHAR(256)
);
CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);
CREATE INDEX idx_accounts_prefix    ON accounts (account_id text_pattern_ops);

-- 3. wasm_interface_metadata (ABI, referenced by soroban_contracts.wasm_hash)
CREATE TABLE wasm_interface_metadata (
    wasm_hash BYTEA PRIMARY KEY,
    metadata  JSONB NOT NULL,
    CONSTRAINT ck_wim_hash_len CHECK (octet_length(wasm_hash) = 32)
);

-- 4. soroban_contracts
CREATE TABLE soroban_contracts (
    contract_id             VARCHAR(56) PRIMARY KEY,
    wasm_hash               BYTEA       REFERENCES wasm_interface_metadata(wasm_hash),
    wasm_uploaded_at_ledger BIGINT,
    deployer_id             BIGINT      REFERENCES accounts(id),
    deployed_at_ledger      BIGINT,
    contract_type           VARCHAR(50),
    is_sac                  BOOLEAN     NOT NULL DEFAULT false,
    metadata                JSONB,
    search_vector           TSVECTOR GENERATED ALWAYS AS (
        to_tsvector('simple', COALESCE(metadata->>'name', '') || ' ' || contract_id)
    ) STORED,
    CONSTRAINT ck_sc_wasm_hash_len CHECK (wasm_hash IS NULL OR octet_length(wasm_hash) = 32)
);
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);
