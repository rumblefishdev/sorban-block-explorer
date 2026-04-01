-- Migration: create operations table (task 0017)
-- Plain SQL — Drizzle schema DSL has no PARTITION BY support; run via psql or sqlx migrate

CREATE TABLE operations (
    id                  BIGSERIAL,
    transaction_id      BIGINT NOT NULL,
    application_order   SMALLINT NOT NULL,
    source_account      VARCHAR(56) NOT NULL,
    type                VARCHAR(50) NOT NULL,
    details             JSONB NOT NULL,
    PRIMARY KEY (id, transaction_id),
    FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE
) PARTITION BY RANGE (transaction_id);

CREATE INDEX idx_operations_tx ON operations (transaction_id);
CREATE INDEX idx_operations_source ON operations (source_account);
CREATE INDEX idx_operations_details ON operations USING GIN (details);

-- Initial partition: transaction IDs 0 to 10,000,000
-- Additional partitions created by task 0022 (partition management automation)
CREATE TABLE operations_p0 PARTITION OF operations
    FOR VALUES FROM (0) TO (10000000);
