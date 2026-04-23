-- ADR 0027 + ADR 0030 + ADR 0031 + ADR 0033 — initial schema, step 5/7: assets and NFTs
-- Assets carry typed SEP-1 metadata columns (ADR 0023).
-- NFTs get a surrogate SERIAL PK; identity is still (contract_id, token_id)
-- where contract_id is the BIGINT FK into soroban_contracts.id (ADR 0030).
-- nft_ownership is partitioned and cascades from transactions.
--
-- Tables:
--   11. assets          (unpartitioned registry)
--   12. nfts            (unpartitioned)
--   13. nft_ownership   (partitioned history)

-- 11. assets (ADR 0027 §11 + ADR 0031 + ADR 0033)
-- `asset_type` SMALLINT is the Rust `TokenAssetType` enum
-- (0=native, 1=classic_credit, 2=sac, 3=soroban — label helper: token_asset_type_name).
-- Identity is enforced by ck_assets_identity (which columns must be NOT NULL
-- for each asset_type) plus per-asset_type partial UNIQUE indexes. The CHECK
-- closes the NULL-in-UNIQUE loophole — PostgreSQL treats NULLs as distinct,
-- so without it the partial uniques would admit duplicate logical assets.
CREATE TABLE assets (
    id              SERIAL        PRIMARY KEY,
    asset_type      SMALLINT      NOT NULL, -- ADR 0031: TokenAssetType
    asset_code      VARCHAR(12),
    issuer_id       BIGINT        REFERENCES accounts(id),
    contract_id     BIGINT        REFERENCES soroban_contracts(id), -- ADR 0030
    name            VARCHAR(256),
    total_supply    NUMERIC(28,7),
    holder_count    INTEGER,
    description     TEXT,
    icon_url        VARCHAR(1024),
    home_page       VARCHAR(256),
    CONSTRAINT ck_assets_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    CONSTRAINT ck_assets_identity CHECK (
        (asset_type = 0  -- native
            AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
     OR (asset_type = 1  -- classic_credit
            AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
     OR (asset_type = 2  -- sac
            AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
     OR (asset_type = 3  -- soroban
            AND issuer_id IS NULL      AND contract_id IS NOT NULL)
    )
);
CREATE UNIQUE INDEX uidx_assets_native        ON assets ((asset_type))
    WHERE asset_type = 0;              -- native
CREATE UNIQUE INDEX uidx_assets_classic_asset ON assets (asset_code, issuer_id)
    WHERE asset_type IN (1, 2);        -- classic_credit, sac
CREATE UNIQUE INDEX uidx_assets_soroban       ON assets (contract_id)
    WHERE asset_type IN (2, 3);        -- sac, soroban
CREATE INDEX idx_assets_type      ON assets (asset_type);
CREATE INDEX idx_assets_code_trgm ON assets USING GIN (asset_code gin_trgm_ops);

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
    event_type      SMALLINT     NOT NULL, -- ADR 0031: NftEventType (mint/transfer/burn)
    ledger_sequence BIGINT       NOT NULL,
    event_order     SMALLINT     NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (nft_id, created_at, ledger_sequence, event_order),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_nft_own_event_type_range CHECK (event_type BETWEEN 0 AND 15)
) PARTITION BY RANGE (created_at);
