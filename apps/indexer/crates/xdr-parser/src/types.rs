//! Output types that map directly to the PostgreSQL schema.
//!
//! These types are the contract between the XDR parser and the database persistence layer.
//! Field names match DB column names (snake_case).

/// Extracted ledger data, maps to the `ledgers` table.
#[derive(Debug, Clone)]
pub struct ExtractedLedger {
    /// Ledger sequence number (PK).
    pub sequence: u32,
    /// SHA-256 hash of the LedgerHeaderHistoryEntry XDR, hex-encoded (64 chars).
    pub hash: String,
    /// Ledger close time as Unix timestamp (seconds). `i64` for PostgreSQL BIGINT compatibility.
    pub closed_at: i64,
    /// Stellar protocol version at this ledger.
    pub protocol_version: u32,
    /// Number of transactions in this ledger.
    pub transaction_count: u32,
    /// Base fee in stroops.
    pub base_fee: u32,
}

/// Extracted transaction data, maps to the `transactions` table.
#[derive(Debug, Clone)]
pub struct ExtractedTransaction {
    /// SHA-256 hash of the TransactionEnvelope, hex-encoded (64 chars).
    /// This is the public lookup key.
    pub hash: String,
    /// Parent ledger sequence number (FK to ledgers.sequence).
    pub ledger_sequence: u32,
    /// Transaction source account (G... or M... address, max 56 chars).
    pub source_account: String,
    /// Actual fee charged in stroops.
    pub fee_charged: i64,
    /// Whether the transaction succeeded.
    pub successful: bool,
    /// Transaction result code string (e.g., "txSuccess", "txFailed").
    pub result_code: String,
    /// Full transaction envelope, base64-encoded.
    pub envelope_xdr: String,
    /// Transaction result, base64-encoded.
    pub result_xdr: String,
    /// Transaction result metadata, base64-encoded. Nullable.
    pub result_meta_xdr: Option<String>,
    /// Memo type: `None` when no memo, or "text", "id", "hash", "return".
    pub memo_type: Option<String>,
    /// Memo value as string. Nullable.
    pub memo: Option<String>,
    /// Timestamp derived from parent ledger close time (Unix seconds). `i64` for PostgreSQL BIGINT compatibility.
    pub created_at: i64,
    /// True if XDR parsing failed for this transaction.
    pub parse_error: bool,
}
