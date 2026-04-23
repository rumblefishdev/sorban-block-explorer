-- ADR 0027 + ADR 0030 + ADR 0033 + ADR 0034 — initial schema, step 4/7: Soroban activity time-series
-- Both tables partition on created_at and cascade from transactions via
-- composite FK (transaction_id, created_at).
--
-- Tables:
--   9.  soroban_events_appearances      (partitioned — contract-event appearance index per ADR 0033)
--   10. soroban_invocations_appearances (partitioned — contract-invocation appearance index per ADR 0034)

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

-- 10. soroban_invocations_appearances (ADR 0034)
--
-- Appearance index: one row per (contract, transaction, ledger) trio;
-- `amount` counts the invocation-tree nodes aggregated into that trio.
-- `caller_id` is the root-level caller of the trio, preserving the
-- staging `is_strkey_account` filter (G-accounts retained, C-contract
-- sub-invocation callers collapsed to NULL) — kept as an unindexed
-- payload column so that E11's `unique_callers` stat remains answerable
-- via COUNT(DISTINCT caller_id) without extra JOINs. All per-node
-- detail (function name, per-node index, successful flag, function
-- args, return value, depth) lives at read time in the public Stellar
-- archive and is re-expanded by the API through
-- xdr_parser::extract_invocations.
CREATE TABLE soroban_invocations_appearances (
    contract_id      BIGINT       NOT NULL REFERENCES soroban_contracts(id), -- ADR 0030
    transaction_id   BIGINT       NOT NULL,
    ledger_sequence  BIGINT       NOT NULL,
    caller_id        BIGINT       REFERENCES accounts(id),
    amount           INTEGER      NOT NULL,
    created_at       TIMESTAMPTZ  NOT NULL,
    PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at),
    FOREIGN KEY (transaction_id, created_at)
        REFERENCES transactions (id, created_at) ON DELETE CASCADE
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_sia_contract_ledger
    ON soroban_invocations_appearances (contract_id, ledger_sequence DESC);
CREATE INDEX idx_sia_transaction
    ON soroban_invocations_appearances (transaction_id);
