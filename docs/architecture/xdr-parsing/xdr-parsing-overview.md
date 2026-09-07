# Stellar Block Explorer - XDR Parsing Overview

> This document expands the XDR parsing portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same parsing scope and decode/storage assumptions, but specifies the
> model in more detail so it can later serve as input for implementation task planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Architectural Role](#2-architectural-role)
3. [Parsing Strategy](#3-parsing-strategy)
4. [Data Extracted from XDR](#4-data-extracted-from-xdr)
5. [Soroban-Specific Handling](#5-soroban-specific-handling)
6. [Storage Contract](#6-storage-contract)
7. [Error Handling and Compatibility](#7-error-handling-and-compatibility)
8. [Boundaries and Delivery Notes](#8-boundaries-and-delivery-notes)

---

## 1. Purpose and Scope

XDR parsing is the translation layer between canonical Stellar ledger payloads and the
structured explorer data model stored in ClickHouse and served by the backend API.

This document covers the current XDR parsing design. It does not redefine frontend
behavior, backend transport contracts, or the full database schema except where those are
needed to explain decode responsibilities and storage outcomes.

The parsing implementation lives in `crates/xdr-parser/` (shared between the ingest
Lambda and the backend API).

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
main overview document takes precedence. This file is an XDR-parsing-focused refinement of
that source, not an independent redesign — kept in sync with the code per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md).

## 2. Architectural Role

The block explorer relies on canonical `LedgerCloseMeta` XDR as its only required chain
input. XDR parsing exists to turn that low-level payload into explorer-friendly read models
without relying on Horizon, Soroban RPC, or any third-party explorer API.

The parsing layer has four jobs:

- decode canonical Stellar payloads into typed summary records + appearance indexes
  at ingestion time (ingest path)
- re-decode heavy-field XDR at request time for the two detail endpoints that need
  it (E3, E14 per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)),
  fetching `.xdr.zst` from the public Stellar ledger archive
- extract Soroban-specific structures such as invocation trees, events, and contract
  metadata
- keep frontend and normal API responses free from protocol-level decode work

The parser itself is shared: both the ingest Lambda and the backend API link
`crates/xdr-parser`. The parsing layer is not a generic XDR inspection service for
arbitrary clients. Its main purpose is to feed the explorer's own storage and read
paths.

## 3. Parsing Strategy

### 3.1 Two Parsing Paths, One Rust Parser

> Per [ADR 0004](../../../lore/2-adrs/0004_rust-only-xdr-parsing.md): Rust-only XDR
> parsing — the shared `crates/xdr-parser` crate is the single decoder.
> Per [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md):
> raw XDR is not stored in ClickHouse; heavy-field endpoints re-parse from the public
> Stellar ledger archive at read time.

**Ingest path (Ledger Processor Lambda).** Every ledger's `LedgerCloseMeta` is
fully deserialized with `stellar-xdr` via `crates/xdr-parser`. The ingest extracts
typed summary columns + appearance-index rows (per
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md) /
[ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md) /
[ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md))
and commits them in a single atomic per-ledger DB transaction via the 14-step
`persist_ledger` method.

**Read path (axum API).** For two endpoints the API fetches the relevant
`.xdr.zst` from the public archive on demand, decompresses it with zstd, and
re-parses with the same shared `crates/xdr-parser`:

- **E3 `/transactions/:hash`** — fetches envelope + result-meta to expand the
  operation list into a full invocation tree, render decoded events, and carry
  `envelope_xdr` / `result_xdr` / `result_meta_xdr` in the advanced view
- **E14 `/contracts/:id/events`** — fetches result-meta for every appearance
  row returned by `soroban_events_appearances` to produce decoded event detail

List endpoints never invoke the parser at read time — they answer from typed
summary columns and appearance indexes.

Using one parser crate for both paths means no dual-language sync on protocol
upgrades and no decode drift between ingest and read.

### 3.2 What Is Not Stored

Per ADR 0029 the following are **not** stored in ClickHouse:

- `envelope_xdr`, `result_xdr`, `result_meta_xdr` as strings or blobs on the
  `transactions` row
- decoded invocation-tree JSONB (`transactions.operation_tree` does not exist)
- full decoded event payload (no `soroban_events` JSONB table — only the
  `soroban_events_appearances` index)
- per-node invocation detail (no `soroban_invocations` row per node — only
  the `soroban_invocations_appearances` index)

All of these are re-derived at request time from the public archive by the
read-path code in §3.1.

### 3.4 Frontend Parsing Boundary

The frontend is not expected to parse XDR for normal explorer operation.

The frontend receives pre-decoded data for:

- transaction summaries
- operation lists and details
- Soroban invocations
- Soroban events
- account, asset, NFT, and pool views derived from indexed chain state

Raw XDR is exposed only for advanced transaction inspection.

## 4. Data Extracted from XDR

### 4.1 Ledger Header

From the enclosing `LedgerHeaderHistoryEntry` and its `LedgerHeader`, the
parsing layer extracts:

- `hash` — the canonical Stellar ledger hash, taken **directly** from
  `LedgerHeaderHistoryEntry.hash` (already populated by stellar-core).
  Never recomputed by the parser — that is the value Horizon
  (`/ledgers/:N.hash`) and every other Stellar tool publishes
- `sequence`
- `closeTime`
- `protocolVersion`
- `baseFee`
- `txSetResultHash`

These fields anchor ledger ordering, freshness checks, and high-level network statistics.

### 4.2 Transaction Envelope and Result

From `TransactionEnvelope` and `TransactionResult`, the ingest path extracts typed
summary columns:

- `hash`, stored as `BYTEA(32)`
  ([ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md))
- `source_id`, resolved from the source StrKey to `accounts.id`
  ([ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md))
- `fee_charged`, `successful`
- `application_order`, `operation_count`, `has_soroban`, `inner_tx_hash` (fee-bump)
- `result_code` is not persisted at ingest; it is re-derived on demand from
  the archive for the advanced view

Raw envelope / result / result-meta XDR is **not** retained in ClickHouse (ADR 0029).
The advanced transaction view pulls the corresponding `.xdr.zst` from the public
archive at request time.

### 4.3 Operation-Level Data (Appearance Index)

Per task 0163, `operations` was collapsed to an appearance index and renamed
to `operations_appearances`. Ingest aggregates operations by identity at
staging time (`HashMap<OpIdentity, i64>`), writing one row per distinct
identity per transaction with `amount BIGINT` counting collapsed duplicates.

From `OperationMeta` per transaction, the ingest path extracts:

- operation `type` as `SMALLINT` backed by the Rust `OperationType` enum
  ([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
- `source_id`, `destination_id` surrogate FKs (ADR 0026)
- `contract_id` surrogate FK
  ([ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md))
- `asset_code`, `asset_issuer_id`, `pool_id` (BYTEA 32; CH: `pool_ids`
  Array — see below)
- `ledger_sequence`, `created_at`
- `amount` aggregate count of physical operations collapsed into this identity

**Liquidity-pool attribution (task 0261).** LP deposit/withdraw ops carry
their pool id in the operation body. Path payments
(`path_payment_strict_send` / `path_payment_strict_receive`) and offer ops
(`manage_sell_offer` / `manage_buy_offer` / `create_passive_sell_offer`) do
**not** — the pools they cross are only visible in the `OperationResult`
success branch as `ClaimAtom::LiquidityPool` entries (path payments expose
`offers`, offers expose `ManageOfferSuccessResult.offers_claimed`; a single
op can fill against both the order book and an AMM, CAP-38). The parser
unwraps the per-op results (`tx_op_results`, including the fee-bump inner
nesting) and, for each such successful op, appends to the op details:

- `poolIds` — deduped list of crossed pools (hex), and
- `claimedAtoms` — every LP fill with `poolId`, `assetSold`/`amountSold`,
  `assetBought`/`amountBought`, so per-(pool, ledger) `gross_volume_a`
  can be computed downstream without a second parse pass (tasks
  0247/0266/0199).

`tx_op_results` returns op results only for **successful** transactions: a
failed transaction rolls every op back, yet an op that executed before the
failing one still shows op-level `Success` with claim atoms — gating on tx
success keeps those phantom crossings out of `pool_ids` and
`gross_volume_a`. Failed (or order-book-only) ops therefore contribute no
pool attribution. Its companion `tx_op_results_any` unwraps the failed arms
too (`TxFailed` carries the same per-op array) — consumed only by the
request-time heavy path, where `op_result_code` names each op's result with
the XDR library's own variant names (`"LowReserve"`, `"OpNoAccount"`, …) and
the API surfaces it as `operations[].result_code`, the fail-reason source
for the transaction page (task 0352). Claim-atom extraction never reads it. The CH writer folds `poolIds` into
`operations_appearances.pool_ids Array(FixedString(32))` (sorted + deduped;
one row per op identity — task 0268); the PG store keeps the legacy scalar
`pool_id`, where these ops remain NULL pending PG retirement.
`/liquidity-pools/:id/transactions` on CH therefore surfaces path payments
and offers that crossed the pool, matching Horizon.

Not stored at ingest (re-derived from XDR at read time per ADR 0029):
`transfer_amount` (dropped), `application_order` (dropped), per-op JSONB
`details` (never existed), envelope/args/memo/predicates decode.

For `INVOKE_HOST_FUNCTION`, ingest captures only the appearance-index rows
(§4.4 / §4.5). The `functionName`, decoded `functionArgs`, `returnValue`, and
per-node invocation tree are re-expanded at request time from the archive by
the E3 read path.

### 4.4 Soroban Event Data (Ingest: Appearance Index)

From `SorobanTransactionMeta.events`, the ingest path extracts one
**appearance-index row** per `(contract, tx, ledger)` trio in
`soroban_events_appearances` with:

- `contract_id` surrogate FK (ADR 0030), `transaction_id`, `ledger_sequence`,
  `created_at`
- `amount` = count of consensus events in the trio (tx-level + per-op
  sources only — the entire `*.diagnostic_events` container is dropped
  at staging by source, see §5.1; task 0182)

Full decoded event detail (`eventType` as `SMALLINT`, `topics` as decoded
`ScVal[]`, `data` as decoded `ScVal`) is **not** stored. Known NFT-related
event patterns are still interpreted at ingest into derived state updates on
`nfts` / `nft_ownership` / `assets` (classification happens by looking at the
events without persisting them).

At read time, `xdr_parser::extract_events` re-expands the decoded payload from
the archive for E14 `/contracts/:id/events`.

### 4.5 Soroban Invocations (Ingest: Appearance Index)

Mirroring §4.4, ingest writes one row per `(contract, tx, ledger)` trio to
`soroban_invocations_appearances` with:

- surrogate `contract_id`, `transaction_id`, `ledger_sequence`, `created_at`
- `caller_id` — the root-level caller `accounts.id`, NULL for C-contract
  sub-invocation callers
- `amount` = count of invocation-tree nodes in the trio

Per-node decode (function name, args, return value, depth) happens at read
time in `xdr_parser::extract_invocations` for E3 and E11 /
E-contract-invocations endpoints.

### 4.6 Ledger Entry Changes

From `LedgerEntryChanges`, the parser extracts derived state used by explorer
entities:

- contract deployments → `soroban_contracts` row (contract_id surrogate, wasm_hash
  BYTEA 32, deployer_id surrogate, is_sac, contract_type SMALLINT). The
  `deployer_id` value is the **operation-level effective source** of the
  `CreateContract*` host function (`op.source_account` override or inner
  `tx.source_account` fallback; fee-bump `feeSource` is never used). For
  factory-pattern deploys nested in an `InvokeContract` auth tree, the
  deployer is the signer of the enclosing `SorobanAuthorizationEntry`.
  `xdr_parser::extract_op_source_per_contract` produces the per-contract
  override map consumed by `extract_contract_deployments`. Task 0255
  Phase 1; pre-fix the parser stored the inner-tx source unconditionally
  and misattributed deploys with per-op overrides
- contract WASM upgrade → re-emitted `soroban_contracts` row (task 0320).
  `extract_contract_deployments` records `wasm_hash` only on the `created`
  instance, so a later executable swap on an `updated` ContractInstance was
  dropped and the row kept its stale deploy-time hash + verdict forever.
  `state::extract_contract_wasm_upgrades` scans `updated` instances for the new
  `wasm_hash` (SACs carry no hash → skipped). The CH writer does a
  read-modify-write: it pre-fetches the prior row's identity
  (`persist::fetch_prior_contract_rows`) and re-emits a full row with the new
  `wasm_hash`, `wasm_uploaded_at_ledger` bumped to the upgrade ledger (wins the
  RMT `ORDER BY (contract_id)` collapse), the verdict re-derived from the new
  hash, and deployer / deploy-ledger / name carried forward unchanged — so the
  whole-row RMT replace never clobbers deploy identity (the naive filter-flip
  rejected in 0283). The classification cache is evicted for upgraded
  `contract_id`s so the new verdict takes effect.
- contract token metadata → `soroban_contract_metadata` side table (ClickHouse,
  task 0297). `name` / `symbol` / `decimals` are read from the contract instance
  entry's metadata struct (`{decimal?, name, symbol}`) via
  `token_metadata::extract_token_metadata`, collected by
  `state::extract_contract_metadata_writes` on `created` + `updated` instance
  changes (SACs skipped — derivable from the SAC identity). Two on-chain key
  shapes are matched (`token_metadata::is_metadata_key`): fungible SEP-41 / OZ
  tokens use `Symbol("METADATA")`; OpenZeppelin **NFTs** use the
  `NFTStorageKey::Metadata` enum variant, which serializes as
  `Vec([Symbol("Metadata")])` — so an NFT collection name is captured straight
  from the ledger (lore-0340). The earlier `Symbol("METADATA")`-only match missed
  the NFT key: the false "0%" that had wrongly implied a `name()` RPC was needed.
  This also corrects the legacy assumption that token names are a standalone
  `Symbol("name")` entry — they are not (that path matched 0 contracts); the name
  lives nested in the metadata struct in instance storage, which
  `scval_to_typed_json` used to drop.
- WASM upload → `wasm_interface_metadata` row (SEP-48-derived JSONB, keyed by
  wasm_hash BYTEA)
- account state → `accounts` row + `account_balances_current` entries per
  trustline / native (balances are typed `NUMERIC(28,7)` per-asset rows, not a
  JSONB blob on `accounts`)
- classic LP state → `liquidity_pools` row + `liquidity_pool_snapshots` row +
  `lp_positions` upsert per participating account (asset pair modeled as typed
  `asset_*_type SMALLINT` + code + issuer_id, not JSONB)
- **classic-credit + native asset entity rows** → `assets` row per distinct
  `(asset_code, issuer)` pair observed in a `trustline` LedgerEntryChange
  (`xdr_parser::detect_classic_credit_assets`, task 0219). Native XLM is a
  per-ledger singleton emit (`xdr_parser::native_asset_singleton`) — the
  persist `WHERE NOT EXISTS` against `uidx_assets_native` keeps re-emit free.
  These paths complement `detect_assets`, which emits Soroban-native rows plus
  **folds a SAC deploy onto the underlying classic/native row** (see below).
  Without the dedicated classic-credit producer, `account_balances_current`
  would carry the balances but the entity row never existed (Karol's pre-audit
  Bug #1).
- **SAC is a facet of the classic/native asset, not a separate row** (ADR 0051 /
  task 0339). A SAC deploy (`detect_assets` SAC branch) and an un-deployed SAC
  seen via a CAP-67 event (`detect_undeployed_sac_overrides`, task 0323) both
  record the SAC handle (`sac_contract_id` = surrogate of the derived `C…`, +
  `sac_deployed`) keyed on the `classic_credit` / `native` identity in the
  `asset_sac` side table — NOT a distinct `asset_type = 2` row, and NOT columns on
  `assets` (which is re-written whole every ledger and would clobber them). The
  staging `push_sac` accumulator `max`-merges the facet per key (a deploy sighting
  beats a later un-deployed override), mirrored cross-ledger by the `asset_sac`
  AggregatingMergeTree. Override collection is crypto-gated
  (`sac_override_from_event_topics`, `emitter == derive_sac(asset)`, so a bespoke
  contract is never mislabeled) and suppresses the Pass-2 FK stub (**no
  `soroban_contracts` row**), so `soroban_contracts` holds **deployed instances
  only**. The `C…` StrKey itself is not stored — the read path re-derives it from
  `code:issuer` (`derive_sac_strkey`). The legacy PG path still flips `is_sac=true`
  on pre-window SAC skeletons (`apply_sac_overrides_for_skeleton_contracts`, task 0218) — being deprecated with PG.
- **SAC event gate at NFT detection** (task 0294) → a CAP-67 classic-asset SAC
  emits `transfer`/`mint`/`burn` under its deterministic `contract_id` carrying
  the SEP-11 asset `CODE:ISSUER` in the LAST topic and an i128 **amount** in data.
  `detect_nft_events` runs the shared `sac_override_from_event_topics` gate first
  (`emitter == derive_sac(asset)`); a crypto-proven SAC event is **skipped before
  it can be minted as an NFT candidate**, so a payment/transfer-only un-deployed
  SAC's amount is never mis-read as an NFT token_id. Stateless and per-event (no
  prior-row / no quarantine needed for this class). The gate returns `None` for
  bespoke emitters, so genuine NFTs are unaffected (false-negative-only).

This stage is where low-level ledger changes are translated into query-oriented
explorer records.

**Post-parse derivations not produced by the XDR parser itself** (covered in
[`indexing-pipeline-overview.md`](../indexing-pipeline/indexing-pipeline-overview.md) §5.2
step 14, called out here so the parser/indexer boundary stays explicit):

- `balance_aggregates.total_supply` / `.holder_count` — recomputed from
  `balances` by the refreshable `balance_aggregates_mv` (task 0293/0331), never
  by the parser and no longer on the `assets` row at all (those columns were
  dropped in task 0310). Per
  [ADR 0043](../../../lore/2-adrs/0043_field-allocation-rule.md) both are
  on-chain-derivable, hence indexer-owned. The parser only produces the
  per-trustline / per-balance rows; the aggregate is a downstream rollup.
- `liquidity_pool_snapshots.volume` / `fee_revenue` / `tvl` — per-op extraction
  half is on-chain (parser provides PathPayment `claimedOffers[].amount_sold`),
  USD denomination half is off-chain (Lambda 2 / price oracle). Consolidated
  under task 0199. The parser does NOT compute these aggregates today —
  columns stay NULL until 0199 ships.
- `assets.name` for classic credit — extracted from issuer's SEP-1 TOML by
  Lambda 2 (`sep1_assets` kind, task 0195 §2a). Parser produces only
  `asset_code` / `issuer_id` for classic credits; the human-readable name is
  off-chain and lives outside parser scope.
- `nfts.name` / `nfts.media_url` / `nfts.collection_name` — extracted from the
  NFT contract's `token_uri()` JSON by Lambda 2 (`nft_token_uri` kind,
  task 0195 §2d). Parser only writes the (`contract_id`, `token_id`,
  `current_owner_id`) tuple — see §5.1 NFT pattern.

### 4.7 Ledger balance deltas — the value reader

> The tx-list "Net settled" COLUMN was removed on 2026-09-04 (no storage column,
> no API field, no UI). The reader and reducer described here are KEPT: the
> authoritative per-(account, asset) LEDGER deltas are the input the lossless
> per-transfer replacement needs.

The reducer needs a single figure per (transaction, asset).
The protocol has no per-transaction amount — value lives on operations and Soroban
token events — so the parser derives the **net-settled value**:
`max(Σ positive account deltas, Σ negative account deltas)` per (tx, asset),
which nets out routing hops (a pass-through account ends at delta 0) instead of
double-counting them. The reducer is `xdr_parser::net_settled`
(`net_settled.rs`); its three rules — `max` of both sides (so burns / payments
-to-issuer stay non-zero), native canonicalised to one surrogate, fee excluded —
are covered in task 0393.

This figure is the network-flow **flow value**, not a heuristic: the flow
decomposition theorem splits any flow into source→sink **paths** plus **cycles**,
where a path contributes its flow and a **cycle contributes exactly zero**. Hence
`gross = Σ path + Σ cycle`, `net = Σ path`. A wash / round-trip is a pure cycle
and therefore nets to zero **by definition** (the same zero-balance-cycle
signature the wash-trading literature uses to detect washes), and two offsetting
but intent-wise unrelated payments decompose into a single path — the arithmetic
cannot see intent and does not try to. Net is preferred to gross because
`net ≤ gross` always: net never overstates, while gross inflates every routed
payment (3 hops of 100 read as 300), and routing is the common case. If a gross
figure is ever needed, `cycle volume = gross − net` falls out of the theorem.

A single **ledger** reader feeds it, for EVERY tx (classic and Soroban):

- `xdr_parser::ledger_balance_deltas` (`ledger_value.rs`) reads the before→after
  balance changes on `AccountEntry` / `TrustLineEntry` / `ContractData` from
  `TransactionMeta` (via the version-safe `meta.rs` change accessor). Every value
  flow — payment, path payment, offer/DEX fill, LP deposit/withdraw,
  claimable-balance create/claim, clawback, **and** Soroban SAC / bespoke-token
  transfers (which settle as `ContractData` `Balance` changes) — is an
  account / trustline / contract balance change, so this one reader covers them
  all and auto-nets. Token EVENTS are contract-emitted logs and are **never** used
  for value (any contract can emit any `"transfer"` it likes); a ledger balance
  cannot be forged. The fee is charged in the ledger's separate `feeProcessing`
  phase, not in `TransactionMeta`, so it is excluded by construction.

Surrogate resolution and the net reduction run at ingest
(`db_clickhouse::persist::stage`), which writes the result to
memory only — since 2026-09-04 there is no storage column for it.

## 5. Soroban-Specific Handling

### 5.1 CAP-67 Events

CAP-67 contract events follow the **appearance-index + read-time decode** pattern
per [ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md):

- at ingest, one row per `(contract, tx, ledger)` trio is written to
  `soroban_events_appearances` with a non-diagnostic-event count — no decoded
  event type, topics, or data are persisted
- at read time, E14 re-parses the archive via `xdr_parser::extract_events` and
  renders decoded `ScVal` topics / data per event
- known NFT / SEP-41 patterns are still interpreted at ingest to drive
  `assets` / `nfts` / `nft_ownership` upserts, but the triggering events
  themselves are not retained as rows

#### V3 vs V4 meta dispatch (Protocol 22 ↔ Protocol 23+)

`xdr_parser::extract_events` dispatches on the `TransactionMeta` variant
because Protocol 23 (CAP-67) reorganised the on-chain event surface
([ADR 0002](../../../lore/2-adrs/0002_rust-ledger-processor-lambda.md) §1):

- **V3** (`TransactionMetaV3`, Protocol ≤ 22): all Soroban contract events
  are at `soroban_meta.events`; diagnostic events at
  `soroban_meta.diagnostic_events`. The parser reads both.
- **V4** (`TransactionMetaV4`, Protocol ≥ 23): events live in **three**
  locations and the parser reads all three in this order:

  1. `tx_meta.events` (`VecM<TransactionEvent>`) — transaction-level
     events: fee `BeforeAllTxs` charge, `AfterTx` refund, `AfterAllTxs`.
  2. `tx_meta.operations[i].events` (`OperationMetaV2.events: VecM<ContractEvent>`) —
     per-operation events: Soroban contract events emitted during
     `InvokeHostFunction` execution **and** SAC `transfer` / `mint` / `burn`
     events emitted by classic operations under Protocol 23 unification.
  3. `tx_meta.diagnostic_events` (`VecM<DiagnosticEvent>`) — host-level
     diagnostic / trace events.

  `SorobanTransactionMetaV2` (the V4 `soroban_meta`) no longer carries an
  `events` field — that field was removed in CAP-67. `event_index` is
  numbered sequentially across all three sources within a single
  transaction so the V3 contract (monotonic per-tx index) is preserved.

The split matters because per-operation events carry the bulk of
post-Protocol 23 Soroban traffic. Missing them produces a silently
incomplete `soroban_events_appearances` index for every Protocol ≥ 23
ledger — the canonical symptom is a Soroban tx with exactly two events,
both XLM-SAC fee events at the tx-level location, while the contract's
own `transfer` / `mint` / `burn` events (which lived under
`operations[i].events`) are dropped.

##### Source-container tagging (task 0182)

Every `ExtractedEvent` carries an `EventSource` discriminator —
`TxLevel`, `PerOp`, or `Diagnostic` — populated by the parser at the
extraction site. **The diagnostic_events container is dropped at staging
regardless of inner type**: the staging filter
(`crates/indexer/src/handler/persist/staging.rs`) routes on
`source == EventSource::Diagnostic`, not on inner `event_type`.

Why: when diagnostic mode is enabled (the default for archive-bound
captive-core like Galexie), `v4.diagnostic_events` **holds
byte-identical Contract-typed copies of every consensus per-op
Contract event** — the copy carries the same inner `type_ = Contract`
as the original. CAP-67 explicitly says diagnostic_events are
auxiliary, "not hashed into the ledger, and therefore are not part of
the protocol", so they must not contribute to the appearance index.
A type-based filter (`event_type == Diagnostic`) cannot tell the
original from the copy and silently double-counts. Container-based
filtering is the only reliable signal.

The same routing applies at read time: `split_events`
(`crates/api/src/runtime_enrichment/stellar_archive/extractors.rs`) and the
`/contracts/:id/events` handler (`crates/api/src/contracts/handlers.rs`)
both filter on `EventSource::Diagnostic` to suppress the duplicates when
rendering contract event lists. The host-VM Diagnostic-typed entries
(`fn_call`, `fn_return`, `core_metrics`, errors) drop out the same way.

Mapping per location (after task 0182):

| Source location                         | `EventSource` | Counts in `amount` |
| --------------------------------------- | ------------- | ------------------ |
| `v3.soroban_meta.events`                | `TxLevel`     | yes                |
| `v3.soroban_meta.diagnostic_events`     | `Diagnostic`  | no                 |
| `v4.events`                             | `TxLevel`     | yes                |
| `v4.operations[i].events`               | `PerOp`       | yes                |
| `v4.diagnostic_events` (any inner type) | `Diagnostic`  | no                 |

### 5.2 Return Values

Return values of `invokeHostFunction` are decoded from XDR `ScVal` into typed
representations (integer, string, address, bytes, map, list) by
`xdr_parser::extract_invocations` at **request time** — not at ingest.
Ingest only records the appearance-index row in
`soroban_invocations_appearances` (ADR 0034).

### 5.3 Invocation Tree

Complex Soroban transactions may contain nested contract-to-contract calls.

Per [ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md)
the parser's responsibilities are split:

- **ingest**: write an appearance row per trio with `amount` = node count and
  `caller_id` = root-level account caller (C-contract sub-callers collapsed to
  NULL by the `is_strkey_account` filter so that `COUNT(DISTINCT caller_id)`
  answers E11's `unique_callers` stat directly)
- **read**: E3's transaction-detail renderer pulls the `.xdr.zst` from the
  public archive, decodes the full tree with `xdr_parser::extract_invocations`,
  and returns it as the `operation_tree` field of the response

Raw `result_meta_xdr` is not persisted on `transactions` (ADR 0029); the
archive is the authoritative source.

### 5.4 Contract Interface Extraction

Public function signatures are extracted from contract WASM at deployment time
and stored in `wasm_interface_metadata.metadata` (keyed by `wasm_hash BYTEA(32)`),
deduplicated across every contract instance that shares the same WASM.

The same pass also derives the **mutability** bit (task 0327): the parser scans
the WASM's import section for the `update_current_contract_wasm` host fn (Soroban
env import module `"l"`, field `"6"`) and stores `metadata.upgradeable: bool`. A
contract can only replace its own code by calling that host fn, so importing it is
the authoritative "self-upgradeable" signal; its absence means the contract is
effectively immutable/frozen (there is no on-ledger immutability flag — CAP-0046).
Because the bit is keyed by `wasm_hash`, it re-resolves correctly after an upgrade
swaps a contract's `wasm_hash` (task 0320). The import-section walker is a small
hand-rolled LEB128 scan in `crates/xdr-parser/src/contract.rs`
(`wasm_imports_upgrade_fn`), reusing the same `read_leb128` reader as the custom-
section parser; it is validated against real mainnet WASM in
`crates/xdr-parser/tests/upgradeable_real_wasm.rs`.

`soroban_contracts.name VARCHAR(256)` (per
[ADR 0042](../../../lore/2-adrs/0042_soroban-contracts-typed-name-column.md))
carries the human-readable contract name extracted from the standard
`Symbol("name")` ContractData persistent storage entry. The parser's
`extract_contract_deployments` second pass (state.rs) populates the
field at deploy time when the storage init lands in the same ledger
(constructor pattern); `extract_contract_data_name_writes` plus the
indexer's `apply_contract_name_writes` helper covers the deploy-then-init
and re-init patterns by emitting a retroactive UPDATE on every ledger
that surfaces a `Symbol("name")` Created or Updated event. The earlier
JSONB-backed metadata column was retired by ADR 0042 in favour of the
typed shape; richer metadata enrichment (description, icon, home_page)
landed off-row per [ADR 0023](../../../lore/2-adrs/0023_tokens-typed-metadata-columns.md)
narrowing rather than as further JSONB fields.

This extraction is part of the broader XDR/protocol decode pipeline because it
turns deployment-related protocol artifacts into stable explorer-facing contract
metadata.

### 5.5 NFT Event Shapes (`detect_nft_events`)

NFT mint/transfer/burn events are matched by their first topic Symbol
(`transfer`/`mint`/`burn`, case-insensitive) and parsed by `detect_nft_events`
(`crates/xdr-parser/src/nft.rs`). The `token_id` (vs a fungible `amount`) rides in
one of three on-chain `data` encodings — all observed on mainnet — and each is handled:

- **Shape A — scalar:** addresses in topics, bare scalar `token_id` in `data`
  (`u32`/`u64`/`i128`); the SEP-41/SEP-50 `single-value` form.
- **Shape B — packed vec:** topics carry only the event Symbol, `data = Vec[addr…,
token_id]` (older ERC-721-port / pre-`#[contractevent]` contracts).
- **Shape C — map:** addresses in topics, `data = map{ "token_id": uN }` — the
  soroban-sdk `#[contractevent]` map-by-field default, i.e. the OpenZeppelin
  reference-impl / SEP-50 shape (the dominant modern NFT encoding).

`consecutive_mint` (OpenZeppelin Consecutive / EIP-2309) is a batch event: topics
`[consecutive_mint, to]`, `data` a `[from_token_id, to_token_id]` range (map or vec),
expanded into one mint per id and bounded by `MAX_CONSECUTIVE_RANGE` (over-cap or
inverted ranges are dropped + tripwired).

**Fungible disambiguation.** The same symbols + a `map` data shape are also used by
SEP-41/SAC fungible events (`map{ amount, to_muxed_id }`, the CAP-67 muxed form). A map
is treated as NFT **iff it carries a `token_id` key**; a map with `amount`/`to_muxed_id`
and no `token_id` is fungible and skipped.

**Silent-drop tripwire.** A recognised NFT symbol whose argument shape does not parse is
dropped with a `tracing::warn!` tripwire (not silently), so unhandled future shapes
surface instead of vanishing. The parser is deliberately permissive here; the
authoritative NFT-vs-fungible-vs-other decision is the downstream WASM-spec classifier
(`soroban_contracts.contract_type`): only `Nft`-classified contracts' rows reach the hot
`nfts`/`nft_ownership` tables, `Fungible`/`Token` are dropped, and `Other`/`NULL` wait in
the `nfts_pending` quarantine until a later WASM observation reclassifies them. A
parse-time false-positive (a non-NFT emitting a `token_id`-keyed map) is therefore
contained in quarantine and never reaches the hot tables. (See lore task 0296 for the
prod/RPC evidence behind these shapes.)

### 5.6 Fungible Token Event Decode (`parse_token_event`)

Fungible SEP-41 / CAP-67 token movements — `transfer` / `mint` / `burn` /
`clawback` — are decoded by `parse_token_event` (`crates/xdr-parser/src/event_filters.rs`),
the fungible counterpart to `detect_nft_events`. Where the NFT path keys on a
`token_id`, this path reads the account operands and the asset identity (lore
task [0383](../../../lore/1-tasks/active/0383_FEATURE_l2-soroban-event-token-flow-decode/README.md)):

- **operands** — `transfer [sym, from, to, …]`, `mint [sym, to, …]`,
  `burn`/`clawback [sym, from, …]`. Missing / non-address operands ⇒ not a token
  event.
- **asset** — CAP-67 "unified" SAC events carry the classic asset as a **trailing
  SEP-11 string topic**: `"native"` → the native XLM asset, `"CODE:ISSUER"` → the
  classic credit. A bespoke (non-SAC) token omits it, so its asset identity is the
  emitting contract (`EventAsset::Contract`).
- **amount** — not decoded here. The presence indexes never store it, and the
  tx-detail page decodes amounts from archive XDR at read time (E3, ADR 0029), so
  the flow parser only needs operands + asset (see indexing-pipeline overview §5.3).

The decode lives in `db_clickhouse::persist::stage::derive_token_event`. The
`soroban-token-flow-backfill` one-shot pass called that same fn so both emitted
byte-identical surrogate rows; the pass was removed in lore 0425 once its history
was closed, leaving live ingest as the only caller. NFT-shaped events (no SEP-11 asset string) still
register their account operands as participants but are excluded from the fungible
asset index — that identity is ambiguous and tracked separately by the NFT path
above.

### 5.7 AMM Pool Registrations and State (`pool_router` / `pool_state`)

Router-family AMM pools (Aquarius's shape; ADR 0058, lore task 0374) are
decoded by two sibling modules:

- **`pool_router.rs`** — `parse_add_pool` decodes a registration event
  (pool, verbatim `pool_type` sym, 2–4 token legs, `subpool_salt`, raw
  `init_args` — three arg vocabularies exist and are stored verbatim);
  `detect_pool_registrations` sweeps a ledger's events SHAPE-first, from any
  deployment (an address list loses ~6% of live pools, measured), skipping the
  diagnostic container — which carries copies of events from FAILED
  transactions, and would otherwise register pools whose registration never
  applied. Shape alone does not make a registration trustworthy: the pool
  named in the payload is corroborated against its own instance state at
  staging (see the indexing-pipeline overview). The deposit⇄mint share-token
  rule is NOT part of this module — it is a demoted cross-check living with
  its corpus oracle in `tests/`, because instance state is the primary source.
  Verified against the full mainnet population: 497/497 registrations decode,
  0 false positives on a 307-event all-signatures negative corpus.
- **`pool_state.rs`** — reserves from ledger-entry changes, two layouts:
  `parse_plane_pool_data` reads the deployment's shared plane contract's
  `PoolData[pool]` entries (fungible pools; reserves vector VERBATIM);
  `parse_pool_instance` reads a pool instance's `TokenShare` / `Plane` /
  `Router` keys plus `Reserve0`/`Reserve1` — `Plane` is the key that makes it a
  pool, while `Router` is absent on an older contract version (five of the ten
  live deployments, measured on chain) and is therefore optional (concentrated pools keep reserves
  on their own instance — the plane holds their `PoolData` only at
  registration). Extraction mirrors the token-balance extractors: state
  images from created/updated/restored changes only. Verified by a
  bidirectional anti-test against the routers' own `update_reserves` events
  (0 missing, 0 foreign captures, last-write-per-ledger values 17/17).

Note the asymmetry between the two, which the storage contract depends on:
`parse_pool_instance` keys on the entry's OWNER (the pool contract itself), so
a pool can only ever describe itself, while `parse_plane_pool_data` takes the
pool identity from the entry's KEY PAYLOAD and uses the owner only as the
`plane` attribution. That is why the plane's claim is authoritative for
reserves only once a read pairs it with the plane the pool itself declares.

Both feed `persist::stage`, which writes the `liquidity_pools` registry rows
(`pool_kind = 1`), `pool_state_changes` and `pool_instance_state` (see the
database-schema overview and ADR 0058).

**`pool_pair_factory.rs` (task 0518)** is the second adapter, proving the ADR's
adapter-not-redesign consequence: same three tables, no shared shape change.
Differences worth knowing: discovery is the factory's `new_pair` event
(String label + Symbol name — the 0517 label convention; the vendor's
`new_pairs_length` counter is gapless per factory and doubles as the
backfill closure check); the pair's instance keys are BARE u32 enum
DISCRIMINANTS (0/1 = leg tokens, 2/3 = reserves, 4 = the deploying factory
— the corroboration authority), deliberately a separate reader from the
symbol-keyed Aquarius one, with the composite shape's false-positive rate
measured at zero over the raw corpus; and the SEP-41 half MIXES key
spellings in one instance (`METADATA` bare sym, `TotalSupply` VEC-WRAPPED —
the token-SDK enum encoding; a CLI dump flattens the wrap, which is exactly
how a wrong fixture passed unit tests and was caught by the local e2e).
The pair is its own LP token, so owner, stamp and declaration coincide —
`plane_id = share_token_id =` the pair itself.

**`pool_config_factory.rs` (task 0518)** is the third adapter (the
Phoenix-family shape) and the first whose state is NOT one atomic entry:
the pool keeps per-key PERSISTENT entries — a `CONFIG` symbol-keyed map
(legs, separate share token, per-pool `total_fee_bps`, `pool_type`
discriminant) written at creation/config-change only, plus bare-u32
`DataKey` discriminants (0 = TotalShares, 1/2 = the reserves) rewritten per
operation; the contract instance itself is storage-less. Discovery is the
factory's `("create", "liquidity_pool")` event carrying ONLY the pool
address, and the pool records no factory back-pointer — corroboration is
therefore the created gate + the pool's own full CONFIG in the registering
ledger (validated on the entire 14-registration population). Per-operation
recognition rests on the RESERVE PAIR co-occurring in one transaction
(measured across three eras; both-or-neither, half a pair refuses loudly).
All three adapters feed the same `PoolFamilyWrite` seam (`pool_family.rs`,
decision 4a): one enum from extraction to staging, a new family being a
variant + an arm rather than a field through every pipeline struct.

## 6. Storage Contract

### 6.1 Typed Columns and Appearance Indexes, No Raw XDR

Per [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md),
the DB holds only what list endpoints and partition-pruned reads need.

Typed summary columns / structured artifacts retained for normal explorer reads:

- `transactions` — `hash BYTEA`, `source_id BIGINT`, `fee_charged`, `successful`,
  `application_order`, `operation_count`, `has_soroban`, `inner_tx_hash`,
  `parse_error`, `created_at`
- `operations_appearances` — `type SMALLINT`, surrogate FKs, `BYTEA pool_id`, `amount BIGINT` count,
  typed `asset_code`/`asset_issuer_id`
- `soroban_events_appearances` / `soroban_invocations_appearances` — appearance
  indexes only (per §4.4 / §4.5)
- `soroban_contracts`, `wasm_interface_metadata` — with surrogate PK, BYTEA
  wasm_hash, SMALLINT contract_type, JSONB metadata
- derived explorer entities: `accounts`, `assets`, `nfts`, `nft_ownership`,
  `liquidity_pools`, `liquidity_pool_snapshots`, `lp_positions`,
  `account_balances_current` (the previously-planned
  `account_balance_history` was dropped per
  [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md))

Raw artifacts **not** retained in the DB (fetched at request time from the
public archive):

- envelope / result / result-meta XDR
- decoded event payload (type, topics, data)
- per-node invocation detail (function name, args, return value, depth)
- full invocation tree

### 6.2 Two Phases of Materialization

Ingestion owns writing typed summary + appearance-index rows into ClickHouse.
That is the only phase that runs unconditionally per ledger close.

The backend read path owns re-materializing heavy fields on demand for E3 / E14
via `xdr_parser::extract_*`; this phase runs only when a request asks for it
and is cacheable at the API Gateway / CloudFront layer.

### 6.3 Advanced View Contract

The advanced transaction experience is served by the read path, not by stored
raw payloads:

- E3 `/transactions/:hash` fetches the relevant `.xdr.zst` from the public
  archive, decompresses and parses it, and returns `envelope_xdr`, `result_xdr`,
  `result_meta_xdr`, `operation_tree`, and decoded events in the response
- response fields preserve their historical names so the public API surface is
  unchanged
- if the Rust parser is updated to expose a new field, no re-ingest is needed —
  the archive is the canonical source; the next request for a given transaction
  just picks up the new field

This contract should remain stable unless the main design document is updated
first.

## 7. Error Handling and Compatibility

### 7.1 Malformed XDR

If `stellar-xdr` returns an error during ingestion:

- the Ledger Processor logs the error with the transaction hash
- the typed summary columns that were successfully extracted are still written
- `transactions.parse_error = true` is set on the affected row
- the transaction remains visible with all non-failed fields available

At read time, the transaction detail handler reads `transactions.parse_error`
from the DB and **short-circuits** the archive fetch + re-parse for any row
where the flag is `true` (task 0190 — `crates/api/src/transactions/handlers.rs`).
The response carries `heavy: null` + `heavy_fields_status: "unavailable"` so
the lore-0044 / lore-0046 contract holds: the light slice is always served,
the heavy block is explicitly unavailable. Skipping the archive call on
known-degraded rows also avoids a wasted S3 round-trip per request. List
endpoints are unaffected because they do not call the archive.

The three reachable triggers for `parse_error = true` — `envelope.is_none()`,
`envelope_xdr.is_empty()`, `result_xdr.is_empty()` — are exercised by
synthetic-fixture unit tests in
`crates/xdr-parser/src/transaction.rs::parse_error_tests` (Variant A,
Variant B, plus a default-limits regression sentinel). End-to-end
persist coverage lives in `crates/indexer/tests/persist_integration.rs`
(`parse_error_transaction_persists_and_replays_idempotent`); the API
overlay contract is locked in
`crates/api/src/tests_integration.rs::detail_parse_error_tx_returns_unavailable_heavy_without_s3_contact`.

### 7.2 Unknown Operation Types

New protocol versions may introduce operation types not yet supported by the
pinned `stellar-xdr` crate.

In that case, the documented behavior is:

- render the operation as unknown in explorer responses
- surface the raw XDR (fetched from the archive) in the advanced view
- raise operational visibility through logging / alarming so the `stellar-xdr`
  bump can be scheduled

### 7.3 Protocol Upgrades

When Stellar introduces protocol changes that affect `LedgerCloseMeta` structure, the
system updates the Rust `stellar-xdr` crate in the Ledger Processor (per ADR 0004).

The parsing design assumes protocol upgrades are:

- infrequent
- announced in advance
- handled by updating the decode layer rather than redesigning the explorer architecture

## 8. Boundaries and Delivery Notes

### 8.1 Boundary with Other Parts of the System

Responsibility is split along the two-path parsing model:

- **ingestion** (Rust Ledger Processor) owns decode-at-ingest → typed summary
  columns + appearance indexes written to ClickHouse (single parser crate,
  shared with the API — `crates/xdr-parser`)
- **the database schema** owns persistence of typed summaries + appearance
  indexes; it does not hold raw XDR (ADR 0029)
- **the backend** (axum) owns request-time re-decode for E3 / E14 via
  `xdr_parser::extract_*` against `.xdr.zst` fetched from the public Stellar
  ledger archive; list endpoints run no parser
- **the frontend** consumes the API response and does not own XDR parsing in
  normal paths

### 8.2 Current Workspace State

The parsing implementation lives in `crates/xdr-parser/` and is invoked from
both the ingest Lambda (`crates/indexer`) and the backend API (`crates/api`).
Per [ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md)
this document is kept in sync with the code by requiring ADRs that touch the
parsing path to update it in the same PR.

[`technical-design-general-overview.md`](../technical-design-general-overview.md)
remains the primary cross-component source of truth; this file is the detailed
XDR-parsing reference.
