CREATE TABLE liquidity_pools (
    pool_id              VARCHAR(64) PRIMARY KEY,
    asset_a              JSONB NOT NULL,
    asset_b              JSONB NOT NULL,
    fee_bps              INTEGER NOT NULL,
    reserves             JSONB NOT NULL,
    total_shares         NUMERIC NOT NULL,
    tvl                  NUMERIC,
    created_at_ledger    BIGINT NOT NULL,
    last_updated_ledger  BIGINT NOT NULL
);

CREATE INDEX idx_pools_updated ON liquidity_pools (last_updated_ledger DESC);

CREATE TABLE liquidity_pool_snapshots (
    id               BIGSERIAL,
    pool_id          VARCHAR(64) NOT NULL REFERENCES liquidity_pools(pool_id),
    ledger_sequence  BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    reserves         JSONB NOT NULL,
    total_shares     NUMERIC NOT NULL,
    tvl              NUMERIC,
    volume           NUMERIC,
    fee_revenue      NUMERIC,
    PRIMARY KEY (id, created_at),
    UNIQUE (pool_id, ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_pool_snapshots_pool ON liquidity_pool_snapshots (pool_id, created_at DESC);

-- Initial monthly partitions (Apr-Jun 2026)
CREATE TABLE liquidity_pool_snapshots_y2026m04 PARTITION OF liquidity_pool_snapshots
    FOR VALUES FROM ('2026-04-01 00:00:00+00') TO ('2026-05-01 00:00:00+00');
CREATE TABLE liquidity_pool_snapshots_y2026m05 PARTITION OF liquidity_pool_snapshots
    FOR VALUES FROM ('2026-05-01 00:00:00+00') TO ('2026-06-01 00:00:00+00');
CREATE TABLE liquidity_pool_snapshots_y2026m06 PARTITION OF liquidity_pool_snapshots
    FOR VALUES FROM ('2026-06-01 00:00:00+00') TO ('2026-07-01 00:00:00+00');
CREATE TABLE liquidity_pool_snapshots_default PARTITION OF liquidity_pool_snapshots DEFAULT;

CREATE TABLE nfts (
    contract_id      VARCHAR(56) NOT NULL REFERENCES soroban_contracts(contract_id),
    token_id         VARCHAR(256) NOT NULL,
    collection_name  VARCHAR(256),
    owner_account    VARCHAR(56),
    name             VARCHAR(256),
    media_url        TEXT,
    metadata         JSONB,
    minted_at_ledger BIGINT,
    last_seen_ledger BIGINT NOT NULL,
    PRIMARY KEY (contract_id, token_id)
);

CREATE INDEX idx_nfts_owner ON nfts (owner_account);
CREATE INDEX idx_nfts_collection ON nfts (contract_id, collection_name);
