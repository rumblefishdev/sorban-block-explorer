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
    /// Nested invocation tree JSON for direct rendering of the call graph.
    /// Populated externally by the persistence layer after calling `extract_invocations`.
    pub operation_tree: Option<serde_json::Value>,
    /// Memo type: `None` when no memo, or "text", "id", "hash", "return".
    pub memo_type: Option<String>,
    /// Memo value as string. Nullable.
    pub memo: Option<String>,
    /// Timestamp derived from parent ledger close time (Unix seconds). `i64` for PostgreSQL BIGINT compatibility.
    pub created_at: i64,
    /// True if XDR parsing failed for this transaction.
    pub parse_error: bool,
}

/// Extracted Soroban event data, maps to the `soroban_events` table.
///
/// Produced by `extract_events` from `SorobanTransactionMeta.events`.
#[derive(Debug, Clone)]
pub struct ExtractedEvent {
    /// Parent transaction hash, hex-encoded. Resolved to `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// Event type: "contract", "system", or "diagnostic".
    pub event_type: String,
    /// Contract that emitted the event (C... address). `None` for system events without a contract.
    pub contract_id: Option<String>,
    /// ScVal-decoded topic values as JSON array.
    pub topics: serde_json::Value,
    /// ScVal-decoded event data payload as JSON.
    pub data: serde_json::Value,
    /// Zero-based index of this event within the transaction.
    pub event_index: u32,
    /// Parent ledger sequence number.
    pub ledger_sequence: u32,
    /// Timestamp from parent ledger close time (Unix seconds), used for monthly partitioning.
    pub created_at: i64,
}

/// Extracted Soroban invocation data, maps to the `soroban_invocations` table.
///
/// Produced by `extract_invocations` — one row per node in the invocation tree.
#[derive(Debug, Clone)]
pub struct ExtractedInvocation {
    /// Parent transaction hash, hex-encoded. Resolved to `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// Invoked contract (C... address). `None` for non-contract invocations (e.g. create contract).
    pub contract_id: Option<String>,
    /// Account or contract that initiated this call. For root invocations this is the
    /// transaction source account; for sub-invocations it is the parent's contract address.
    pub caller_account: Option<String>,
    /// Function name invoked. `None` for contract creation invocations.
    pub function_name: Option<String>,
    /// ScVal-decoded function arguments as JSON value (typically an array; may be an object for
    /// create-contract invocations).
    pub function_args: serde_json::Value,
    /// ScVal-decoded return value. Populated for root invocations from `SorobanTransactionMeta`;
    /// `null` for sub-invocations (not available from auth entries).
    pub return_value: serde_json::Value,
    /// Whether this invocation succeeded (derived from the parent transaction success).
    pub successful: bool,
    /// Zero-based depth-first index of this node in the invocation tree.
    pub invocation_index: u32,
    /// Depth in the invocation tree (0 = root).
    pub depth: u32,
    /// Parent ledger sequence number.
    pub ledger_sequence: u32,
    /// Timestamp from parent ledger close time (Unix seconds), used for monthly partitioning.
    pub created_at: i64,
}

/// Extracted contract interface from WASM bytecode at deployment time.
///
/// Produced by `extract_contract_interfaces` when LedgerEntryChanges contain
/// new `ContractCodeEntry` items. Stored in `soroban_contracts.metadata` JSONB.
#[derive(Debug, Clone)]
pub struct ExtractedContractInterface {
    /// SHA-256 hash of the WASM bytecode, hex-encoded (64 chars).
    pub wasm_hash: String,
    /// Extracted public function signatures.
    pub functions: Vec<ContractFunction>,
    /// Raw WASM byte length (informational).
    pub wasm_byte_len: usize,
}

/// A single public function signature extracted from a contract's WASM spec.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContractFunction {
    /// Function name.
    pub name: String,
    /// Documentation string (may be empty).
    pub doc: String,
    /// Input parameter definitions.
    pub inputs: Vec<FunctionParam>,
    /// Output type names.
    pub outputs: Vec<String>,
}

/// A function parameter with name and type.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FunctionParam {
    pub name: String,
    pub type_name: String,
}

/// An NFT-related event detected during event extraction.
///
/// Produced by `detect_nft_events` for consumption by task 0027 (NFT state derivation).
#[derive(Debug, Clone)]
pub struct NftEvent {
    /// Parent transaction hash, hex-encoded.
    pub transaction_hash: String,
    /// Contract that emitted the event (C... address).
    pub contract_id: String,
    /// NFT event kind: "mint", "transfer", or "burn".
    pub event_kind: String,
    /// Token ID as ScVal-decoded JSON (e.g. `{ "type": ..., "value": ... }`).
    pub token_id: serde_json::Value,
    /// Sender address. `None` for mint events.
    pub from: Option<String>,
    /// Recipient address. `None` for burn events.
    pub to: Option<String>,
    /// Parent ledger sequence number.
    pub ledger_sequence: u32,
    /// Timestamp from parent ledger close time.
    pub created_at: i64,
}

/// Extracted operation data, maps to the `operations` table.
///
/// **Note:** field names do not directly mirror DB column names for this struct:
/// - `transaction_hash` → resolved to `transaction_id` (BIGSERIAL) by the persistence layer
/// - `operation_index` → `application_order` (Rust keyword `type` forces `op_type` similarly)
/// - `op_type` → `type` (`type` is a Rust keyword)
/// - `source_account: None` → persistence layer must substitute the transaction source account
///   (DB column is `NOT NULL`; `None` means no per-operation override)
#[derive(Debug, Clone)]
pub struct ExtractedOperation {
    /// Parent transaction hash, hex-encoded (64 chars). Used to resolve the
    /// surrogate `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// Zero-based index of this operation within the transaction (maps to `application_order`).
    pub operation_index: u32,
    /// Operation type string (e.g., "INVOKE_HOST_FUNCTION", "PAYMENT"). Maps to `type` column.
    pub op_type: String,
    /// Per-operation source account override. `None` if the operation inherits the transaction
    /// source. Persistence layer must resolve `None` to the transaction source (column is NOT NULL).
    pub source_account: Option<String>,
    /// Type-specific details as a JSON value, stored as JSONB in PostgreSQL.
    pub details: serde_json::Value,
}
