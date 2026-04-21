//! Parsed-ledger artifact v1 — canonical JSON shape.
//!
//! See `lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md` for the full
//! specification. Every field here mirrors an ADR 0028 field exactly.
//!
//! This module is a local sanity-check scaffold — the public API is frozen
//! after PR 1 of task 0146. Do not change shapes without a coordinated
//! update to 0145 / 0147 / ADR 0028.

use serde::{Deserialize, Serialize};
use stellar_xdr::curr::LedgerCloseMeta;

use crate::error::ParseError;

/// Schema version marker placed in `ledger_metadata.schema_version`.
pub const SCHEMA_VERSION: &str = "v1";

/// Default zstd compression level for artifact serialization.
pub const DEFAULT_ZSTD_LEVEL: i32 = 3;

// ========================================================================
// Root
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedLedgerArtifact {
    pub ledger_metadata: LedgerMetadata,
    pub transactions: Vec<TransactionArtifact>,
    pub account_states: Vec<AccountStateArtifact>,
    pub liquidity_pools: Vec<LiquidityPoolArtifact>,
    pub liquidity_pool_snapshots: Vec<LiquidityPoolSnapshotArtifact>,
    pub nft_events: Vec<NftEventArtifact>,
    pub wasm_uploads: Vec<WasmUploadArtifact>,
    pub contract_metadata: Vec<ContractMetadataArtifact>,
    pub token_metadata: Vec<TokenMetadataArtifact>,
    pub nft_metadata: Vec<NftMetadataArtifact>,
}

// ========================================================================
// ledger_metadata
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerMetadata {
    pub schema_version: String,
    pub sequence: u32,
    pub hash: String,
    pub closed_at: i64,
    pub protocol_version: u32,
    pub transaction_count: u32,
    pub base_fee: u32,
}

// ========================================================================
// transactions[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionArtifact {
    pub hash: String,
    pub application_order: u16,
    pub source_account: String,
    pub source_account_muxed: Option<String>,
    pub fee_account: Option<String>,
    pub fee_account_muxed: Option<String>,
    pub inner_tx_hash: Option<String>,
    pub fee_charged: i64,
    pub successful: bool,
    pub result_code: String,
    pub memo_type: MemoType,
    pub memo: Option<String>,
    pub envelope_xdr: String,
    pub result_xdr: String,
    pub result_meta_xdr: Option<String>,
    pub signatures: Vec<Signature>,
    pub operations: Vec<OperationArtifact>,
    pub events: Vec<EventArtifact>,
    pub invocations: Vec<InvocationArtifact>,
    pub ledger_entry_changes: Vec<LedgerEntryChangeArtifact>,
    pub parse_error: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoType {
    None,
    Text,
    Id,
    Hash,
    Return,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub hint: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationArtifact {
    pub application_order: u16,
    pub op_type: String,
    pub source_account: Option<String>,
    pub source_account_muxed: Option<String>,
    pub destination: Option<String>,
    pub destination_muxed: Option<String>,
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    pub pool_id: Option<String>,
    pub function_name: Option<String>,
    pub transfer_amount: Option<String>,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventArtifact {
    pub event_index: u32,
    pub event_type: EventType,
    pub contract_id: Option<String>,
    pub topics: serde_json::Value,
    pub data: serde_json::Value,
    pub transfer_from: Option<String>,
    pub transfer_to: Option<String>,
    pub transfer_amount: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Contract,
    System,
    Diagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationArtifact {
    pub invocation_index: u32,
    pub parent_index: Option<u32>,
    pub contract_id: Option<String>,
    pub caller_account: Option<String>,
    pub function_name: String,
    pub function_args: serde_json::Value,
    pub return_value: Option<serde_json::Value>,
    pub successful: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntryChangeArtifact {
    pub change_index: u32,
    pub change_type: ChangeType,
    pub entry_type: EntryType,
    pub key: serde_json::Value,
    pub data: Option<serde_json::Value>,
    pub operation_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Created,
    Updated,
    Removed,
    State,
    Restored,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Account,
    Trustline,
    Offer,
    Data,
    ClaimableBalance,
    LiquidityPool,
    ContractData,
    ContractCode,
    ConfigSetting,
    Ttl,
}

// ========================================================================
// account_states[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStateArtifact {
    pub account_id: String,
    pub first_seen_ledger: Option<u32>,
    pub last_seen_ledger: u32,
    pub sequence_number: String,
    pub home_domain: Option<String>,
    pub balances: Vec<BalanceArtifact>,
    pub removed_trustlines: Vec<RemovedTrustlineArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceArtifact {
    pub asset_type: BalanceAssetType,
    pub asset_code: Option<String>,
    pub issuer_address: Option<String>,
    pub balance: String,
    pub last_updated_ledger: u32,
}

/// Balance asset types carried in `account_states[].balances[]`.
///
/// Pool-share trustlines are NOT represented here — parser
/// `extract_account_states` skips them (see `state.rs:225-226`), and
/// `lp_positions` population is out of scope for artifact v1 pending
/// task 0126.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceAssetType {
    Native,
    Classic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedTrustlineArtifact {
    pub asset_type: RemovedTrustlineAssetType,
    pub asset_code: String,
    pub issuer_address: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemovedTrustlineAssetType {
    Classic,
}

// ========================================================================
// liquidity_pools[] + liquidity_pool_snapshots[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPoolArtifact {
    pub pool_id: String,
    pub asset_a_type: PoolAssetType,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer: Option<String>,
    pub asset_b_type: PoolAssetType,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer: Option<String>,
    pub fee_bps: i32,
    pub created_at_ledger: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolAssetType {
    Native,
    Classic,
    Sac,
    Soroban,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPoolSnapshotArtifact {
    pub pool_id: String,
    pub reserve_a: String,
    pub reserve_b: String,
    pub total_shares: String,
}

// ========================================================================
// nft_events[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftEventArtifact {
    pub transaction_hash: String,
    pub contract_id: String,
    pub event_kind: NftEventKind,
    pub token_id: serde_json::Value,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NftEventKind {
    Mint,
    Transfer,
    Burn,
}

// ========================================================================
// wasm_uploads[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmUploadArtifact {
    pub wasm_hash: String,
    pub wasm_byte_len: u64,
    pub functions: Vec<WasmFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmFunction {
    pub name: String,
    pub doc: String,
    pub inputs: Vec<WasmFunctionParam>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmFunctionParam {
    pub name: String,
    pub type_name: String,
}

// ========================================================================
// contract_metadata[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetadataArtifact {
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    pub deployer_account: Option<String>,
    pub deployed_at_ledger: u32,
    pub contract_type: ContractType,
    pub is_sac: bool,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContractType {
    Token,
    Dex,
    Lending,
    Nft,
    Other,
}

// ========================================================================
// token_metadata[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadataArtifact {
    pub asset_type: TokenAssetType,
    pub asset_code: Option<String>,
    pub issuer_address: Option<String>,
    pub contract_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenAssetType {
    Native,
    Classic,
    Sac,
    Soroban,
}

// ========================================================================
// nft_metadata[]
// ========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftMetadataArtifact {
    pub contract_id: String,
    pub token_id: String,
    pub owner_account: Option<String>,
    pub minted_at_ledger: Option<u32>,
    pub current_owner_ledger: u32,
}

// ========================================================================
// Errors
// ========================================================================

#[derive(Debug)]
pub enum ArtifactError {
    Parse(ParseError),
    Serialize(serde_json::Error),
    Compress(std::io::Error),
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "parse: {}", e.message),
            Self::Serialize(e) => write!(f, "serialize: {e}"),
            Self::Compress(e) => write!(f, "compress: {e}"),
        }
    }
}

impl std::error::Error for ArtifactError {}

impl From<ParseError> for ArtifactError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<serde_json::Error> for ArtifactError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialize(e)
    }
}

impl From<std::io::Error> for ArtifactError {
    fn from(e: std::io::Error) -> Self {
        Self::Compress(e)
    }
}

// ========================================================================
// Public API (frozen after PR 1)
// ========================================================================

/// Build the canonical artifact from a single `LedgerCloseMeta`.
///
/// Pure function: no I/O, no AWS, no DB. Reuses the `extract_*`
/// functions already exported from this crate. Partial failures on a
/// per-tx basis surface as `transactions[i].parse_error = true`; a
/// top-level `Err` indicates the ledger could not be processed at all.
pub fn build_parsed_ledger_artifact(
    _meta: &LedgerCloseMeta,
) -> Result<ParsedLedgerArtifact, ParseError> {
    todo!("ADR 0028 artifact builder — implemented in PR 2 (task 0146)")
}

/// Serialize the artifact to canonical JSON bytes.
///
/// Deterministic: the same input produces byte-identical output. No
/// pretty printing, no trailing newline.
pub fn serialize_artifact_json(_artifact: &ParsedLedgerArtifact) -> Result<Vec<u8>, ArtifactError> {
    todo!("ADR 0028 serializer — implemented in PR 2 (task 0146)")
}

/// Compress JSON bytes with zstd at `DEFAULT_ZSTD_LEVEL`.
pub fn compress_artifact_zstd(_json: &[u8]) -> Result<Vec<u8>, ArtifactError> {
    todo!("zstd compression — implemented in PR 2 (task 0146)")
}

/// Build the S3 key for a given ledger sequence.
///
/// Layout: `parsed-ledgers/v1/{partition_start}-{partition_end}/parsed_ledger_{seq}.json.zst`
/// with 64k-ledger partitions.
pub fn parsed_ledger_s3_key(_sequence: u32) -> String {
    todo!("S3 key layout — implemented in PR 2 (task 0146)")
}
