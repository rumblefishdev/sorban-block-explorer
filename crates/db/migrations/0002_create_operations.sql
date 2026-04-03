-- Derived from: domain::operation::Operation (crates/domain/src/operation.rs)
CREATE TABLE operations (
    id                BIGSERIAL,                       -- Operation.id: i64
    transaction_id    BIGINT NOT NULL,                 -- Operation.transaction_id: i64
    application_order SMALLINT NOT NULL,               -- Operation.application_order: i16
    source_account    VARCHAR(56) NOT NULL,            -- Operation.source_account: String
    type              VARCHAR(50) NOT NULL,            -- Operation.op_type: String (serde rename "type")
    details           JSONB NOT NULL,                  -- Operation.details: serde_json::Value
    PRIMARY KEY (id, transaction_id),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE
) PARTITION BY RANGE (transaction_id);

CREATE INDEX idx_operations_tx ON operations (transaction_id);
CREATE INDEX idx_operations_source ON operations (source_account);
CREATE INDEX idx_operations_details ON operations USING GIN (details);

CREATE TABLE operations_p0 PARTITION OF operations
    FOR VALUES FROM (0) TO (10000000);

CREATE TABLE operations_default PARTITION OF operations
    DEFAULT;
