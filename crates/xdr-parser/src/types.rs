//! Output types that map directly to the PostgreSQL schema.
//!
//! These types are the contract between the XDR parser and the database persistence layer.
//! Field names match DB column names (snake_case).
//!
//! ADR 0031: enum-like columns (`operations.type`, `soroban_events.event_type`,
//! `soroban_contracts.contract_type`, `assets.asset_type`,
//! `nft_ownership.event_type`) are typed via `domain::enums` enums — the
//! parser emits the typed variant directly, skipping the legacy
//! string round-trip through `Debug`/`Display`. ADR 0033 removed
//! `soroban_events.event_type` from the DB column; `ExtractedEvent.event_type`
//! is still produced for in-memory classification (diagnostic filtering +
//! read-time tagging).

use domain::{ContractEventType, ContractType, NftEventType, OperationType, TokenAssetType};

/// Underlying classic asset identity carried by a SAC deployment.
///
/// Sourced from `CreateContractArgs.contract_id_preimage` with variant
/// `FromAsset(Asset)`. The ContractInstance XDR entry for a SAC is a
/// marker-only `{"type": "stellar_asset"}` and carries no asset data,
/// so this is the sole path for populating `assets.asset_code` /
/// `.issuer_id` for SAC rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SacAssetIdentity {
    /// XLM-SAC — wraps the native Stellar asset. No code / issuer.
    /// Persisted with NULL `asset_code` + NULL `issuer_id` (allowed by
    /// `ck_assets_identity` when `contract_id IS NOT NULL`).
    Native,
    /// Classic-credit SAC — wraps a `credit_alphanum4` or
    /// `credit_alphanum12` asset with a real issuer.
    Credit {
        /// Asset code (trailing NULs already stripped by the parser).
        code: String,
        /// Issuer `G...` StrKey.
        issuer: String,
    },
}

/// Extracted ledger data, maps to the `ledgers` table.
#[derive(Debug, Clone)]
pub struct ExtractedLedger {
    /// Ledger sequence number (PK).
    pub sequence: u32,
    /// Canonical Stellar ledger hash from `LedgerHeaderHistoryEntry.hash`,
    /// hex-encoded (64 chars). Matches Horizon `/ledgers/:N.hash` and every
    /// other Stellar tool — populated by core, never recomputed.
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
    /// SHA-256 hash of the **inner** transaction for fee-bump envelopes,
    /// hex-encoded. `None` for non-fee-bump (where `hash` already IS the
    /// principal hash). Matches Horizon's `inner_transaction.hash`.
    pub inner_tx_hash: Option<String>,
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

/// Container an `ExtractedEvent` was sourced from in the on-chain meta.
///
/// CAP-67 (Protocol 23+) splits events across three V4 locations:
/// `v4.events` (tx-level), `v4.operations[i].events` (per-op), and
/// `v4.diagnostic_events` (host-VM trace entries + byte-identical
/// Contract-typed copies of the per-op consensus events). Filtering by
/// inner `event_type` cannot distinguish a real consensus Contract event
/// from its diagnostic-container copy; the source container is the only
/// reliable signal. Task 0182.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSource {
    /// `v4.events` (Protocol 23+) or `soroban_meta.events` (Protocol 22).
    /// Hashed into `txSetResultHash` — counts toward consensus.
    TxLevel,
    /// `v4.operations[i].events` — CAP-67 per-operation consensus events.
    /// Hashed; carries the bulk of post-Protocol-23 Soroban traffic.
    /// Not produced for V3 meta.
    PerOp,
    /// `v4.diagnostic_events` or `soroban_meta.diagnostic_events`. NOT
    /// hashed (CAP-67 spec). When diagnostic mode is enabled (the default
    /// for Galexie's captive-core), this container holds byte-identical
    /// Contract-typed copies of every consensus per-op Contract event
    /// alongside the host-VM trace entries — staging must drop the
    /// entire container regardless of inner type.
    Diagnostic,
}

/// Extracted Soroban event data, produced by `extract_events` from
/// `SorobanTransactionMeta.events`.
///
/// ADR 0033: only contract-scoped non-diagnostic events contribute to the
/// `soroban_events_appearances` aggregate — full detail is re-expanded from
/// the public archive at read time.
#[derive(Debug, Clone)]
pub struct ExtractedEvent {
    /// Parent transaction hash, hex-encoded. Resolved to `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// Event type (ADR 0031). In-memory classifier only; not persisted to DB.
    pub event_type: ContractEventType,
    /// Source container this event was extracted from. Used by staging and
    /// read-time API to drop the entire `diagnostic_events` container,
    /// including its byte-identical Contract-typed mirrors of per-op
    /// consensus events (task 0182).
    pub source: EventSource,
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

/// Extracted Soroban invocation data, aggregated at indexer staging into
/// `soroban_invocations_appearances` rows (ADR 0034). At read time the API
/// re-extracts this structure from the public archive's XDR to render E13
/// per-node detail (function name, caller, success, args, return value).
///
/// Produced by `extract_invocations` — one value per node in the invocation
/// tree, emitted depth-first (root before sub-invocations).
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

/// Extracted ledger entry change from `TransactionMeta` V3/V4.
///
/// Produced by `extract_ledger_entry_changes`. One row per `LedgerEntryChange`
/// found in `tx_changes_before`, per-operation changes, and `tx_changes_after`.
#[derive(Debug, Clone)]
pub struct ExtractedLedgerEntryChange {
    /// Parent transaction hash, hex-encoded. Resolved to `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// Change type: "created", "updated", "removed", "state", or "restored".
    pub change_type: String,
    /// Ledger entry type: "account", "trustline", "offer", "data", "claimable_balance",
    /// "liquidity_pool", "contract_data", "contract_code", "config_setting", "ttl".
    pub entry_type: String,
    /// Identifying key fields as JSON (e.g. account_id, offer_id, contract + key).
    pub key: serde_json::Value,
    /// Full entry data as JSON. `None` for "removed" changes (only key is available).
    pub data: Option<serde_json::Value>,
    /// Zero-based index of this change within the transaction.
    pub change_index: u32,
    /// Operation index this change belongs to. `None` for tx-level changes
    /// (`tx_changes_before` / `tx_changes_after`).
    pub operation_index: Option<u32>,
    /// Parent ledger sequence number.
    pub ledger_sequence: u32,
    /// Timestamp from parent ledger close time (Unix seconds).
    pub created_at: i64,
}

/// Extracted contract deployment from LedgerEntryChanges.
///
/// Produced by `extract_contract_deployments` when a new contract instance
/// appears in ledger entry changes. Maps to `soroban_contracts` table.
#[derive(Debug, Clone)]
pub struct ExtractedContractDeployment {
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    pub deployer_account: Option<String>,
    pub deployed_at_ledger: u32,
    /// Explorer-synthetic classification (ADR 0031). Maps to
    /// `soroban_contracts.contract_type SMALLINT` (nullable at DB level,
    /// but always set when the parser produces a deployment).
    pub contract_type: ContractType,
    pub is_sac: bool,
    /// Human-readable contract name extracted from the standard
    /// `Symbol("name")` ContractData persistent storage entry, when
    /// present in the same ledger as the deployment (constructor
    /// pattern). For deploy-then-init contracts where storage writes
    /// land in a later ledger, this is `None` at deployment time and
    /// the indexer's retroactive UPDATE path
    /// (`extract_contract_data_name_writes`) populates the column on
    /// the next ledger that emits the storage entry.
    ///
    /// Maps to `soroban_contracts.name VARCHAR(256)` per ADR 0042.
    pub name: Option<String>,
    /// Task 0160 — SAC underlying asset identity resolved from
    /// `ContractIdPreimage::FromAsset` (top-level op OR auth-entry
    /// `CreateContractHostFn`), correlated by the preimage-derived
    /// contract_id.
    ///
    /// * `Some(Native)` — XLM-SAC, persisted with NULL code/issuer.
    /// * `Some(Credit { .. })` — classic credit SAC, persisted with real
    ///   code + issuer.
    /// * `None` — non-SAC deployment, OR a SAC whose creating preimage
    ///   is not present in the current batch (e.g. replay starting from
    ///   mid-ledger without the original deploy tx). `detect_assets`
    ///   skips such SACs with a `tracing::warn`.
    pub sac_asset: Option<SacAssetIdentity>,
}

/// Extracted account state from LedgerEntryChanges.
///
/// Produced by `extract_account_states`. Maps to `accounts` table.
#[derive(Debug, Clone)]
pub struct ExtractedAccountState {
    pub account_id: String,
    /// Set on account creation only. `None` for updates.
    pub first_seen_ledger: Option<u32>,
    /// Updated on every change. Watermark column.
    pub last_seen_ledger: u32,
    pub sequence_number: i64,
    pub balances: serde_json::Value,
    /// Trustlines removed in this change set. Each entry is `{asset_type, asset_code, issuer}`.
    /// Tracked separately from `balances` to avoid marker pollution on INSERT.
    pub removed_trustlines: Vec<serde_json::Value>,
    pub home_domain: Option<String>,
    pub created_at: i64,
}

/// Extracted liquidity pool state from LedgerEntryChanges.
///
/// Produced by `extract_liquidity_pools`. Maps to `liquidity_pools` table.
#[derive(Debug, Clone)]
pub struct ExtractedLiquidityPool {
    pub pool_id: String,
    pub asset_a: serde_json::Value,
    pub asset_b: serde_json::Value,
    pub fee_bps: i32,
    pub reserves: serde_json::Value,
    pub total_shares: String,
    pub tvl: Option<String>,
    /// Set on pool creation only. `None` for updates.
    pub created_at_ledger: Option<u32>,
    /// Updated on every change. Watermark column.
    pub last_updated_ledger: u32,
    pub created_at: i64,
}

/// Liquidity pool snapshot, appended on each pool change.
///
/// Produced alongside `ExtractedLiquidityPool`. Maps to `liquidity_pool_snapshots`.
#[derive(Debug, Clone)]
pub struct ExtractedLiquidityPoolSnapshot {
    pub pool_id: String,
    pub ledger_sequence: u32,
    pub created_at: i64,
    pub reserves: serde_json::Value,
    pub total_shares: String,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
}

/// Detected asset from contract deployments or classic assets.
///
/// Produced by `detect_assets`. Maps to `assets` table.
#[derive(Debug, Clone)]
pub struct ExtractedAsset {
    /// Asset classification (ADR 0031). Maps to `assets.asset_type SMALLINT`.
    pub asset_type: TokenAssetType,
    pub asset_code: Option<String>,
    pub issuer_address: Option<String>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
}

/// Detected NFT from events and ledger entry changes.
///
/// Produced by `detect_nfts`. Maps to `nfts` table.
#[derive(Debug, Clone)]
pub struct ExtractedNft {
    pub contract_id: String,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub owner_account: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub minted_at_ledger: Option<u32>,
    /// Updated on every NFT state change. Watermark column.
    pub last_seen_ledger: u32,
    pub created_at: i64,
}

/// NFT ownership event carried from the parser into `nft_ownership`.
///
/// Schema-shaped superset of `NftEvent` that resolves the NFT row identity
/// (`contract_id`, `token_id`) and carries the ownership transition needed for
/// `nft_ownership` rows. Not produced by the parser today — task 0118 (NFT
/// false-positive filtering) will gate population. Until then, `process_ledger`
/// passes an empty slice.
#[derive(Debug, Clone)]
pub struct ExtractedNftEvent {
    /// Parent transaction hash, hex-encoded. Resolved to `transaction_id` at persistence time.
    pub transaction_hash: String,
    /// NFT collection contract address (C... StrKey).
    pub contract_id: String,
    /// Stable per-collection token identity (matches `nfts.token_id`).
    pub token_id: String,
    /// Event kind (ADR 0031). Maps to `nft_ownership.event_type SMALLINT`.
    pub event_type: NftEventType,
    /// New owner after the event. `None` for burns.
    pub owner_account: Option<String>,
    /// Stable order within the ledger. Maps to `nft_ownership.event_order`.
    pub event_order: u16,
    /// Parent ledger sequence number.
    pub ledger_sequence: u32,
    /// Unix seconds. Matches parent transaction partitioning key.
    pub created_at: i64,
}

/// LP position change carried from the parser into `lp_positions`.
///
/// Not produced by the parser today — task 0126 (LP participant tracking) will
/// gate population. Until then, `process_ledger` passes an empty slice.
#[derive(Debug, Clone)]
pub struct ExtractedLpPosition {
    /// Pool hash, hex-encoded (matches `liquidity_pools.pool_id` after decode).
    pub pool_id: String,
    /// Participant StrKey. Resolved to `accounts.id` at persistence time.
    pub account_id: String,
    /// Pool-share balance as decimal string (NUMERIC(28,7) in schema).
    pub shares: String,
    /// Ledger where this participant first deposited. Set only on the first
    /// appearance of `(pool_id, account_id)`; `None` on subsequent updates.
    pub first_deposit_ledger: Option<u32>,
    /// Ledger of the change. Watermark column — older values must not overwrite newer.
    pub last_updated_ledger: u32,
}

/// Extracted operation data. Feeds the `operations_appearances` indexer path
/// (task 0163) where operations of identical identity are collapsed into a
/// single appearance row, and the API's XDR re-materialisation path
/// (`stellar_archive::extractors`) where `operation_index` is surfaced as
/// `application_order` in the DTO.
///
/// **Note:** field names do not directly mirror DB column names:
/// - `transaction_hash` → resolved to `transaction_id` (BIGSERIAL) by the persistence layer
/// - `operation_index` → not persisted in `operations_appearances` (ordering is
///   re-derived from XDR by the API when needed); still surfaced in the
///   `stellar_archive` DTO as `application_order`
/// - `op_type` → `type` (`type` is a Rust keyword)
/// - `source_account: None` → operation inherits the transaction source account
#[derive(Debug, Clone)]
pub struct ExtractedOperation {
    /// Parent transaction hash, hex-encoded (64 chars). Used to resolve the
    /// surrogate `transaction_id` FK at persistence time.
    pub transaction_hash: String,
    /// 1-based index of this operation within the transaction (matches Horizon
    /// `paging_token` convention; see ADR 0028 / task 0172). Not persisted in
    /// `operations_appearances` — the API re-derives ordering from XDR.
    pub operation_index: u32,
    /// Operation type (ADR 0031). Maps to `operations_appearances.type SMALLINT`.
    pub op_type: OperationType,
    /// Per-operation source account override. `None` if the operation inherits the transaction
    /// source.
    pub source_account: Option<String>,
    /// Type-specific details as a JSON value. Consumed by staging to extract
    /// identity columns; not persisted as JSONB anywhere in the DB.
    pub details: serde_json::Value,
}
