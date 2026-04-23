-- ADR 0027 + ADR 0030 + ADR 0033 — initial schema, step 4/7: Soroban activity time-series
-- Both tables partition on created_at and cascade from transactions via
-- composite FK (transaction_id, created_at).
--
-- Tables:
--   9.  soroban_events_appearances (partitioned — contract-event appearance index per ADR 0033)
--   10. soroban_invocations        (partitioned — caller / function / status)

-- 9. soroban_events_appearances (ADR 0033)
--
-- Pure appearance index: one row per (contract, transaction, ledger) trio;
-- `amount` counts the non-diagnostic contract events aggregated into that
-- trio. All parsed event detail (type, topics, data, per-event index,
-- transfer triple) lives at read time in the public Stellar archive and is
-- re-expanded by the API through xdr_parser::extract_events. Matches the
-- rewrite-in-place pattern of ADR 0030 / ADR 0031 — no production DB yet.
CREATE TABLE soroban_events_appearances (
    contract_id     BIGINT       NOT NULL REFERENCES soroban_contracts(id), -- ADR 0030
    transaction_id  BIGINT       NOT NULL,
    ledger_sequence BIGINT       NOT NULL,
    amount          BIGINT       NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_sea_contract_ledger
    ON soroban_events_appearances (contract_id, ledger_sequence DESC, created_at DESC);
CREATE INDEX idx_sea_transaction
    ON soroban_events_appearances (transaction_id, created_at DESC);

-- 10. soroban_invocations (ADR 0027 §10)
CREATE TABLE soroban_invocations (
    id               BIGSERIAL    NOT NULL,
    transaction_id   BIGINT       NOT NULL,
    contract_id      BIGINT       REFERENCES soroban_contracts(id), -- ADR 0030
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
