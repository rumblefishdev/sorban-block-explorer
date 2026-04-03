-- Derived from: domain::pool::LiquidityPool (crates/domain/src/pool.rs)
CREATE TABLE liquidity_pools (
    pool_id              VARCHAR(64) PRIMARY KEY,       -- LiquidityPool.pool_id: String
    asset_a              JSONB NOT NULL,                -- LiquidityPool.asset_a: serde_json::Value
    asset_b              JSONB NOT NULL,                -- LiquidityPool.asset_b: serde_json::Value
    fee_bps              INTEGER NOT NULL,              -- LiquidityPool.fee_bps: i32
    reserves             JSONB NOT NULL,                -- LiquidityPool.reserves: serde_json::Value
    total_shares         NUMERIC NOT NULL,              -- LiquidityPool.total_shares: String (NUMERIC as String)
    tvl                  NUMERIC,                       -- LiquidityPool.tvl: Option<String>
    created_at_ledger    BIGINT NOT NULL,               -- LiquidityPool.created_at_ledger: i64
    last_updated_ledger  BIGINT NOT NULL                -- LiquidityPool.last_updated_ledger: i64 (watermark)
);

CREATE INDEX idx_pools_updated ON liquidity_pools (last_updated_ledger DESC);

-- Derived from: domain::pool::LiquidityPoolSnapshot (crates/domain/src/pool.rs)
CREATE TABLE liquidity_pool_snapshots (
    id               BIGSERIAL,                         -- LiquidityPoolSnapshot.id: i64
    pool_id          VARCHAR(64) NOT NULL REFERENCES liquidity_pools(pool_id), -- LiquidityPoolSnapshot.pool_id: String
    ledger_sequence  BIGINT NOT NULL,                   -- LiquidityPoolSnapshot.ledger_sequence: i64
    created_at       TIMESTAMPTZ NOT NULL,              -- LiquidityPoolSnapshot.created_at: DateTime<Utc>
    reserves         JSONB NOT NULL,                    -- LiquidityPoolSnapshot.reserves: serde_json::Value
    total_shares     NUMERIC NOT NULL,                  -- LiquidityPoolSnapshot.total_shares: String (NUMERIC as String)
    tvl              NUMERIC,                           -- LiquidityPoolSnapshot.tvl: Option<String>
    volume           NUMERIC,                           -- LiquidityPoolSnapshot.volume: Option<String>
    fee_revenue      NUMERIC,                           -- LiquidityPoolSnapshot.fee_revenue: Option<String>
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

-- Derived from: domain::nft::Nft (crates/domain/src/nft.rs)
CREATE TABLE nfts (
    contract_id      VARCHAR(56) NOT NULL REFERENCES soroban_contracts(contract_id), -- Nft.contract_id: String
    token_id         VARCHAR(256) NOT NULL,             -- Nft.token_id: String
    collection_name  VARCHAR(256),                      -- Nft.collection_name: Option<String>
    owner_account    VARCHAR(56),                       -- Nft.owner_account: Option<String>
    name             VARCHAR(256),                      -- Nft.name: Option<String>
    media_url        TEXT,                              -- Nft.media_url: Option<String>
    metadata         JSONB,                             -- Nft.metadata: Option<Value>
    minted_at_ledger BIGINT,                            -- Nft.minted_at_ledger: Option<i64>
    last_seen_ledger BIGINT NOT NULL,                   -- Nft.last_seen_ledger: i64 (watermark)
    PRIMARY KEY (contract_id, token_id)
);

CREATE INDEX idx_nfts_owner ON nfts (owner_account);
CREATE INDEX idx_nfts_collection ON nfts (contract_id, collection_name);
