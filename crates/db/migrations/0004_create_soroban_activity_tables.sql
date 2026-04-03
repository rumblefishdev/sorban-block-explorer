-- Derived from: domain::soroban::SorobanInvocation (crates/domain/src/soroban.rs)
CREATE TABLE soroban_invocations (
    id               BIGSERIAL,                         -- SorobanInvocation.id: i64
    transaction_id   BIGINT NOT NULL,                   -- SorobanInvocation.transaction_id: i64
    contract_id      VARCHAR(56),                       -- SorobanInvocation.contract_id: Option<String>
    caller_account   VARCHAR(56),                       -- SorobanInvocation.caller_account: Option<String>
    function_name    VARCHAR(100) NOT NULL,             -- SorobanInvocation.function_name: String
    function_args    JSONB,                             -- SorobanInvocation.function_args: Option<Value>
    return_value     JSONB,                             -- SorobanInvocation.return_value: Option<Value>
    successful       BOOLEAN NOT NULL,                  -- SorobanInvocation.successful: bool
    ledger_sequence  BIGINT NOT NULL,                   -- SorobanInvocation.ledger_sequence: i64
    created_at       TIMESTAMPTZ NOT NULL,              -- SorobanInvocation.created_at: DateTime<Utc>
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE,
    FOREIGN KEY (contract_id) REFERENCES soroban_contracts(contract_id)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_invocations_contract ON soroban_invocations (contract_id, created_at DESC);
CREATE INDEX idx_invocations_function ON soroban_invocations (contract_id, function_name);
CREATE INDEX idx_invocations_tx ON soroban_invocations (transaction_id);

-- Derived from: domain::soroban::SorobanEvent (crates/domain/src/soroban.rs)
CREATE TABLE soroban_events (
    id               BIGSERIAL,                         -- SorobanEvent.id: i64
    transaction_id   BIGINT NOT NULL,                   -- SorobanEvent.transaction_id: i64
    contract_id      VARCHAR(56),                       -- SorobanEvent.contract_id: Option<String>
    event_type       VARCHAR(20) NOT NULL,              -- SorobanEvent.event_type: String
    topics           JSONB NOT NULL,                    -- SorobanEvent.topics: serde_json::Value
    data             JSONB NOT NULL,                    -- SorobanEvent.data: serde_json::Value
    ledger_sequence  BIGINT NOT NULL,                   -- SorobanEvent.ledger_sequence: i64
    created_at       TIMESTAMPTZ NOT NULL,              -- SorobanEvent.created_at: DateTime<Utc>
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE,
    FOREIGN KEY (contract_id) REFERENCES soroban_contracts(contract_id)
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_events_contract ON soroban_events (contract_id, created_at DESC);
CREATE INDEX idx_events_topics ON soroban_events USING GIN (topics);
CREATE INDEX idx_events_tx ON soroban_events (transaction_id);

-- Initial monthly partitions for soroban_invocations (Apr-Jun 2026)
CREATE TABLE soroban_invocations_y2026m04 PARTITION OF soroban_invocations
    FOR VALUES FROM ('2026-04-01 00:00:00+00') TO ('2026-05-01 00:00:00+00');
CREATE TABLE soroban_invocations_y2026m05 PARTITION OF soroban_invocations
    FOR VALUES FROM ('2026-05-01 00:00:00+00') TO ('2026-06-01 00:00:00+00');
CREATE TABLE soroban_invocations_y2026m06 PARTITION OF soroban_invocations
    FOR VALUES FROM ('2026-06-01 00:00:00+00') TO ('2026-07-01 00:00:00+00');

CREATE TABLE soroban_invocations_default PARTITION OF soroban_invocations
    DEFAULT;

-- Initial monthly partitions for soroban_events (Apr-Jun 2026)
CREATE TABLE soroban_events_y2026m04 PARTITION OF soroban_events
    FOR VALUES FROM ('2026-04-01 00:00:00+00') TO ('2026-05-01 00:00:00+00');
CREATE TABLE soroban_events_y2026m05 PARTITION OF soroban_events
    FOR VALUES FROM ('2026-05-01 00:00:00+00') TO ('2026-06-01 00:00:00+00');
CREATE TABLE soroban_events_y2026m06 PARTITION OF soroban_events
    FOR VALUES FROM ('2026-06-01 00:00:00+00') TO ('2026-07-01 00:00:00+00');

CREATE TABLE soroban_events_default PARTITION OF soroban_events
    DEFAULT;
