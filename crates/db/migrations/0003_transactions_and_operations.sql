-- ADR 0027 + ADR 0030 + ADR 0031 — initial schema, step 3/7: transactions, operations, and participants
-- Partitioned tables use composite PK (id, created_at) with the partition key
-- included per Postgres rules. Monthly partitions are provisioned by the
-- partition-management Lambda (see task 0139 and crates/db-partition-mgmt).
--
-- Tables:
--   3. transactions              (partitioned on created_at)
--   4. transaction_hash_index    (unpartitioned — hash lookup)
--   5. operations                (partitioned on created_at)
--   6. transaction_participants  (partitioned on created_at)
--
-- Note: operations.pool_id FK → liquidity_pools(pool_id) is attached in
-- migration 0006 once liquidity_pools exists.

-- 3. transactions (ADR 0027 §3)
CREATE TABLE transactions (
    id                BIGSERIAL   NOT NULL,
    hash              BYTEA       NOT NULL,
    ledger_sequence   BIGINT      NOT NULL,
    application_order SMALLINT    NOT NULL,
    source_id         BIGINT      NOT NULL REFERENCES accounts(id),
    fee_charged       BIGINT      NOT NULL,
    inner_tx_hash     BYTEA,
    successful        BOOLEAN     NOT NULL,
    operation_count   SMALLINT    NOT NULL,
    has_soroban       BOOLEAN     NOT NULL DEFAULT false,
    parse_error       BOOLEAN     NOT NULL DEFAULT false,
    created_at        TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (id, created_at),
    CONSTRAINT ck_transactions_hash_len       CHECK (octet_length(hash) = 32),
    CONSTRAINT ck_transactions_inner_hash_len CHECK (inner_tx_hash IS NULL OR octet_length(inner_tx_hash) = 32)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tx_source_created ON transactions (source_id, created_at DESC);
CREATE INDEX idx_tx_ledger         ON transactions (ledger_sequence);
CREATE INDEX idx_tx_has_soroban    ON transactions (created_at DESC) WHERE has_soroban;

-- 4. transaction_hash_index (ADR 0027 §4)
CREATE TABLE transaction_hash_index (
    hash            BYTEA       PRIMARY KEY,
    ledger_sequence BIGINT      NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    CONSTRAINT ck_thi_hash_len CHECK (octet_length(hash) = 32)
);

-- 5. operations (ADR 0027 §5)
-- pool_id FK added in 0006 after liquidity_pools exists.
CREATE TABLE operations (
    id                BIGSERIAL    NOT NULL,
    transaction_id    BIGINT       NOT NULL,
    application_order SMALLINT     NOT NULL,
    type              SMALLINT     NOT NULL, -- ADR 0031 (Rust OperationType enum; see op_type_name() in 0008)
    source_id         BIGINT       REFERENCES accounts(id),
    destination_id    BIGINT       REFERENCES accounts(id),
    contract_id       BIGINT       REFERENCES soroban_contracts(id), -- ADR 0030
    asset_code        VARCHAR(12),
    asset_issuer_id   BIGINT       REFERENCES accounts(id),
    pool_id           BYTEA,
    transfer_amount   NUMERIC(28,7),
    ledger_sequence   BIGINT       NOT NULL,
    created_at        TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_ops_pool_id_len   CHECK (pool_id IS NULL OR octet_length(pool_id) = 32),
    CONSTRAINT ck_ops_type_range    CHECK (type BETWEEN 0 AND 127)  -- ADR 0031: room beyond Protocol 21's 27 variants
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_ops_tx          ON operations (transaction_id);
CREATE INDEX idx_ops_type        ON operations (type, created_at DESC);
CREATE INDEX idx_ops_contract    ON operations (contract_id, created_at DESC)
    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_ops_asset       ON operations (asset_code, asset_issuer_id, created_at DESC)
    WHERE asset_code IS NOT NULL;
CREATE INDEX idx_ops_pool        ON operations (pool_id, created_at DESC)
    WHERE pool_id IS NOT NULL;
CREATE INDEX idx_ops_destination ON operations (destination_id, created_at DESC)
    WHERE destination_id IS NOT NULL;

-- 6. transaction_participants (ADR 0027 §6)
CREATE TABLE transaction_participants (
    transaction_id BIGINT      NOT NULL,
    account_id     BIGINT      NOT NULL REFERENCES accounts(id),
    created_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, created_at, transaction_id),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_tp_tx ON transaction_participants (transaction_id);
