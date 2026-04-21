-- ADR 0027 + ADR 0030 — initial schema, step 5/7: tokens and NFTs
-- Tokens carry typed SEP-1 metadata columns (ADR 0023).
-- NFTs get a surrogate SERIAL PK; identity is still (contract_id, token_id)
-- where contract_id is the BIGINT FK into soroban_contracts.id (ADR 0030).
-- nft_ownership is partitioned and cascades from transactions.
--
-- Tables:
--   11. tokens          (unpartitioned registry)
--   12. nfts            (unpartitioned)
--   13. nft_ownership   (partitioned history)

-- 11. tokens (ADR 0027 §11)
-- Identity is enforced by ck_tokens_identity (which columns must be NOT NULL
-- for each asset_type) plus per-asset_type partial UNIQUE indexes. The CHECK
-- closes the NULL-in-UNIQUE loophole — PostgreSQL treats NULLs as distinct,
-- so without it the partial uniques would admit duplicate logical tokens.
CREATE TABLE tokens (
    id              SERIAL        PRIMARY KEY,
    asset_type      VARCHAR(20)   NOT NULL,
    asset_code      VARCHAR(12),
    issuer_id       BIGINT        REFERENCES accounts(id),
    contract_id     BIGINT        REFERENCES soroban_contracts(id), -- ADR 0030
    name            VARCHAR(256),
    total_supply    NUMERIC(28,7),
    holder_count    INTEGER,
    description     TEXT,
    icon_url        VARCHAR(1024),
    home_page       VARCHAR(256),
    CONSTRAINT ck_tokens_asset_type CHECK (asset_type IN ('native', 'classic', 'sac', 'soroban')),
    CONSTRAINT ck_tokens_identity CHECK (
        (asset_type = 'native'
            AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
     OR (asset_type = 'classic'
            AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
     OR (asset_type = 'sac'
            AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
     OR (asset_type = 'soroban'
            AND issuer_id IS NULL      AND contract_id IS NOT NULL)
    )
);
CREATE UNIQUE INDEX uidx_tokens_native        ON tokens ((asset_type))
    WHERE asset_type = 'native';
CREATE UNIQUE INDEX uidx_tokens_classic_asset ON tokens (asset_code, issuer_id)
    WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX uidx_tokens_soroban       ON tokens (contract_id)
    WHERE asset_type IN ('soroban', 'sac');
CREATE INDEX idx_tokens_type      ON tokens (asset_type);
CREATE INDEX idx_tokens_code_trgm ON tokens USING GIN (asset_code gin_trgm_ops);

-- 12. nfts (ADR 0027 §12)
CREATE TABLE nfts (
    id                   SERIAL       PRIMARY KEY,
    contract_id          BIGINT       NOT NULL REFERENCES soroban_contracts(id), -- ADR 0030
    token_id             VARCHAR(256) NOT NULL,
    collection_name      VARCHAR(256),
    name                 VARCHAR(256),
    media_url            TEXT,
    metadata             JSONB,
    minted_at_ledger     BIGINT,
    current_owner_id     BIGINT       REFERENCES accounts(id),
    current_owner_ledger BIGINT,
    UNIQUE (contract_id, token_id)
);
CREATE INDEX idx_nfts_collection ON nfts (collection_name);
CREATE INDEX idx_nfts_owner      ON nfts (current_owner_id);
CREATE INDEX idx_nfts_name_trgm  ON nfts USING GIN (name gin_trgm_ops);

-- 13. nft_ownership (ADR 0027 §13)
CREATE TABLE nft_ownership (
    nft_id          INTEGER      NOT NULL REFERENCES nfts(id) ON DELETE CASCADE,
    transaction_id  BIGINT       NOT NULL,
    owner_id        BIGINT       REFERENCES accounts(id),
    event_type      VARCHAR(20)  NOT NULL,
    ledger_sequence BIGINT       NOT NULL,
    event_order     SMALLINT     NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (nft_id, created_at, ledger_sequence, event_order),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);
