-- ADR 0027 + ADR 0030 + ADR 0031 — initial schema, step 3/7: transactions, operations, and participants
-- Partitioned tables use composite PK (id, created_at) with the partition key
-- included per Postgres rules. Monthly partitions are provisioned by the
-- partition-management Lambda (see task 0139 and crates/db-partition-mgmt).
--
-- Tables:
--   3. transactions              (partitioned on created_at)
--   4. transaction_hash_index    (unpartitioned — hash lookup)
--   5. operations_appearances    (partitioned on created_at — task 0163)
--   6. transaction_participants  (partitioned on created_at)
--
-- Note: operations_appearances.pool_id FK → liquidity_pools(pool_id) is
-- attached in migration 0006 once liquidity_pools exists.

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

-- 5. operations_appearances (ADR 0027 §5, task 0163)
--
-- Appearance index: one row per distinct operation-shape per transaction;
-- `amount` counts how many operations of that shape were folded into the row.
-- Mirrors `soroban_events_appearances` / `soroban_invocations_appearances`
-- (ADR 0033 / ADR 0034). Per-op detail (transfer amount, application order,
-- memo, claimants, function args, predicates, …) lives in the XDR archive and
-- is re-materialised by the API via `xdr_parser::extract_operations` — the DB
-- only records *that* an operation of a given identity occurred.
--
-- Replay idempotency: wide natural-key UNIQUE `uq_ops_app_identity` with
-- NULLS NOT DISTINCT (PG 15+) — NULL-heavy shapes (e.g. type-14 claimable
-- balance create, source inherited from tx) collapse to a single row under
-- ON CONFLICT DO NOTHING on re-ingest.
--
-- pool_id FK added in 0006 after liquidity_pools exists.
CREATE TABLE operations_appearances (
    id                BIGSERIAL    NOT NULL,
    transaction_id    BIGINT       NOT NULL,
    type              SMALLINT     NOT NULL, -- ADR 0031 (Rust OperationType enum; label helper: op_type_name)
    source_id         BIGINT       REFERENCES accounts(id),
    destination_id    BIGINT       REFERENCES accounts(id),
    contract_id       BIGINT       REFERENCES soroban_contracts(id), -- ADR 0030
    asset_code        VARCHAR(12),
    asset_issuer_id   BIGINT       REFERENCES accounts(id),
    pool_id           BYTEA,
    amount            BIGINT       NOT NULL,
    ledger_sequence   BIGINT       NOT NULL,
    created_at        TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE,
    CONSTRAINT ck_ops_app_pool_id_len CHECK (pool_id IS NULL OR octet_length(pool_id) = 32),
    CONSTRAINT ck_ops_app_type_range  CHECK (type BETWEEN 0 AND 127),  -- ADR 0031: room beyond Protocol 21's 27 variants
    CONSTRAINT ck_ops_app_amount_pos  CHECK (amount > 0),
    CONSTRAINT uq_ops_app_identity    UNIQUE NULLS NOT DISTINCT
        (transaction_id, type, source_id, destination_id,
         contract_id, asset_code, asset_issuer_id, pool_id,
         ledger_sequence, created_at)
) PARTITION BY RANGE (created_at);

-- `WHERE transaction_id = X` is served by the leftmost prefix of
-- `uq_ops_app_identity` (starts with `transaction_id, type, ...`). A dedicated
-- narrower index would be ~4× smaller and marginally faster at scale, but
-- the single-endpoint cost of the wide UNIQUE prefix scan is acceptable.
-- Add `idx_ops_app_tx (transaction_id)` later (CONCURRENTLY per partition)
-- if production telemetry shows it's needed — the decision is reversible.
CREATE INDEX idx_ops_app_type        ON operations_appearances (type, created_at DESC);
CREATE INDEX idx_ops_app_contract    ON operations_appearances (contract_id, created_at DESC)
    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_ops_app_asset       ON operations_appearances (asset_code, asset_issuer_id, created_at DESC)
    WHERE asset_code IS NOT NULL;
CREATE INDEX idx_ops_app_pool        ON operations_appearances (pool_id, created_at DESC)
    WHERE pool_id IS NOT NULL;
CREATE INDEX idx_ops_app_source      ON operations_appearances (source_id, created_at DESC)
    WHERE source_id IS NOT NULL;
CREATE INDEX idx_ops_app_destination ON operations_appearances (destination_id, created_at DESC)
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
