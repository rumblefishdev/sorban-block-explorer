-- ADR 0027 + ADR 0031 — initial schema, step 6/7: liquidity pools
-- pool_id is the 32 B pool hash (BYTEA). Snapshots are partitioned on
-- created_at. LP positions are unpartitioned current-state.
--
-- Tables:
--   14. liquidity_pools
--   15. liquidity_pool_snapshots  (partitioned)
--   16. lp_positions
--
-- Also: attach the deferred operations.pool_id FK now that liquidity_pools exists.

-- 14. liquidity_pools (ADR 0027 §14 + ADR 0031)
-- `asset_*_type` SMALLINT carries the raw XDR AssetType
-- (0=native, 1=credit_alphanum4, 2=credit_alphanum12, 3=pool_share —
-- see asset_type_name() in 0008; Rust `AssetType`).
CREATE TABLE liquidity_pools (
    pool_id            BYTEA       PRIMARY KEY,
    asset_a_type       SMALLINT    NOT NULL, -- ADR 0031: AssetType (XDR)
    asset_a_code       VARCHAR(12),
    asset_a_issuer_id  BIGINT      REFERENCES accounts(id),
    asset_b_type       SMALLINT    NOT NULL, -- ADR 0031: AssetType (XDR)
    asset_b_code       VARCHAR(12),
    asset_b_issuer_id  BIGINT      REFERENCES accounts(id),
    fee_bps            INTEGER     NOT NULL,
    created_at_ledger  BIGINT      NOT NULL,
    CONSTRAINT ck_lp_pool_id_len        CHECK (octet_length(pool_id) = 32),
    CONSTRAINT ck_lp_asset_a_type_range CHECK (asset_a_type BETWEEN 0 AND 15),
    CONSTRAINT ck_lp_asset_b_type_range CHECK (asset_b_type BETWEEN 0 AND 15)
);
CREATE INDEX idx_pools_asset_a ON liquidity_pools (asset_a_code, asset_a_issuer_id);
CREATE INDEX idx_pools_asset_b ON liquidity_pools (asset_b_code, asset_b_issuer_id);

-- 15. liquidity_pool_snapshots (ADR 0027 §15)
CREATE TABLE liquidity_pool_snapshots (
    id              BIGSERIAL     NOT NULL,
    pool_id         BYTEA         NOT NULL REFERENCES liquidity_pools(pool_id),
    ledger_sequence BIGINT        NOT NULL,
    reserve_a       NUMERIC(28,7) NOT NULL,
    reserve_b       NUMERIC(28,7) NOT NULL,
    total_shares    NUMERIC(28,7) NOT NULL,
    tvl             NUMERIC(28,7),
    volume          NUMERIC(28,7),
    fee_revenue     NUMERIC(28,7),
    created_at      TIMESTAMPTZ   NOT NULL,
    PRIMARY KEY (id, created_at),
    CONSTRAINT ck_lps_pool_id_len CHECK (octet_length(pool_id) = 32)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_lps_pool ON liquidity_pool_snapshots (pool_id, created_at DESC);
CREATE INDEX idx_lps_tvl  ON liquidity_pool_snapshots (tvl DESC) WHERE tvl IS NOT NULL;

-- 16. lp_positions (ADR 0027 §16)
CREATE TABLE lp_positions (
    pool_id              BYTEA         NOT NULL REFERENCES liquidity_pools(pool_id),
    account_id           BIGINT        NOT NULL REFERENCES accounts(id),
    shares               NUMERIC(28,7) NOT NULL,
    first_deposit_ledger BIGINT        NOT NULL,
    last_updated_ledger  BIGINT        NOT NULL,
    PRIMARY KEY (pool_id, account_id),
    CONSTRAINT ck_lpp_pool_id_len CHECK (octet_length(pool_id) = 32)
);
CREATE INDEX idx_lpp_shares ON lp_positions (pool_id, shares DESC) WHERE shares > 0;

-- Deferred FK from migration 0003: operations.pool_id → liquidity_pools(pool_id)
ALTER TABLE operations
    ADD CONSTRAINT fk_ops_pool_id
    FOREIGN KEY (pool_id) REFERENCES liquidity_pools(pool_id);
