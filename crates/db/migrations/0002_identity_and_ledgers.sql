-- ADR 0027 + ADR 0030 + ADR 0031 — initial schema, step 2/7: identity hubs and ledgers
-- Tables (FK-safe creation order):
--   1. ledgers               — chain-head anchor, referenced implicitly via ledger_sequence
--   2. accounts              — surrogate BIGSERIAL PK (ADR 0026), all account FKs target accounts.id
--   3. wasm_interface_metadata — ABI keyed by wasm_hash, referenced by soroban_contracts
--   4. soroban_contracts     — surrogate BIGSERIAL PK (ADR 0030); contract_id VARCHAR(56)
--      stays as a UNIQUE natural key for StrKey lookup, display, and E22 search.
--      All FK columns on operations/events/invocations/assets/nfts target
--      soroban_contracts.id (BIGINT), not contract_id (VARCHAR).
--      contract_type is SMALLINT (ADR 0031) — Rust `ContractType` enum is the
--      source of truth for the mapping; see crates/domain/src/enums/contract_type.rs.
--      Nullable because the two-pass upsert in persist/write.rs registers bare
--      StrKey refs before the deployment meta is seen — those rows start NULL
--      and get filled in when the deploy meta lands.

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

-- Task 0160: XLM-SAC issuer sentinel — all-zero Ed25519 StrKey. Not a real
-- Stellar account; seeded so `assets.issuer_id` FK resolves for the
-- synthetic XLM-SAC row (`detect_assets` applies this StrKey when a SAC
-- wraps Asset::Native and no classic issuer exists). Matches the
-- `XLM_SAC_ISSUER_SENTINEL` const in crates/xdr-parser/src/state.rs —
-- drift is caught by an xdr-parser unit test comparing to the runtime
-- `AccountId(Uint256([0; 32])).to_string()` value.
INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number)
VALUES ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF', 0, 0, 0);

-- 3. wasm_interface_metadata (ABI, referenced by soroban_contracts.wasm_hash)
CREATE TABLE wasm_interface_metadata (
    wasm_hash BYTEA PRIMARY KEY,
    metadata  JSONB NOT NULL,
    CONSTRAINT ck_wim_hash_len CHECK (octet_length(wasm_hash) = 32)
);

-- 4. soroban_contracts (ADR 0030 — surrogate PK)
CREATE TABLE soroban_contracts (
    id                      BIGSERIAL   PRIMARY KEY,
    contract_id             VARCHAR(56) NOT NULL UNIQUE,
    wasm_hash               BYTEA       REFERENCES wasm_interface_metadata(wasm_hash),
    wasm_uploaded_at_ledger BIGINT,
    deployer_id             BIGINT      REFERENCES accounts(id),
    deployed_at_ledger      BIGINT,
    contract_type           SMALLINT,                       -- ADR 0031 (nullable; filled on deploy observation)
    is_sac                  BOOLEAN     NOT NULL DEFAULT false,
    metadata                JSONB,
    search_vector           TSVECTOR GENERATED ALWAYS AS (
        to_tsvector('simple', COALESCE(metadata->>'name', '') || ' ' || contract_id)
    ) STORED,
    CONSTRAINT ck_sc_wasm_hash_len     CHECK (wasm_hash IS NULL OR octet_length(wasm_hash) = 32),
    CONSTRAINT ck_sc_contract_type_range CHECK (contract_type IS NULL OR contract_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);
