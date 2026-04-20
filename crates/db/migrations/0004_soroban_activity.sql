-- ADR 0027 — initial schema, step 4/7: Soroban activity time-series
-- Both tables partition on created_at and cascade from transactions via
-- composite FK (transaction_id, created_at).
--
-- Tables:
--   9.  soroban_events       (partitioned — typed transfer prefix)
--   10. soroban_invocations  (partitioned — caller / function / status)

-- 9. soroban_events (ADR 0027 §9)
CREATE TABLE soroban_events (
    id               BIGSERIAL    NOT NULL,
    transaction_id   BIGINT       NOT NULL,
    contract_id      VARCHAR(56)  REFERENCES soroban_contracts(contract_id),
    event_type       VARCHAR(20)  NOT NULL,
    topic0           TEXT,
    event_index      SMALLINT     NOT NULL,
    transfer_from_id BIGINT       REFERENCES accounts(id),
    transfer_to_id   BIGINT       REFERENCES accounts(id),
    transfer_amount  NUMERIC(39,0),
    ledger_sequence  BIGINT       NOT NULL,
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_events_contract      ON soroban_events (contract_id, created_at DESC);
CREATE INDEX idx_events_transfer_from ON soroban_events (transfer_from_id, created_at DESC)
    WHERE transfer_from_id IS NOT NULL;
CREATE INDEX idx_events_transfer_to   ON soroban_events (transfer_to_id, created_at DESC)
    WHERE transfer_to_id IS NOT NULL;

-- 10. soroban_invocations (ADR 0027 §10)
CREATE TABLE soroban_invocations (
    id               BIGSERIAL    NOT NULL,
    transaction_id   BIGINT       NOT NULL,
    contract_id      VARCHAR(56)  REFERENCES soroban_contracts(contract_id),
    caller_id        BIGINT       REFERENCES accounts(id),
    function_name    VARCHAR(100) NOT NULL,
    successful       BOOLEAN      NOT NULL,
    invocation_index SMALLINT     NOT NULL,
    ledger_sequence  BIGINT       NOT NULL,
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_inv_contract ON soroban_invocations (contract_id, created_at DESC);
CREATE INDEX idx_inv_caller   ON soroban_invocations (caller_id, created_at DESC);
