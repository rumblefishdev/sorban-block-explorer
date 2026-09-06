//! Output types that map directly to the ClickHouse schema.
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

use domain::{AssetFamily, ContractEventType, ContractType, NftEventType, OperationType};
use stellar_xdr::TransactionEventStage;

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
    /// Ledger close time as Unix timestamp (seconds). `i64` for the DB `Int64` column.
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
    /// Transaction source account as a `G…` StrKey (a muxed `M…` source is
    /// reduced to its underlying account here, ADR 0026; the id it dropped is
    /// `source_muxed_id`).
    pub source_account: String,
    /// Multiplexing id of the source when the envelope named an `M…` address,
    /// `None` for a plain `G…`. Task 0540 keeps it for `from_muxed_id`.
    pub source_muxed_id: Option<u64>,
    /// Fee-bump payer account (`fee_source`), G-StrKey. `Some` only for a
    /// fee-bump envelope, where the payer funds the fee but is neither the inner
    /// `source_account` nor an op participant — registered separately into
    /// `transaction_participants` (task 0359 K2-4). `None` otherwise.
    pub fee_source: Option<String>,
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
    /// Timestamp derived from parent ledger close time (Unix seconds). `i64` for the DB `Int64` column.
    pub created_at: i64,
    /// True if XDR parsing failed for this transaction.
    pub parse_error: bool,
    /// Per-(account, asset) ledger balance deltas (task 0393), from the tx's
    /// `TransactionMeta`. Populated for EVERY tx, classic AND Soroban:
    /// `ledger_balance_deltas` reads the authoritative before→after balance
    /// changes on `AccountEntry` / `TrustLineEntry` / `ContractData` — the ledger
    /// is the unspoofable source of value, never the token events. There is no
    /// `soroban_meta` short-circuit; a Soroban invocation's value moves as
    /// `ContractData` balance changes the reader also covers.
    pub ledger_deltas: Vec<crate::ledger_value::LedgerDelta>,
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
    /// Zero-based envelope position of the operation that emitted this event.
    /// Only the CAP-67 V4 per-operation container carries the attribution —
    /// `None` for tx-level, diagnostic and V3 events (task 0453 D7).
    pub op_index: Option<u32>,
    /// Zero-based position of this event within its operation's own event
    /// list (`v4.operations[op_index].events`). Together with `op_index` this
    /// is Stellar's official event identity — the `getEvents` cursor is
    /// `(ledger, tx, op, event)` with `event` reset per operation (stellar-rpc
    /// `db/event.go`) — and it is what keys the edge table (task 0540).
    /// `None` whenever `op_index` is `None`.
    pub event_pos_in_op: Option<u32>,
    /// CAP-67 `TransactionEvent.stage` — when in ledger application the event
    /// fired. Measured on mainnet (`tests/tx_event_stage_real_meta.rs`): the
    /// fee charge is `BeforeAllTxs` and the refund is `AfterAllTxs` — settled
    /// after every transaction in the ledger, not after this one.
    /// The protocol's own statement of ordering, and the only one there is:
    /// `event_index` is our flat counter over the three containers, so an
    /// refund is numbered ahead of the operations it refunds.
    /// `None` for per-op, diagnostic and V3 events — only `v4.events` carries
    /// a stage.
    pub stage: Option<TransactionEventStage>,
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
/// new `ContractCodeEntry` items. Stored in the contract `metadata` column.
#[derive(Debug, Clone)]
pub struct ExtractedContractInterface {
    /// SHA-256 hash of the WASM bytecode, hex-encoded (64 chars).
    pub wasm_hash: String,
    /// Extracted public function signatures.
    pub functions: Vec<ContractFunction>,
    /// Raw WASM byte length (informational).
    pub wasm_byte_len: usize,
    /// Task 0327: the WASM imports `update_current_contract_wasm` (a self-upgrade
    /// path). See `wasm_imports_upgrade_fn` for how, and the API
    /// `ContractDetailResponse::upgradeable` for the user-facing 3-state.
    pub upgradeable: bool,
}

/// A single public function signature extracted from a contract's WASM spec.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Token metadata (`name` / `symbol` / `decimals`) recovered from the
    /// instance-storage `Symbol("METADATA")` struct, for a **non-SAC**
    /// contract-instance entry that carries one; `None` for every other entry
    /// type/change AND for SACs (skipped at extraction — a SAC's name/symbol/
    /// decimals derive from the asset identity, so no row is stored). Populated
    /// on entry-bearing changes (created / updated / state / restored); the
    /// producer consumes it on created / updated / restored.
    ///
    /// Chain-verified location (task 0297) — distinct from, and replaces, the
    /// dead standalone `Symbol("name")` path. Derived in-memory only; NOT
    /// serialized into `data` JSON, so it does not bloat persisted entry rows.
    pub token_metadata: Option<crate::token_metadata::TokenMetadata>,
}

/// One token-metadata write for a contract, derived from a contract-instance
/// `created` / `updated` / `restored` change carrying a `Symbol("METADATA")`
/// struct.
///
/// Maps to a `soroban_contract_metadata` side-table row (task 0297): a separate
/// per-contract table, written by the indexer, composed at read time — never
/// mixed into `soroban_contracts` (RMT whole-row clobber + different update
/// clocks). SACs never reach here — `entry_token_metadata` returns `None` for
/// them at extraction (their name/symbol/decimals derive from the asset
/// identity).
#[derive(Debug, Clone)]
pub struct ExtractedContractMetadata {
    pub contract_id: String,
    pub metadata: crate::token_metadata::TokenMetadata,
    /// Ledger the metadata was observed — the side table's RMT version slot.
    pub ledger: u32,
}

/// A per-holder Soroban token balance recovered from a `ContractData`
/// `Balance(Address)` ledger-entry change (task 0331).
///
/// Persisted into the unified `balances` table (task 0331 Option C; the in-memory
/// `soroban_token_balances` identifiers are leftover Option-A naming — there is no
/// table of that name): one row per `(holder, asset_id)`, RMT-versioned by `ledger`.
/// Reading ledger STATE is
/// correct-by-construction for vault / rebasing / non-SEP-41-event tokens, where
/// an event-fold under-counts (see task README DECISION 2026-06-29).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedSorobanBalance {
    /// Token contract StrKey (`C…`). Hashed to the surrogate id at persistence.
    pub contract_id: String,
    /// Holder address StrKey — `G…` account OR `C…` contract (34% are contracts).
    pub holder: String,
    /// Current balance (i128; tokens can exceed i64 / use >7 decimals).
    pub balance: i128,
    /// Ledger the balance was observed — the side table's RMT version slot.
    pub ledger: u32,
    /// `true` when the `ContractData` entry was REMOVED in this change, rather
    /// than merely spent down to zero. Both emit `balance = 0`, so the flag is
    /// the only thing separating them — ADR 0055.
    pub closed: bool,
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
    /// Full signer set from the observed `AccountEntry`, master EXCLUDED (raw
    /// XDR truth — master weight is thresholds byte 0; Horizon synthesizes a
    /// master entry, we do not). `Some(vec)` — entry observed, and the vec IS
    /// the complete set (may be empty: removing the last signer must emit an
    /// empty set so the RMT row atomically supersedes the old one). `None` —
    /// no `AccountEntry` in this change set (trustline-only accum); no signers
    /// row may be written. lore-0463.
    pub signers: Option<Vec<serde_json::Value>>,
    /// 4-byte hex from `AccountEntry.thresholds`: master weight, low, med,
    /// high. Same Some/None semantics as `signers`.
    pub thresholds: Option<String>,
    /// `AccountEntry.flags`. Same Some/None semantics as `signers`.
    pub flags: Option<u32>,
    /// `true` when the ACCOUNT ENTRY itself was removed in this change set —
    /// an `account_merge`, the only way an account is deleted. The native
    /// balance is emitted as 0 either way, so without this flag the merge
    /// tombstone is indistinguishable from an account legitimately holding no
    /// XLM (sponsored reserves make that a live state, CAP-0033). Persist maps
    /// it to `balances.closed_at_ledger` — ADR 0055.
    pub account_removed: bool,
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
}

/// Detected asset from contract deployments or classic assets.
///
/// Produced by `detect_assets`. Maps to `assets` table.
#[derive(Debug, Clone)]
pub struct ExtractedAsset {
    /// Asset classification (ADR 0031). Maps to `assets.asset_type SMALLINT`.
    pub asset_type: AssetFamily,
    pub asset_code: Option<String>,
    pub issuer_address: Option<String>,
    /// Soroban contract identity (`C…` StrKey) — set ONLY for `soroban`
    /// (type=3) assets, where the contract IS the asset. `None` for
    /// native / classic_credit (a SAC handle rides in `sac_contract_id`).
    pub contract_id: Option<String>,
    /// SAC facet (ADR 0051): the wrapping Stellar-Asset-Contract's `C…`
    /// StrKey for a `native` / `classic_credit` asset. `None` when the asset
    /// has no SAC. Persisted as a cityhash64 surrogate on `asset_sac.sac_contract_id`
    /// (a side table — not a column on `assets`).
    pub sac_contract_id: Option<String>,
    /// Whether `sac_contract_id`'s SAC is actually deployed on-chain (ADR 0051).
    /// `true` from a SAC deploy sighting; `false` for an un-deployed SAC seen
    /// only via events. Meaningless (and `false`) when `sac_contract_id` is `None`.
    pub sac_deployed: bool,
    // No `total_supply` / `holder_count`: the parser never had a value for
    // either (always `None`), and the columns they fed were dropped in 0310 —
    // supply/holders are computed from `balances` into `balance_aggregates`.
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
    /// `true` when the pool-share trustline was REMOVED — the participant left
    /// the pool. A full withdrawal that keeps the trustline also writes
    /// `shares = 0`, so the flag is what separates them. ADR 0055.
    pub closed: bool,
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
    /// identity columns; not persisted as JSON anywhere in the DB.
    pub details: serde_json::Value,
    /// The assets declared in this operation's body (task 0359). Pure presence,
    /// staged into `operation_asset_appearances`. Deterministic: identical
    /// between live ingest and the archive backfill re-parse (both run this
    /// parser).
    pub asset_appearances: Vec<crate::asset_appearances::AssetRef>,
    /// Account counterparties the operation touches beyond its source (task 0359
    /// F-C / K1-5): crossed-offer sellers (result claim atoms), claimable-balance
    /// claimants, `SetOptions.inflationDest`, revoke-sponsorship targets. Raw
    /// G-StrKeys, staged into `transaction_participants` (deduped there).
    /// Deterministic between live ingest and the backfill re-parse.
    pub counterparties: Vec<String>,
    /// Multiplexing id of the per-op source when it is an `M…` address; the
    /// `G…` half is `source_account`. `None` when the op inherits the tx source
    /// or the source is a plain account. Task 0540 (`from_muxed_id`).
    pub source_muxed_id: Option<u64>,
    /// Multiplexing id of the op's destination when it is an `M…` address —
    /// `Payment`, both path payments and `AccountMerge` are the only classic
    /// operations CAP-27 lets address a muxed account. The `G…` half is
    /// `details.destination`. Task 0540 (`to_muxed_id`).
    pub destination_muxed_id: Option<u64>,
}
