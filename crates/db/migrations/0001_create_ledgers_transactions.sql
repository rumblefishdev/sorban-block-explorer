-- Derived from: domain::ledger::Ledger (crates/domain/src/ledger.rs)
CREATE TABLE ledgers (
    sequence          BIGINT PRIMARY KEY NOT NULL,  -- Ledger.sequence: i64
    hash              VARCHAR(64) NOT NULL UNIQUE,  -- Ledger.hash: String
    closed_at         TIMESTAMPTZ NOT NULL,         -- Ledger.closed_at: DateTime<Utc>
    protocol_version  INTEGER NOT NULL,             -- Ledger.protocol_version: i32
    transaction_count INTEGER NOT NULL,             -- Ledger.transaction_count: i32
    base_fee          BIGINT NOT NULL               -- Ledger.base_fee: i64
);

CREATE INDEX idx_closed_at ON ledgers (closed_at DESC);

-- Derived from: domain::transaction::Transaction (crates/domain/src/transaction.rs)
CREATE TABLE transactions (
    id                BIGSERIAL PRIMARY KEY NOT NULL,  -- Transaction.id: i64
    hash              VARCHAR(64) NOT NULL UNIQUE,     -- Transaction.hash: String
    ledger_sequence   BIGINT NOT NULL,                 -- Transaction.ledger_sequence: i64
    source_account    VARCHAR(56) NOT NULL,            -- Transaction.source_account: String
    fee_charged       BIGINT NOT NULL,                 -- Transaction.fee_charged: i64
    successful        BOOLEAN NOT NULL,                -- Transaction.successful: bool
    result_code       VARCHAR(50),                     -- Transaction.result_code: Option<String>
    envelope_xdr      TEXT NOT NULL,                   -- Transaction.envelope_xdr: String
    result_xdr        TEXT NOT NULL,                   -- Transaction.result_xdr: String
    result_meta_xdr   TEXT,                            -- Transaction.result_meta_xdr: Option<String>
    memo_type         VARCHAR(20),                     -- Transaction.memo_type: Option<String>
    memo              TEXT,                            -- Transaction.memo: Option<String>
    created_at        TIMESTAMPTZ NOT NULL,            -- Transaction.created_at: DateTime<Utc>
    parse_error       BOOLEAN,                         -- Transaction.parse_error: Option<bool>
    operation_tree    JSONB,                           -- Transaction.operation_tree: Option<Value>
    FOREIGN KEY (ledger_sequence) REFERENCES ledgers(sequence)
);

CREATE INDEX idx_source ON transactions (source_account, created_at DESC);
CREATE INDEX idx_ledger ON transactions (ledger_sequence);
