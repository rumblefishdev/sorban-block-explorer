//! Row structs for the CH writer — production schema (hybrid: surrogate
//! `id Int64` on the three high-cardinality FK hubs, natural / composite
//! keys elsewhere).
//!
//! One `#[derive(clickhouse::Row, serde::Serialize)]` struct per table
//! in `crates/db-clickhouse/schema/init.sql`. Column name + order
//! match `init.sql` byte-for-byte (RowBinary is positional —
//! mismatch silently corrupts every row written). Column-order
//! pinning tests in `persist/tests_cross.rs` guard against drift.
//!
//! ## Surrogate-id hubs (`accounts`, `soroban_contracts`, `transactions`)
//!
//! Each carries `id: i64` derived deterministically via
//! [`super::ids`] (`cityhash64(natural_key)`). FK columns in 10+
//! downstream tables are `Int64` referencing these `id`s. See `ids.rs`
//! module docs for the why-this-three-tables rationale.
//!
//! ## Natural / composite keys (everything else)
//!
//! `assets`, `nfts`, `liquidity_pools`, `lp_positions`,
//! `liquidity_pool_snapshots`, `operations_appearances`,
//! `transaction_participants`, `nft_ownership` — composite ORDER BY
//! over already-cheap-shape columns (FixedString(32) hashes,
//! low-cardinality codes, Int64 FK references).
//!
//! ## Type translation
//!
//! - `Int64` → `i64`
//! - `Int32` → `i32`
//! - `Int16` → `i16`
//! - `Bool` → `bool`
//! - `String` (incl. `LowCardinality(String)`) → `String`
//! - `Nullable(String)` (incl. LC-wrapped) → `Option<String>`
//! - `FixedString(32)` → `[u8; 32]`
//! - `Nullable(FixedString(32))` → `Option<[u8; 32]>`
//! - `Decimal128(7)` → `i128` (scaled by 10⁷, matches PG `NUMERIC(28,7)`)
//! - `Nullable(Decimal128(7))` → `Option<i128>`
//! - `DateTime64(3, 'UTC')` → `i64` ms since Unix epoch

use clickhouse::Row;
use serde::{Deserialize, Serialize};

/// `ledgers` — immutable lookup, MergeTree, partitioned.
#[derive(Debug, Clone, Row, Serialize)]
pub struct LedgerRow {
    pub sequence: i64,
    pub hash: [u8; 32],
    /// `DateTime64(3, 'UTC')` — milliseconds since Unix epoch.
    pub closed_at: i64,
    pub protocol_version: i32,
    pub transaction_count: i32,
    pub base_fee: i64,
}

/// `wasm_interface_metadata` — immutable lookup, MergeTree.
#[derive(Debug, Clone, Row, Serialize)]
pub struct WasmInterfaceMetadataRow {
    pub wasm_hash: [u8; 32],
    pub metadata: String,
}

/// `accounts` — state hub, RMT(last_seen_ledger). Surrogate `id` for
/// FK joins; ORDER BY natural key `account_id` for direct lookups.
#[derive(Debug, Clone, Row, Serialize)]
pub struct AccountRow {
    pub id: i64,
    pub account_id: String,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
    pub sequence_number: i64,
    pub home_domain: Option<String>,
}

/// `account_entry_state` — state, RMT(last_updated_ledger), PK = account_id.
/// ONE row per account, the FULL signer set as parallel arrays — atomic
/// whole-set replacement, so removed signers cannot ghost. Master weight is
/// thresholds byte 0, never in the arrays (raw XDR truth; Horizon
/// synthesizes). lore-0463.
#[derive(Debug, Clone, Row, Serialize)]
pub struct AccountEntryStateRow {
    pub account_id: i64,
    pub signer_keys: Vec<String>,
    pub signer_weights: Vec<u32>,
    pub signer_types: Vec<String>,
    pub master_weight: u8,
    pub threshold_low: u8,
    pub threshold_med: u8,
    pub threshold_high: u8,
    pub flags: u32,
    pub last_updated_ledger: i64,
}

/// `assets` — state, plain RMT. Composite PK: identity 4-tuple.
/// Native XLM: asset_type=0, asset_code='', issuer_id=0, contract_id=0.
/// The dead `total_supply` / `holder_count` / `icon_url` columns were removed
/// from this struct + `init.sql` in task 0310 (nothing read them: supply/holders
/// come from `balance_aggregates` (lore-0293), display name/icon from
/// `asset_enrichment` coalesced over `soroban_contract_metadata`). Dropping the
/// fields here shortens the INSERT column list, so it is safe against a prod
/// table that still has them — they keep taking their NULL default until the
/// operator's `ALTER … DROP COLUMN` runs. `name` went the same way in 0304.
#[derive(Debug, Clone, Row, Serialize)]
pub struct AssetRow {
    pub asset_type: i16,
    pub asset_code: String,
    pub issuer_id: i64,
    pub contract_id: i64,
    /// lore-0331 surrogate (`ids::asset_id`) — single-column asset key for
    /// `balances.asset_id`. Column order MUST match `assets` schema (id last).
    pub id: i64,
}

impl AssetRow {
    /// Build a staging `AssetRow` from its identity, computing the `id`
    /// surrogate ONCE from the identity so no build site can forget it or diverge.
    pub fn staged(asset_type: i16, asset_code: String, issuer_id: i64, contract_id: i64) -> Self {
        Self {
            id: super::ids::asset_id(asset_type, &asset_code, issuer_id, contract_id),
            asset_type,
            asset_code,
            issuer_id,
            contract_id,
        }
    }
}

/// `asset_sac` — indexer-owned SAC facet side table (ADR 0051 / task 0339),
/// `AggregatingMergeTree` with `SimpleAggregateFunction(max)` columns. Keyed
/// byte-for-byte like `assets`; written ONLY on a SAC sighting (deploy or
/// un-deployed override), so the per-ledger whole-row `assets` rewrite cannot
/// zero it. `max()` merges column-wise: `sac_deployed` (monotonic) sticks true
/// once deployed; `sac_contract_id` is a constant per key. Insert side just
/// serialises the raw values (the engine aggregates on merge).
#[derive(Debug, Clone, Row, Serialize)]
pub struct AssetSacRow {
    pub asset_type: i16,
    pub asset_code: String,
    pub issuer_id: i64,
    pub contract_id: i64,
    /// cityhash64 surrogate of the SAC's `C…` StrKey.
    pub sac_contract_id: i64,
    /// Deployed on-chain? Serialised as `UInt8` (0/1) to match the CH column.
    pub sac_deployed: u8,
}

/// `asset_enrichment` — off-chain SEP-1 enrichment side table (task 0231),
/// RMT(version). Written by the enrichment worker, NOT the indexer; keyed
/// byte-for-byte like `assets` and joined at read. `version` =
/// `DateTime64(3, 'UTC')` (ms since epoch); a higher version wins, so the
/// worker can refresh or CLEAR (re-insert NULL with a newer `version`).
#[derive(Debug, Clone, Row, Serialize)]
pub struct AssetEnrichmentRow {
    pub asset_type: i16,
    pub asset_code: String,
    pub issuer_id: i64,
    pub contract_id: i64,
    pub icon_url: Option<String>,
    pub name: Option<String>,
    pub version: i64,
}

// `account_balances_current` — table retained (pending classic→`balances`
// migration + rollback) but NO LONGER WRITTEN (lore-0331 Option A single-write):
// its `AccountBalanceRow` write struct was removed. Classic + native balances now
// stage straight into the unified `balances` table (`BalanceRow`); reads already
// resolve there.

/// `soroban_contracts` — state hub, RMT(wasm_uploaded_at_ledger).
/// Surrogate `id`; `wasm_uploaded_at_ledger = 0` is the stub sentinel
/// (Pass 2 stub-rowing for referenced-but-not-deployed contracts).
///
/// Also read back (not just written): the task-0320 live WASM-upgrade prefetch
/// (`persist::fetch_prior_contract_rows`) reads the prior row in full so
/// `build_wasm_upgrade_rows` can carry the identity columns forward — hence
/// `Deserialize`.
#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct SorobanContractRow {
    pub id: i64,
    pub contract_id: String,
    pub wasm_hash: Option<[u8; 32]>,
    pub wasm_uploaded_at_ledger: i64,
    pub deployer_id: Option<i64>,
    pub deployed_at_ledger: Option<i64>,
    pub contract_type: Option<i16>,
    pub is_sac: bool,
}

/// `soroban_contract_metadata` — on-chain Soroban token metadata
/// (name/symbol/decimals) from the instance-storage `Symbol("METADATA")`
/// struct. RMT(version); `version` = observed ledger (latest wins). Per
/// `contract_id`; SACs are excluded by the producer
/// (`xdr_parser::extract_contract_metadata_writes`). Separate table — never
/// columns on `soroban_contracts` — to dodge the RMT whole-row clobber across
/// that table's many writers (deploy / rebuild EXCHANGE / stubs / db-merge).
/// See task 0297.
#[derive(Debug, Clone, Row, Serialize)]
pub struct SorobanContractMetadataRow {
    pub contract_id: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u32>,
    pub version: i64,
}

/// `balances` — unified per-holder balance (task 0331, Option C). RMT(version);
/// `last_updated_ledger` = observed ledger; removed/zeroed → `amount = 0`.
/// `holder_id` = `cityhash64(holder StrKey)` (one surrogate space with
/// `accounts.id` / `soroban_contracts.id`; resolve back to a StrKey via `accounts`
/// (G) / `soroban_contracts` (C)); `asset_id` = `ids::asset_id` (→ `assets.id`);
/// `amount` raw `Int128` (scale by the asset's decimals at read). Replaces
/// `soroban_token_balances` + (after step 6) classic `account_balances_current`.
#[derive(Debug, Clone, Row, Serialize)]
pub struct BalanceRow {
    pub holder_id: i64,
    pub asset_id: i64,
    pub amount: i128,
    pub last_updated_ledger: i64,
    /// ADR 0055 — 0 while the holding relationship is live, otherwise the
    /// ledger in which the ledger entry disappeared. `amount = 0` alone cannot
    /// carry this: a live-but-empty holding writes exactly the same amount.
    pub closed_at_ledger: i64,
}

/// `nfts` — state, RMT(current_owner_ledger). Composite PK
/// = (contract_id, token_id). No surrogate id.
#[derive(Debug, Clone, Row, Serialize)]
pub struct NftRow {
    pub contract_id: i64,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub minted_at_ledger: Option<i64>,
    pub current_owner_id: Option<i64>,
    pub current_owner_ledger: i64,
}

/// `nfts_pending` — task 0217 quarantine for NFT-candidate rows whose
/// contract is still `Other`/NULL-classified. Same shape as
/// [`NftRow`]; routed via `stage::prepare` based on the per-contract
/// `wasm_classification` verdict. Promoted to hot `nfts` via the
/// post-backfill drain runbook
/// (`docs/runbooks/0217_nfts_pending_migration_and_drain.md`) — CH
/// has no per-row UPDATE / `WHERE NOT EXISTS` equivalent to PG's
/// in-tx `promote_pending_nfts_to_hot` step.
#[derive(Debug, Clone, Row, Serialize)]
pub struct NftPendingRow {
    pub contract_id: i64,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub minted_at_ledger: Option<i64>,
    pub current_owner_id: Option<i64>,
    pub current_owner_ledger: i64,
}

/// `nft_enrichment` — off-chain `token_uri` enrichment side table (task 0231),
/// RMT(version). Same rationale as [`AssetEnrichmentRow`]: written by the
/// enrichment worker (not the indexer), keyed like `nfts`, joined at read;
/// `version` is the worker's own clock (`DateTime64(3, 'UTC')`, ms),
/// independent of the `nfts` ownership clock.
#[derive(Debug, Clone, Row, Serialize)]
pub struct NftEnrichmentRow {
    pub contract_id: i64,
    pub token_id: String,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub collection_name: Option<String>,
    pub version: i64,
}

/// `liquidity_pools` — state, RMT(last_updated_ledger). PK = pool_id.
/// `created_at_ledger` removed (derive read-time from snapshots).
#[derive(Debug, Clone, Row, Serialize)]
pub struct LiquidityPoolRow {
    pub pool_id: [u8; 32],
    pub asset_a_type: i16,
    pub asset_a_code: String,
    pub asset_a_issuer_id: i64,
    pub asset_b_type: i16,
    pub asset_b_code: String,
    pub asset_b_issuer_id: i64,
    pub fee_bps: i32,
    pub last_updated_ledger: i64,
    /// 0 = classic (pool_id: CAP-38 hash), 1 = soroban contract (pool_id:
    /// the 32-byte payload of the C… address). Registry columns below are
    /// meaningful only for kind 1; classic writers set the defaults.
    pub pool_kind: u8,
    /// One surrogate per leg, in a PER-KIND id space (`pool_kind` says
    /// which): kind 1 = token-CONTRACT surrogates (`ids::contract_id`) in
    /// emission order, matching the pool's own `get_tokens()` so reserve
    /// vectors align index-for-index; kind 0 = ASSET surrogates
    /// (`ids::pool_leg_asset_id` — the `lp_operation_amounts` join key),
    /// legs-migration step 2 towards retiring the pair columns.
    ///
    /// NOT `assets.id` in general (an earlier comment claimed that): the two
    /// coincide only for bespoke type-3 tokens. 96% of legs are SACs, whose
    /// classic asset has a DIFFERENT id — a leg resolves to its display
    /// identity via `asset_sac` (`resolve_leg_assets`, task 0374 step 13).
    pub legs: Vec<i64>,
    /// Surrogate of the registering router contract. Venue labels resolve
    /// from this id at read time — no label is stored on the pool.
    pub deployment_id: i64,
    /// Verbatim `pool_type` sym from `add_pool`; deliberately un-normalised.
    pub pool_type_raw: String,
}

/// `pool_state_changes` — pool reserve state, ONE deterministic row per
/// `(pool, plane, ledger)` (task 0374; grain aligned with the classic
/// snapshots by decision karolkow 2026-08-30; the plane joined the key so a
/// forged plane write cannot evict a genuine row — see the DDL comment).
/// The collapse happens at parse time in ledger apply order
/// (`fold_pool_state_changes`, the cross-writer twin of
/// `dedup_final_pool_snapshots`), so no intra-ledger ordering column is
/// needed and the 0356 LIMIT-1/no-FINAL invariant holds here too.
/// Intra-ledger history stays reconstructible from `soroban_events`
/// (`update_reserves` per action, permanent) — storing intermediates
/// duplicated it; an earlier per-write design needed an `application_order`
/// key component and was collapsed away before any production DDL existed.
///
/// Two on-chain layouts feed it: fungible pools' plane `PoolData` vector
/// VERBATIM (possibly a per-tick tail — readers slice by the pool's leg
/// count, never vector length) and concentrated pools' own-instance
/// `Reserve0`/`Reserve1`.
#[derive(Debug, Clone, Row, Serialize)]
pub struct PoolStateChangeRow {
    pub pool_id: [u8; 32],
    pub ledger_sequence: i64,
    pub reserves: Vec<i128>,
    /// The plane contract that wrote these reserves — the provenance a read
    /// checks against the pool's own declared plane (review #438). For the
    /// concentrated arm the pool writes its own reserves, so this carries the
    /// plane the instance declares.
    pub plane_id: i64,
}

/// `pool_instance_state` — what a pool declares ABOUT ITSELF, read from its
/// own instance storage (task 0374 step 15; extended by review #438).
///
/// The pool contract is the ledger-authenticated OWNER of that entry, so
/// nothing here can be forged by a third party — which is what makes
/// `plane_id` usable as the reserve-provenance authority.
///
/// A SIDE table, deliberately (the `asset_sac` pattern): a partial row in the
/// RMT registry would clobber the full registration on merge, and these facts
/// move on a different clock than the registration. Versioned by sighting
/// ledger, so a share-token migration converges on the newest — exactly what
/// `share_id()` returns on chain.
#[derive(Debug, Clone, Row, Serialize)]
pub struct PoolInstanceStateRow {
    pub pool_id: [u8; 32],
    /// The plane this pool reports its reserves to. Always populated: an
    /// instance is only recognised as a pool when it carries `Plane`.
    pub plane_id: i64,
    /// `0` is STRUCTURAL for concentrated pools, which never mint a share
    /// token (`share_id()` returns the pool itself).
    pub share_token_id: i64,
    /// Outstanding share supply, raw u128 from the instance's `TotalShares`
    /// (same entry, same pass, same clock as the share token — the write half
    /// of the soroban "Total shares" gap). `0` where the key is absent:
    /// structural for concentrated pools and the elastic layout.
    pub total_shares: i128,
    pub derived_at_ledger: i64,
}

/// `lp_positions` — state, RMT(last_updated_ledger).
#[derive(Debug, Clone, Row, Serialize)]
pub struct LpPositionRow {
    pub pool_id: [u8; 32],
    pub account_id: i64,
    pub shares: i128,
    pub first_deposit_ledger: i64,
    pub last_updated_ledger: i64,
    /// ADR 0055 — see [`BalanceRow::closed_at_ledger`]. A withdrawn position
    /// and a position still open at zero shares both wrote `shares = 0`.
    pub closed_at_ledger: i64,
}

/// `transactions` — append-only fact hub, surrogate `id`,
/// ORDER BY (ledger_sequence, application_order).
#[derive(Debug, Clone, Row, Serialize)]
pub struct TransactionRow {
    pub id: i64,
    pub hash: [u8; 32],
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_id: i64,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<[u8; 32]>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub parse_error: bool,
}

/// `transaction_hash_index` — fact, backs `transaction_hash_dict`.
#[derive(Debug, Clone, Row, Serialize)]
pub struct TransactionHashIndexRow {
    pub hash: [u8; 32],
    pub ledger_sequence: i64,
}

/// `operations_appearances` — fact, no surrogate id. ORDER BY
/// (ledger_sequence, transaction_id, application_order).
#[derive(Debug, Clone, Row, Serialize)]
pub struct OperationAppearanceRow {
    pub transaction_id: i64,
    pub application_order: i16,
    /// `type` is a Rust keyword — serde rename keeps the CH column
    /// match clean.
    #[serde(rename = "type")]
    pub op_type: i16,
    pub source_id: Option<i64>,
    pub destination_id: Option<i64>,
    pub contract_id: Option<i64>,
    pub asset_code: String,
    pub asset_issuer_id: Option<i64>,
    /// Crossed liquidity pools, sorted + deduped (canonical order — see the
    /// stage fold). Empty = no pool involvement; `[]` replaces the legacy
    /// scalar NULL (task 0261/0268).
    pub pool_ids: Vec<[u8; 32]>,
    pub amount: i64,
    pub ledger_sequence: i64,
}

/// `transaction_participants` — fact.
#[derive(Debug, Clone, Row, Serialize)]
pub struct TransactionParticipantRow {
    pub account_id: i64,
    pub ledger_sequence: i64,
    pub transaction_id: i64,
}

/// `operation_asset_appearances` — fact, the per-(asset, transaction) presence
/// index (task 0359). The EXACT `transaction_participants` shape with `asset_id`
/// in place of `account_id` → a per-asset activity page is a PK-prefix seek.
/// Native XLM is a first-class key (`ids::asset_id(0,"",0,0)`), never an empty
/// sentinel. Pure presence: which assets a transaction touched; duplicate
/// (asset, tx) rows collapse in the RMT.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct OperationAssetAppearanceRow {
    pub asset_id: i64,
    pub ledger_sequence: i64,
    pub transaction_id: i64,
}

/// `operation_pools` — fact, the per-(pool, transaction) presence index
/// (task 0365). The EXACT `transaction_participants` shape with `pool_id` in
/// place of `account_id` → a per-pool tx-list is a PK-prefix seek. `pool_id` is
/// the raw 32-byte pool hash (already how `operations_appearances.pool_ids`
/// stores each crossing — no surrogate). Pure presence: which pools a
/// transaction crossed; duplicate (pool, tx) rows collapse in the RMT.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct OperationPoolRow {
    pub pool_id: [u8; 32],
    pub ledger_sequence: i64,
    pub transaction_id: i64,
}

/// `lp_operation_amounts` — fact, what one operation moved through one pool
/// (task 0279). The value twin of [`OperationPoolRow`]: same pool-leading key,
/// plus `application_order` / `asset_id` / `amount`.
///
/// One row per (operation, pool, asset) — the op's claim atoms are SUMMED into
/// it, never written per atom: an op can take the same pool several times
/// (CAP-38 interleaved matching) and those atoms share the whole ORDER BY
/// tuple, so per-atom rows would have the RMT keep one and drop the rest.
///
/// `amount` is raw stroops SIGNED FROM THE POOL'S SIDE — positive = the asset
/// entered the pool, negative = it left. So one shape says trade (`+/-`),
/// deposit (`+/+`) and withdrawal (`-/-`) without an event-type column.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct LpOperationAmountRow {
    pub pool_id: [u8; 32],
    pub ledger_sequence: i64,
    pub transaction_id: i64,
    pub application_order: i16,
    pub asset_id: i64,
    pub amount: i64,
}

/// `asset_transfers` — fact, one row per token movement (task 0540). Keyed
/// by Stellar's official event identity `(ledger, tx, op, event-in-op)`;
/// `event_index` is our flat counter and only joins `soroban_events`.
/// `amount` is `NULL` for exactly one reason: a non-fungible movement.
/// `from_id`/`to_id` are `NULL` for mint / burn+clawback respectively, and
/// always the surrogate of the underlying `G…` (an `M…` is split into
/// `*_id` + `*_muxed_id`). Column order matches `init.sql` byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct AssetTransferRow {
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub op_index: i16,
    pub event_pos_in_op: i16,
    pub event_index: i16,
    pub asset_id: i64,
    pub amount: Option<i128>,
    pub from_id: Option<i64>,
    pub from_kind: String,
    pub from_muxed_id: Option<u64>,
    pub to_id: Option<i64>,
    pub to_kind: String,
    pub to_muxed_id: Option<u64>,
    pub verb: String,
}

/// `transaction_memos` — fact, one row per transaction that carries a memo
/// (task 0540). A memo belongs to the envelope, not to a transfer, so it is
/// stored once per transaction rather than on every edge row. `memo` is the
/// text, the id as decimal, or the hash/return as hex — the same rendering
/// `ExtractedTransaction.memo` already uses.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct TransactionMemoRow {
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub memo_type: String,
    pub memo: String,
}

/// `soroban_event_ops` — narrow side table (task 0541): which operation
/// emitted each event, keyed like `soroban_events`. Only per-operation
/// events have a row; tx-level and diagnostic events have no operation and
/// are absent rather than null.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize)]
pub struct SorobanEventOpRow {
    pub ledger_sequence: i64,
    pub transaction_id: i64,
    pub event_index: i16,
    pub op_index: i16,
    pub event_pos_in_op: i16,
}

/// `soroban_events` — fact, full-content per-event row (ADR 0044
/// §4a unfold). `signature` is the lifted first-topic Symbol.
#[derive(Debug, Clone, Row, Serialize)]
pub struct SorobanEventRow {
    pub contract_id: i64,
    pub transaction_id: i64,
    pub ledger_sequence: i64,
    pub event_index: i16,
    pub event_type: i16,
    pub signature: Option<String>,
    pub topics_xdr: String,
    pub data_xdr: String,
}

/// `soroban_invocations_appearances` — fact (ADR 0034 fold).
#[derive(Debug, Clone, Row, Serialize)]
pub struct SorobanInvocationAppearanceRow {
    pub contract_id: i64,
    pub transaction_id: i64,
    pub ledger_sequence: i64,
    pub caller_id: Option<i64>,
    pub caller_contract_id: Option<i64>,
    pub amount: i32,
}

/// `nft_ownership` — fact, no surrogate. ORDER BY
/// (contract_id, token_id, ledger_sequence, event_order).
#[derive(Debug, Clone, Row, Serialize)]
pub struct NftOwnershipRow {
    pub contract_id: i64,
    pub token_id: String,
    pub ledger_sequence: i64,
    pub event_order: i16,
    pub transaction_id: i64,
    pub owner_id: Option<i64>,
    pub event_type: i16,
}

/// `nft_ownership_pending` — task 0217 quarantine companion to
/// [`NftOwnershipRow`]. Same row shape + same partitioning as the hot
/// `nft_ownership` table so promotion (`INSERT … SELECT FROM
/// nft_ownership_pending`) is a clean part copy. Routed by the same
/// per-contract classifier verdict as [`NftPendingRow`].
#[derive(Debug, Clone, Row, Serialize)]
pub struct NftOwnershipPendingRow {
    pub contract_id: i64,
    pub token_id: String,
    pub ledger_sequence: i64,
    pub event_order: i16,
    pub transaction_id: i64,
    pub owner_id: Option<i64>,
    pub event_type: i16,
}

/// `liquidity_pool_snapshots` — fact, no surrogate. ORDER BY
/// (pool_id, ledger_sequence).
#[derive(Debug, Clone, Row, Serialize)]
pub struct LiquidityPoolSnapshotRow {
    pub pool_id: [u8; 32],
    pub ledger_sequence: i64,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    /// Gross trade volume in asset-A units per (pool, ledger), from
    /// path-payment/offer claim atoms. NULL until the 0266 backfill / 0247
    /// wiring writes it (live ingest leaves it NULL today). Task 0261/0268.
    pub gross_volume_a: Option<i128>,
}
