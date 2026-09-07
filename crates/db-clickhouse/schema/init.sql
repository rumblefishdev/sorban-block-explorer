-- ClickHouse production schema (task 0206 + 0208 + ADR 0044 amendments).
--
-- ## Design — hybrid: surrogate Int64 for high-cardinality join hubs,
-- natural keys everywhere else
--
-- Three tables get surrogate `id Int64` columns (deterministic
-- `cityhash64(natural_key)` derivation in
-- `crates/db-clickhouse/src/persist/ids.rs`):
--
--   - `accounts.id`            ← cityhash64(account_id StrKey)
--   - `soroban_contracts.id`   ← cityhash64(contract_id StrKey)
--   - `transactions.id`        ← cityhash64(hash bytes)
--
-- These three are the **central FK hubs** — referenced by 6–8
-- downstream tables each. Tens of millions of unique values at full
-- mainnet scale. Empirical measurement (10k-ledger smoke):
-- plain-String / LowCardinality FK columns added ~500 MB on disk vs
-- a hash-i64 baseline, projected ~550 GB on the full 11M-ledger
-- backfill. Int64 FK columns ~4× smaller pre-compression and
-- significantly cheaper on JOIN / GROUP BY because CH compares
-- integers in a single CPU op vs variable-length string memcmp.
--
-- Other tables (`assets`, `nfts`, `liquidity_pools`,
-- `liquidity_pool_snapshots`, `operations_appearances`,
-- `transaction_participants`, `nft_ownership`, `lp_positions`,
-- `account_balances_current`, `wasm_interface_metadata`,
-- `ledgers`, `transaction_hash_index`) keep their natural / composite
-- primary keys — no surrogate `id`. Composite (StrKey-or-hash, …)
-- ORDER BYs work cheaply for these without a hash layer.
--
-- ## Why deterministic `cityhash64(natural_key)` for the surrogate IDs
--
-- - **Replay-idempotency** — `ReplacingMergeTree` dedups by
--   `ORDER BY`; replays must produce the same `id` for the same
--   natural key so the merger collapses duplicates from a re-run.
-- - **Parallel-writer safety** — K runners on disjoint partition
--   ranges all compute the same `id` for the same StrKey without
--   coordination.
-- - **Cross-table FK consistency** — every `source_id` /
--   `account_id` / `deployer_id` / etc. across the schema is
--   `cityhash64(strkey)` of the referenced account; every
--   `contract_id` FK column is `cityhash64(strkey)` of the
--   referenced contract; every `transaction_id` FK column is
--   `cityhash64(tx_hash_bytes)`.
--
-- Hash algorithm: `cityhash-rs::cityhash_102_128` (CityHash v1.0.2
-- 128-bit) lower 64 bits. **Not bit-equivalent to CH SQL
-- `cityHash64()`** (CH builtin is the 64-bit variant of CityHash
-- 1.0.2 — a different algorithm from the lower-half of the
-- 128-bit). Future CH-side queries that want to recompute an `id`
-- must call the writer's helper or add a UDF.
--
-- ## Other production design choices
--
-- - **`ReplacingMergeTree` for every state-shaped table.** Including
--   `liquidity_pools` (was `MergeTree` in pilot, ~20× duplication
--   per task 0208 observation; now folded inline). Background merger
--   collapses by `ORDER BY` key.
-- - **`LowCardinality(String)`** on bounded-cardinality columns:
--   asset codes, event signatures, home_domain.
-- - **`ZSTD(3)` codecs** on JSON-ish columns: `soroban_events.topics_xdr`,
--   `soroban_events.data_xdr`, `wasm_interface_metadata.metadata`.
-- - **Empty-string sentinel** for composite-PK "no value" slots
--   (`assets.asset_code = ''` for native, etc.). CH `ORDER BY` on
--   plain `String` is significantly faster than `Nullable(String)`.
-- - **`account_balances_current` trustline removals → `balance = 0`
--   rows**; reads filter `WHERE balance > 0`. No tombstone engine.
--
-- ## Engine assignment
--
--   - append-only fact tables  → ReplacingMergeTree (dedup by ORDER BY)
--   - state tables             → ReplacingMergeTree(version_column) where
--                                a natural NOT NULL ledger column exists
--                                (otherwise plain ReplacingMergeTree)
--   - immutable lookup tables  → ReplacingMergeTree (collapse re-run /
--                                parallel-merge duplicates; lore-0293)
--
-- Partitioning: every fact table uses `intDiv(ledger_sequence, 500000)`
-- (~29 days at 5 s/ledger). State and immutable tables not partitioned.
--
-- All statements are idempotent (`CREATE TABLE IF NOT EXISTS`,
-- `CREATE DICTIONARY IF NOT EXISTS`); applying twice is a no-op.

----------------------------------------------------------------------
-- Immutable lookups (ReplacingMergeTree — collapse re-run / parallel-merge
-- duplicates; were plain MergeTree until lore-0293)
----------------------------------------------------------------------

-- ReplacingMergeTree (was plain MergeTree): the commit marker. A normal
-- single-machine crash-resume never re-writes a marked ledger (resume keys on
-- the ABSENT marker), but parallel-backfill merges / range overlap / manual
-- re-index DO produce duplicate `sequence` rows that plain MergeTree never
-- collapses — they double `ledgers` JOINs and needed a manual
-- `OPTIMIZE … DEDUPLICATE BY sequence` (task 0228). RMT collapses them
-- automatically on merge. Content is immutable per `sequence`, so no version
-- column; cost is merge-time only and ~0 on a unique monotonic key; reads stay
-- FINAL-free. (lore-0293)
CREATE TABLE IF NOT EXISTS ledgers (
    sequence          Int64,
    hash              FixedString(32),
    closed_at         DateTime64(3, 'UTC'),
    protocol_version  Int32,
    transaction_count Int32,
    base_fee          Int64,
    -- `closed_at` is not the sort key (`sequence` is), so a time-bounded read —
    -- the LP chart resolving a window's ledger range — scanned the whole ~26M-row
    -- table. `closed_at` is monotonic with `sequence`, so a minmax skip index
    -- prunes to the matching granule range for a few KB (minmax stores 2 values
    -- per granule group). Measured on the 2026-07-17 50M req/month load test:
    -- lpchart 77.9M -> 26.3M read_rows/request (-27.5bn over a 12-min run), CH
    -- time 411 -> 106 ms. Applied to the live prod table via
    -- ALTER ... ADD INDEX + MATERIALIZE INDEX (online, 2026-07-17); GRANULARITY 4
    -- matches what is deployed — keep them in sync.
    INDEX closed_at_mm closed_at TYPE minmax GRANULARITY 4
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(sequence, 500000)
ORDER BY (sequence);

-- ReplacingMergeTree (was plain MergeTree): written in the entity phase
-- (before the `ledgers` commit marker), so a crash-resume / backfill re-run
-- re-emits the same `(wasm_hash, metadata)` row. Plain MergeTree never dedups
-- → permanent byte-identical duplicates that double `contracts/interface`
-- JOINs and needed a manual `OPTIMIZE … DEDUPLICATE BY wasm_hash` (task 0228).
-- Content is immutable per `wasm_hash`, so no version column — any duplicate is
-- byte-identical and RMT collapses it on merge; reads stay FINAL-free. (lore-0293)
CREATE TABLE IF NOT EXISTS wasm_interface_metadata (
    wasm_hash FixedString(32),
    metadata  String CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
ORDER BY (wasm_hash);

----------------------------------------------------------------------
-- State tables — surrogate-id hubs (accounts, soroban_contracts)
----------------------------------------------------------------------

-- accounts: surrogate `id Int64` for cheap FK joins from 8+ tables.
-- ORDER BY natural key (`account_id`) — direct `WHERE account_id = 'GDMOSA…'`
-- granule-prunes cheaply, FK joins use `id` via hash-join (sort order
-- doesn't matter for hash join). Best of both worlds.
CREATE TABLE IF NOT EXISTS accounts (
    id                Int64,
    account_id        String,
    first_seen_ledger Int64,
    last_seen_ledger  Int64,
    sequence_number   Int64,
    -- home_domain: MUTABLE (SET_OPTIONS rewrites it), but rare — 4 of the
    -- 1.01M prod accounts that carry one have more than one value (measured
    -- 2026-07-22, task 0397). Any read projecting it MUST take the latest
    -- version (`ORDER BY last_seen_ledger DESC LIMIT 1`); the bare
    -- `LIMIT 1 BY id` used where only `account_id` is projected is NOT safe
    -- here. Very low cardinality globally (mainnet has a handful of unique
    -- SEP-1 issuers' domains across tens of millions of accounts; the vast
    -- majority are NULL). LowCardinality dictionary-encodes the few unique
    -- values per block — strong compression on top of default LZ4.
    home_domain       LowCardinality(Nullable(String)),
    -- The table is ORDER BY account_id (StrKey -> id resolves on the PK), but
    -- tx-list endpoints need the REVERSE: id (surrogate) -> account_id, to
    -- project `source_account`. `id` is not the sort key, so that lookup
    -- full-scans accounts (~23M rows, task 0290 — the ~35M/poll the polled
    -- /transactions list read came from this join, NOT the partition scan).
    -- A bloom_filter on `id` lets `WHERE id IN (page source_ids)` prune to a
    -- handful of granules. FP 0.001 (tighter than the 0.025 default) because
    -- the lookup tests N keys at once and per-key false positives compound
    -- (1-(1-p)^N): at default 0.025 / 11 keys ~6M rows survived; at 0.001 ~1M.
    -- Index is ~tens of MB. Applied to the live prod table via
    -- ALTER ... ADD INDEX + MATERIALIZE INDEX (online, 2026-06-16).
    -- CONSUMERS (check before ever dropping this as "unused"): the tx-list /
    -- search / assets id->StrKey seeks in `crates/api`, AND the SEP-1
    -- enrichment worker's issuer resolve (`enrich_and_persist::sep1_assets`,
    -- task 0397) — that one is a single-key seek and reads 3 granules with the
    -- index vs the whole table without it, silently, with no error to notice.
    INDEX idx_acc_id id TYPE bloom_filter(0.001) GRANULARITY 1
)
ENGINE = ReplacingMergeTree(last_seen_ledger)
ORDER BY (account_id);

-- accounts_recent (task 0385): a `last_seen_ledger`-ordered copy of `accounts`
-- powering the acclist browse (`GET /v1/accounts`, sole sort = last_seen). The
-- base table is ORDER BY account_id (identity — REQUIRED for RMT dedup, since
-- last_seen_ledger MUTATES and cannot live in the sort key), so the list's
-- last_seen sort is a whole-dimension `accounts FINAL` scan+sort (~24M rows). A
-- projection on the RMT is refused by CH 26.3 (task 0353, Code 344), so the
-- alternate ordering lives in this separate plain-MergeTree table, filled by a
-- refreshable MV (full recompute + atomic EXCHANGE → reads need no FINAL; mirrors
-- `balance_aggregates_mv`). `accounts::fetch_list` then read-in-order SEEKs it.
CREATE TABLE IF NOT EXISTS accounts_recent (
    id                Int64,
    account_id        String,
    last_seen_ledger  Int64,
    first_seen_ledger Int64,
    home_domain       LowCardinality(Nullable(String))
)
ENGINE = MergeTree
ORDER BY (last_seen_ledger, id);

-- Refreshable MV that recomputes `accounts_recent` from `accounts` (defined above
-- — the source table MUST exist before this CREATE). Full recompute + atomic
-- EXCHANGE, so reads need no FINAL. Refresh interval = acclist freshness: a
-- ≤interval-stale "recently active accounts" browse is fine, and it is a shared
-- server-side origin (strictly better than the old per-client FE 60s cache).
CREATE MATERIALIZED VIEW IF NOT EXISTS accounts_recent_mv
REFRESH EVERY 2 MINUTE
TO accounts_recent AS
SELECT id, account_id, last_seen_ledger, first_seen_ledger, home_domain
FROM accounts FINAL;

-- soroban_contracts: same hybrid pattern as accounts.
-- `wasm_uploaded_at_ledger` is the version slot; `DEFAULT 0` is the
-- stub-row sentinel (Pass 2 stub-rowing for referenced-but-not-deployed
-- contracts in mid-stream backfill ranges).
-- NAMING TRAP (task 0398) — `contract_id` means two different things:
--   * HERE (and in `soroban_contract_metadata`) it is a `String`: the real
--     `C…` StrKey.
--   * EVERYWHERE ELSE (`assets`, `nfts`, `nft_ownership`, `soroban_events`,
--     `operations_appearances`, …) it is an `Int64`: the cityhash64 surrogate
--     OF that StrKey, i.e. the value stored in `soroban_contracts.id`.
-- So a foreign key named `contract_id` joins `soroban_contracts.id`, NEVER
-- `soroban_contracts.contract_id`. Same value, three column names, two types
-- (`sac_contract_id`, `caller_contract_id` are the same surrogate too).
-- Deliberate one shared surrogate space (`ids::{account,contract,address}_id`
-- are byte-identical) — not redundancy. Renaming was costed and deferred to
-- task 0418: the ALTER is metadata-only, but the call sites are 85 in
-- `stage.rs` + 21 in `crates/api`.
CREATE TABLE IF NOT EXISTS soroban_contracts (
    id                       Int64,
    contract_id              String,
    wasm_hash                Nullable(FixedString(32)),
    wasm_uploaded_at_ledger  Int64 DEFAULT 0,
    deployer_id              Nullable(Int64),
    deployed_at_ledger       Nullable(Int64),
    contract_type            Nullable(Int16),
    is_sac                   Bool,
    -- `name` DROPPED (task 0304): dead since 0297 (no writer, reader-less,
    -- 0/148663 populated in prod). Prod `ALTER … DROP COLUMN name` pending.
    -- 0344: tx-detail resolves surrogate `id` -> `contract_id`, but `id` is not
    -- the sort key; mirror accounts' `idx_acc_id` so `WHERE id IN (…)` is a
    -- granule seek instead of a full-table `JOIN soroban_contracts FINAL`.
    INDEX idx_sc_id id TYPE bloom_filter(0.001) GRANULARITY 1
)
ENGINE = ReplacingMergeTree(wasm_uploaded_at_ledger)
ORDER BY (contract_id);

-- On-chain Soroban token metadata (name/symbol/decimals) read from the
-- contract's instance-storage `Symbol("METADATA")` struct. Per-contract,
-- INDEXER-derived (NOT the off-chain enrichment family). A SEPARATE table, not
-- columns on `soroban_contracts`, because: (1) RMT whole-row replace +
-- soroban_contracts' many writers (deploy / contract_type_rebuild EXCHANGE /
-- stub INSERTs / db-merge) would clobber in-row metadata to NULL (the G5 bug
-- class); (2) deploy identity (wasm_hash/deployer, from the deploy tx, NOT in
-- the instance entry) and metadata live on DIFFERENT update clocks, which one
-- RMT version column cannot track. Written by the parser on contract-instance
-- `created` / `updated` / `restored` changes; SACs skipped (name=CODE:ISSUER /
-- symbol=code / decimals=7 already derivable from SAC identity). `version` =
-- observed ledger (deterministic/replay-safe; latest wins). `decimals` is
-- rendered as 7 at read for classic/SAC.
-- INVARIANT: every row is a WHOLE-struct snapshot at one ledger (name+symbol+
-- decimals all set from the same METADATA at that version) — never a partial
-- single-column write. Read with `FINAL` (latest whole row per contract_id) —
-- the direct, frankenstein-proof RMT collapse for a whole-row read; the table is
-- bounded (Soroban-native tokens only) so the read-time merge is cheap. Read
-- (assets list): `assets … LEFT JOIN (SELECT contract_id, name, symbol, decimals
-- FROM soroban_contract_metadata FINAL) m ON m.contract_id = sc.contract_id`,
-- `COALESCE(ae.name, m.name, sc.name, …)`. The contract detail endpoint does NOT
-- read this table. Full reasoning: task 0297.
CREATE TABLE IF NOT EXISTS soroban_contract_metadata (
    contract_id  String,
    name         Nullable(String),
    symbol       Nullable(String),
    decimals     Nullable(UInt32),
    version      Int64
)
ENGINE = ReplacingMergeTree(version)
ORDER BY (contract_id);

----------------------------------------------------------------------
-- State tables — composite natural keys (no surrogate id)
----------------------------------------------------------------------

-- assets identity is a 4-tuple. Native XLM: all-empty + asset_type=0.
-- Classic credit: code+issuer set, contract_id=0. SAC: contract_id
-- set, code/issuer optional. Soroban-native: contract_id set,
-- code/issuer=empty/0.
--
-- DROPPED columns — noted so nobody re-adds them:
--  * `total_supply` / `holder_count` (dead per lore-0293, removed in 0310) — the
--    API serves the aggregate from `balance_aggregates`. A global rollup written
--    into this per-ledger-rewritten row clobbered them (no-version RMT,
--    last-write-wins → ~25% of classic served NULL in prod).
--  * `name` (0304) / `icon_url` (0310) — the indexer never set either (`icon_url`
--    verified 0/411654 rows populated in prod). Every read resolves the display
--    name/icon from `asset_enrichment` (curated) coalesced over
--    `soroban_contract_metadata` (on-chain) — never from `assets`.
CREATE TABLE IF NOT EXISTS assets (
    asset_type      Int16,
    asset_code      LowCardinality(String),
    issuer_id       Int64,            -- 0 for native / soroban-native
    contract_id     Int64,            -- 0 for native / classic-credit
    -- lore-0331 (Option C): single surrogate = ids::asset_id (cityhash64 of the
    -- canonical identity; classic="CODE:ISSUER"; SAC + soroban keyed by their own
    -- contract, so each is a DISTINCT asset id). The first single-column asset key — `balances.asset_id`
    -- references it. NOT in ORDER BY (natural key unchanged; additive, non-breaking).
    -- PROD: existing table needs `ALTER TABLE assets ADD COLUMN id Int64` + a
    -- one-time backfill of `id` for existing rows (maintenance window) — CREATE IF
    -- NOT EXISTS won't add it. Default 0 until a row is rewritten/backfilled.
    id              Int64 DEFAULT 0
)
ENGINE = ReplacingMergeTree
ORDER BY (asset_type, asset_code, issuer_id, contract_id);

-- Account-entry state side table (task 0463 / issue #377). Holds what ONLY an
-- observed `AccountEntry` can supply: the full signer set, the thresholds, and
-- the account's issuer flags. Named for the SOURCE rather than for `signers`
-- because the write CONDITION is what defines it — `accounts` beside it takes
-- whole-row writes from paths that carry no entry at all (participant
-- skeletons, RPC bootstrap), and RMT replaces the whole row, so anything only
-- an entry can know would be clobbered there. Task 0421 is expected to move
-- `sequence_number` and `home_domain` in here for exactly that reason; the
-- name is already correct for them.
--
-- `flags` (AccountFlags: AUTH_REQUIRED / AUTH_REVOCABLE / AUTH_IMMUTABLE /
-- AUTH_CLAWBACK_ENABLED) rides along because it shares all three properties —
-- same source, same write condition, same version. It is stored RAW, not
-- interpreted: the account page needs it to say whether the issuer of a held
-- asset can freeze or claw back that holding (USDC's issuer, for one, carries
-- AUTH_REVOCABLE).
--
-- ONE row per account carrying the FULL signer set as parallel arrays
-- (index i of signer_keys / signer_weights / signer_types is one signer, the
-- ClickHouse `Nested` idiom spelled out): the protocol caps
-- signers at 20 and rewrites AccountEntry wholesale, so RMT atomically
-- replaces the whole set — a removed signer cannot survive as a ghost, no
-- lifecycle column needed. A SIDE table, not columns on `accounts`, because
-- `accounts` takes whole-row writes from more than one path (participant
-- skeletons, RPC bootstrap) and a bolt-on column would be clobbered — the
-- proven failure mode of tasks 0492/0500. Written ONLY when an AccountEntry
-- was observed in the change set; trustline-only appearances never touch it.
-- Master weight is thresholds byte 0 and is NOT in the arrays (Horizon
-- synthesizes a master entry into its list; raw XDR does not — we store raw
-- truth, and any cross-check must diff against getLedgerEntries XDR, not
-- Horizon). signer_weights is UInt32 as in XDR; protocol constrains 0-255
-- and SetOptions deletes at 0, so out-of-range or zero weights are logged as
-- anomalies at persist, stored as carried.
-- PROD: this table shipped as `account_signers` and was renamed before the
-- writer was ever deployed — the rename was free then (table present but
-- EMPTY, no consumers, 0 of 76,334,267 `balances` rows carried a closure) and
-- would not have been afterwards. `CREATE TABLE IF NOT EXISTS` does not
-- rename an existing table, so a database created before 2026-08-21 needs:
--     RENAME TABLE account_signers TO account_entry_state;
-- Metadata-only, instant, nothing to move.
CREATE TABLE IF NOT EXISTS account_entry_state (
    account_id          Int64,
    signer_keys         Array(String),
    signer_weights      Array(UInt32),
    signer_types        Array(LowCardinality(String)),
    master_weight       UInt8,
    threshold_low       UInt8,
    threshold_med       UInt8,
    threshold_high      UInt8,
    flags               UInt32,
    last_updated_ledger Int64
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
ORDER BY (account_id);

-- SAC facet side table (ADR 0051 / task 0339). One logical row per SAC-having
-- classic_credit / native asset, keyed byte-for-byte like `assets` and joined at
-- read (`… GROUP BY key` with `max()`). Written by the INDEXER (not the enricher)
-- ONLY when a SAC is sighted — a deploy (`sac_deployed=1`) or an un-deployed
-- override event (`sac_deployed=0`) — NEVER on a plain trustline re-emit, so the
-- per-ledger whole-row `assets` rewrite can never zero it.
--   * `sac_contract_id` — cityhash64 surrogate of the SAC's `C…` StrKey (the same
--     hash used for `soroban_contracts.id`). The `C…` StrKey itself is NOT stored
--     — it re-derives on read from `code:issuer` (`derive_sac_strkey`).
--   * `sac_deployed` — deployed on-chain? MONOTONIC (false→true, never back), so
--     `SimpleAggregateFunction(max)` makes a deploy sighting stick over any later
--     un-deployed-override event; `max()` on the constant surrogate is a no-op.
-- AggregatingMergeTree (not RMT): `max` merges column-wise per key, so an override
-- event AFTER a deploy cannot downgrade `sac_deployed` (a versioned RMT keeps the
-- last-inserted whole row and WOULD downgrade). No `soroban_contracts` join needed
-- for deployed-ness — the flag is stored.
-- No skip-index on `sac_contract_id`: every read aggregates the whole (small)
-- table — `SELECT key, max(sac_contract_id) … GROUP BY key` for the join, then the
-- `/assets/{C…}` deep-link filters `sac.sac_contract_id = ?` on that join result —
-- so a per-column index would prune nothing. `asset_sac` is one row per
-- SAC-having asset (~31k at mainnet scale), so the full-table aggregate is cheap;
-- add a `sac_contract_id` skip-index + a direct point-lookup only if it ever grows.
CREATE TABLE IF NOT EXISTS asset_sac (
    asset_type      Int16,
    asset_code      LowCardinality(String),
    issuer_id       Int64,
    contract_id     Int64,
    sac_contract_id SimpleAggregateFunction(max, Int64),
    sac_deployed    SimpleAggregateFunction(max, UInt8)
)
ENGINE = AggregatingMergeTree
ORDER BY (asset_type, asset_code, issuer_id, contract_id);

-- Off-chain SEP-1 enrichment for `assets` (task 0231). Written by the
-- enrichment-worker Lambda (NOT the indexer), keyed byte-for-byte like
-- `assets`, joined at read (`… FINAL`/`argMax`). Lives in a separate table
-- because the live indexer re-writes whole `assets` rows (enrichment columns
-- NULL) and would clobber it; `ReplacingMergeTree(version)` is order-safe under
-- retries and lets the enricher CLEAR a value (re-insert NULL with a higher
-- `version`). `version` = enricher processing timestamp (ms; non-nullable as
-- RMT requires). NOTE: this table is now the ONLY icon/name source — the
-- vestigial `assets.{icon_url,name}` were dropped (0304 / 0310), so every read
-- path resolves them here (coalesced over `soroban_contract_metadata` for
-- on-chain values). Full reasoning + measured evidence: lore task 0231,
-- `notes/R-clickhouse-enrichment-write-strategy.md`.
CREATE TABLE IF NOT EXISTS asset_enrichment (
    asset_type   Int16,
    asset_code   LowCardinality(String),
    issuer_id    Int64,
    contract_id  Int64,
    icon_url     Nullable(String),
    name         Nullable(String),
    version      DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(version)
ORDER BY (asset_type, asset_code, issuer_id, contract_id);

-- NOTE: the legacy `asset_aggregates` table + `asset_aggregates_mv` (classic
-- supply/holders over `account_balances_current`, keyed code+issuer) were DROPPED
-- (task 0331) — superseded by `balance_aggregates` (below) over the unified
-- `balances` table, keyed by `assets.id`. It was a refreshable MV (derived), so
-- the drop loses no source data; classic supply now flows through `balances`
-- (single-write + the one-time classic→`balances` migration).
--
-- Pre-computed per-asset aggregates over the unified `balances` table (task 0331,
-- Option C) — supply + active-holder count keyed by the `assets.id` surrogate.
-- `total_supply` is RAW `Int128` (the read scales by the asset's `decimals`); a
-- Soroban token's supply needs raw Int128 (token-specific decimals; PIKA=43224
-- overflows any Decimal), so this table is raw for ALL asset types once classic
-- migrates in (step 6). Refreshable-MV (full recompute, atomic EXCHANGE → no FINAL
-- on read). Columns `Nullable` so a read LEFT-JOIN miss is NULL (→ "—"), not 0.
-- (classic enters `balances` via single-write + the one-time classic→`balances`
-- migration; type-3 + SAC/native contract-held via the live parser + seed.)
CREATE TABLE IF NOT EXISTS balance_aggregates (
    asset_id     Int64,
    total_supply Nullable(Int128),
    holder_count Nullable(Int32)
)
ENGINE = MergeTree
ORDER BY (asset_id);

-- NOTE: `balance_aggregates_mv` (the refreshable MV that fills this table) is
-- defined AFTER `balances` below — a `CREATE MATERIALIZED VIEW … FROM balances`
-- needs its source table to already exist on a fresh `init.sql` run.

-- (tombstone) `soroban_token_supply` was DROPPED — task 0331 Option-A decision.
-- A per-token authoritative `TotalSupply` key read (76.6% of type-3 tokens expose
-- one; 27% do not) added a second supply source + a seed-only staleness bug for
-- no measurable gain: a mint always credits a holder balance (often a contract
-- treasury, summed under Path A G+C holders), so `balance_aggregates.total_supply`
-- (Σ amount, MV-refreshed) equals the real supply. ONE universal method; the
-- narrow residue is the accepted non-100% cost.
--
-- What the residue actually is (measured on USDT0, 2026-09-03 — task 0210):
-- everything the chain holds WITHOUT a holder, because this sum is keyed by
-- holder. Native LP reserves (`LiquidityPoolEntry` — the depositor holds shares,
-- not the asset) and claimable balances (`ClaimableBalanceEntry` — off the
-- sender, not yet the receiver), plus the TTL-archived tail and true rebasing.
-- On USDT0 the first two are the ENTIRE gap: 24.5064194 + 0.3070000 of
-- 2,595,050.05, i.e. 0.001%. An AMM-heavy asset will show far worse. Soroban-DEX
-- pool reserves are NOT in this list — a Soroban pool holds its reserves as a
-- contract, so ADR 0051 already sums them.

CREATE TABLE IF NOT EXISTS account_balances_current (
    account_id          Int64,
    asset_type          Int16,
    asset_code          LowCardinality(String),
    issuer_id           Int64,        -- 0 for native
    balance             Decimal128(7),
    last_updated_ledger Int64
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
ORDER BY (account_id, asset_type, asset_code, issuer_id);

-- ── Option C unified balance model (task 0331) ──────────────────────────────
-- The two tables below are the unified replacement for `account_balances_current`
-- (classic) + the interim `soroban_token_balances` (type-3). The persist path now
-- writes `balances` ONLY (single-write, task 0331 Option A); `account_balances_current`
-- is retained (no longer written) pending the one-time classic→`balances` data
-- migration + drop (OPS steps 6b/6d). See the task README.

-- Unified per-holder balances — the single balance model for ALL asset types.
-- `amount` is RAW `Int128` (scale by the asset's `decimals` at read — universal
-- fixed-point, handles classic 7-dec AND arbitrary Soroban decimals). `holder_id`
-- = `cityhash64(holder StrKey)` (the same surrogate space as `accounts.id` /
-- `soroban_contracts.id`; resolve back to a StrKey via `accounts` (G) or
-- `soroban_contracts` (C) — there is no dedicated address dimension). `asset_id`
-- → the `assets.id` surrogate (`ids::asset_id`). RMT version = `last_updated_ledger`;
-- a removed/zeroed balance writes 0 so a fully-spent holder collapses.
-- `closed_at_ledger` (ADR 0055): 0 = the holding relationship is live, >0 = the
-- ledger in which the entry disappeared from the chain. Before it existed a
-- removal was written as `amount = 0`, byte-identical to a live-but-empty
-- holding, so the read path could not tell them apart and hid both. Ledger 0
-- does not exist (genesis is 1), so 0 is a safe live sentinel. `DEFAULT 0` is
-- load-bearing: the CH driver rejects inserts client-side when the table has a
-- column the writer's struct does not know AND the column has no default, so
-- the default is what lets the `ALTER` land before the writer deploys.
-- PROD: `CREATE TABLE IF NOT EXISTS` does NOT add a column to an existing
-- table, so an already-created database needs the ALTERs run by hand — the
-- same convention as `assets.id` above. Both are metadata-only (no data
-- rewrite) and were applied to production on 2026-08-18, verified with
-- `DESCRIBE TABLE balances` / `lp_positions`:
--     ALTER TABLE balances     ADD COLUMN IF NOT EXISTS closed_at_ledger Int64 DEFAULT 0;
--     ALTER TABLE lp_positions ADD COLUMN IF NOT EXISTS closed_at_ledger Int64 DEFAULT 0;
CREATE TABLE IF NOT EXISTS balances (
    holder_id           Int64,
    asset_id            Int64,
    amount              Int128,
    last_updated_ledger Int64,
    closed_at_ledger    Int64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
-- holder_id FIRST: the account-detail read is a per-holder PK-prefix seek (the
-- hot, latency-critical path — mirrors `account_balances_current`'s account_id-first
-- key, avoids the 0198 Seq Scan). `balance_aggregates_mv` GROUP BY asset_id is a
-- periodic full-recompute scan either way, so it doesn't need asset_id-first.
ORDER BY (holder_id, asset_id);

-- Refreshable MV that recomputes `balance_aggregates` from `balances` (defined
-- above — the source table MUST exist before this CREATE). Full recompute + atomic
-- EXCHANGE, so reads need no FINAL.
CREATE MATERIALIZED VIEW IF NOT EXISTS balance_aggregates_mv
REFRESH EVERY 2 MINUTE
TO balance_aggregates AS
SELECT
    asset_id,
    sum(amount)                  AS total_supply,
    toInt32(countIf(amount > 0)) AS holder_count
FROM balances FINAL
GROUP BY asset_id;

CREATE TABLE IF NOT EXISTS nfts (
    contract_id           Int64,
    token_id              String,
    collection_name       Nullable(String),
    name                  Nullable(String),
    media_url             Nullable(String),
    minted_at_ledger      Nullable(Int64),
    current_owner_id      Nullable(Int64),
    current_owner_ledger  Int64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(current_owner_ledger)
ORDER BY (contract_id, token_id);

-- Task 0217 — quarantine for NFT-candidate rows whose contract is still
-- `Other`/NULL-classified (WASM not observed in the current backfill
-- window). Same row shape as `nfts` so promotion is a column-projection
-- INSERT. API endpoints never read this table — production sees only
-- definitive `Nft`-classified rows in `nfts`. Promoted to `nfts` on
-- `Other → Nft` reclassification, dropped on `Other → Fungible`
-- reclassification. `Token` (SAC) is classified at deploy time and not
-- reachable via WASM reclassification — `Token`-classified contracts are
-- dropped at persist-filter time and never enter pending.
--
-- Note: CH-side writer-driven routing into `*_pending` is not implemented
-- in PR #180; tables ship as schema-only on CH. PG writer drives the
-- persist-time routing today. CH writer parity is a follow-up task.
CREATE TABLE IF NOT EXISTS nfts_pending (
    contract_id           Int64,
    token_id              String,
    collection_name       Nullable(String),
    name                  Nullable(String),
    media_url             Nullable(String),
    minted_at_ledger      Nullable(Int64),
    current_owner_id      Nullable(Int64),
    current_owner_ledger  Int64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(current_owner_ledger)
ORDER BY (contract_id, token_id);

-- Off-chain `token_uri` enrichment for `nfts` (task 0231). Written by the
-- enrichment-worker Lambda (NOT the indexer), keyed like `nfts`, joined at
-- read. Separate table for the same reason as `asset_enrichment`: the live
-- indexer re-writes whole `nfts` rows on every ownership change (metadata
-- NULL) and the ownership clock (`current_owner_ledger`) is its RMT version —
-- so enrichment in `nfts` would be clobbered AND has no safe version to claim.
-- Here `version` is the enricher's own clock (ms), independent of ownership.
-- `nfts.{name,media_url,collection_name}` stay vestigial (NULL; DROP deferred
-- to a cleanup task). See lore task 0231,
-- `notes/R-clickhouse-enrichment-write-strategy.md`.
CREATE TABLE IF NOT EXISTS nft_enrichment (
    contract_id      Int64,
    token_id         String,
    name             Nullable(String),
    media_url        Nullable(String),
    collection_name  Nullable(String),
    version          DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(version)
ORDER BY (contract_id, token_id);

-- liquidity_pools (task 0208 Path 2 folded inline): RMT(last_updated_ledger),
-- `created_at_ledger` dropped (derive read-time from
-- `MIN(ledger_sequence) FROM liquidity_pool_snapshots GROUP BY pool_id`).
-- One registry for EVERY pool (task 0374, decided 2026-08-27): classic rows
-- keep pool_kind=0 and their soroban columns at defaults; Soroban AMM pools
-- are rows with pool_kind=1. One user-facing concept, one table — the same
-- reasoning ADR 0056 applies to positions. Engine stays RMT: whole-row
-- replace is safe because each row has exactly ONE writer — the classic arm
-- reads authenticated `LiquidityPoolEntry` ledger entries and never touches a
-- contract pool_id, and the soroban arm accepts a registration only when the
-- named pool's OWN instance storage names the emitter as its `Router`
-- (`stage.rs`), so a third party cannot register — or overwrite — a pool it
-- does not own, EXCEPT pools whose instance declares no router (older
-- contract versions, 23 measured, all event-silent): those are accepted
-- UNVERIFIED with a warn, and their registry row is forgeable — see the
-- acceptance arm in `stage.rs` for the bounded residual.
--
-- Registration provenance (the router's subpool salt, raw init_args beyond
-- the fee) is NOT materialised: the add_pool event itself sits complete and
-- forever in soroban_events — extract on demand, never copy (depth-first,
-- 2026-08-28).
--
-- The pair-shaped asset_a_*/asset_b_* columns are LEGACY once `legs` is
-- backfilled for classic rows: 3- and 4-leg stable pools exist on mainnet and
-- do not fit a pair. They stay until the ~612 pair-shaped call sites migrate
-- to `legs` (tracked in 0374; do not add new readers).
CREATE TABLE IF NOT EXISTS liquidity_pools (
    pool_id              FixedString(32),        -- classic: SHA-256 of the asset pair (CAP-38); soroban: 32-byte payload of the C... contract address. pool_kind says which — without it a contract id renders as a well-formed WRONG L... strkey
    asset_a_type         Int16,                  -- LEGACY pair shape; XDR AssetType domain (NOT assets.asset_type's AssetFamily domain — task 0496)
    asset_a_code         LowCardinality(String), -- LEGACY pair shape
    asset_a_issuer_id    Int64,                  -- 0 for native; LEGACY pair shape
    asset_b_type         Int16,                  -- LEGACY pair shape
    asset_b_code         LowCardinality(String), -- LEGACY pair shape
    asset_b_issuer_id    Int64,                  -- LEGACY pair shape
    fee_bps              Int32,                  -- both worlds; soroban PER FAMILY: router = add_pool init_args[0] (u32, the one arg every measured shape shares); pair-factory = the vendor's compiled-in 30; config-factory = the pool's own CONFIG total_fee_bps (creation-time snapshot, mutable on chain)
    last_updated_ledger  Int64,
    pool_kind            UInt8                  DEFAULT 0,  -- 0=classic, 1=soroban contract
    legs                 Array(Int64)           DEFAULT [], -- PER-KIND id space (pool_kind says which): kind 1 = token-contract surrogates in emission order (= get_tokens(); == assets.id only for bespoke type-3 — SAC legs resolve via asset_sac); kind 0 = ASSET surrogates (pool_leg_asset_id, the lp_operation_amounts join key) — legs-migration step 2. 3- and 4-leg pools exist, so never a pair
    deployment_id        Int64                  DEFAULT 0,  -- soroban_contracts.id surrogate of the registering router; 0 = classic. Two live router deployments share Aquarius's code and only one is Aquarius (task 0374 T1) — labels resolve from this id at read time, so a new pool is labelled the moment it registers, with no editorial UPDATE to re-run
    pool_type_raw        LowCardinality(String) DEFAULT ''  -- verbatim PER FAMILY, un-normalised on purpose (folding vocabularies is read-time interpretation): router = add_pool sym (constant|stable|concentrated|...); pair-factory = '' (the vendor emits no type); config-factory = the PairType u32 discriminant as text ("0" = XYK)
    -- share_token_id was removed from the write path before any deploy: the relation lives ONLY in pool_instance_state (a registry column would clobber the full row on RMT merge, and a permanent 0 misleads). Prod (which received the column via the registry backfill ALTER) drops it with: ALTER TABLE liquidity_pools DROP COLUMN share_token_id
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
ORDER BY (pool_id);

-- Pool reserve state (task 0374 step 7) — THE reserve source (T4: event
-- arithmetic failed its oracle 6/49). ONE deterministic row per
-- (pool, plane, ledger), collapsed at parse time in ledger apply order — the
-- same grain and mechanism as the classic snapshots (decision 2026-08-30), so
-- the 0356 LIMIT-1/no-FINAL invariant holds and the future unification is
-- a plain union. Intra-ledger history lives in soroban_events forever.
-- Two on-chain layouts feed it: fungible pools write plane PoolData
-- (vector VERBATIM — per-tick tail possible; reads slice by leg count),
-- concentrated pools write Reserve0/1 on their own instance. Named without
-- a family prefix on purpose: classic history joins HERE if the snapshot
-- models unify — never the reverse (ADR 0058).
--
-- `plane_id` is IN the sort key (three-lens review, 2026-09-01): a plane
-- entry names its pool in an attacker-writable payload, so keying on
-- (pool, ledger) alone would let a forged write EVICT the pool's genuine
-- row on RMT merge — the read-side declared-plane filter would then hide
-- the forgery but serve a stale ledger's reserves as current. With the
-- plane in the key a forged row keeps its own key space, dies at the read
-- filter (which joins `pool_instance_state`'s declared plane), and stays
-- visible as evidence. Standing monitor for both forgery attempts and a
-- plane migration (which would orphan pre-migration history — the
-- share-token column of `pool_instance_state` measured 13 such
-- re-pointings, so "planes never migrate" must fail loudly):
--   SELECT pool_id, uniqExact(plane_id) AS planes
--   FROM pool_state_changes GROUP BY pool_id HAVING planes > 1
--
-- Version-less RMT, exactly like its classic twin `liquidity_pool_snapshots`:
-- the row is made a deterministic function of the ledger by folding at STAGE
-- time (`fold_pool_state_changes`), not by a version column. Two writers feed
-- this table — the plane arm and the concentrated-instance arm — and the fold
-- collapses their collision; per backfills.md rule 4 a re-parse then wins on
-- its own, because it lands last. A version column keyed on anything
-- batch-local would BREAK that: a narrow re-parse could stamp a lower version
-- than the original wide parse and lose to the stale row.
CREATE TABLE IF NOT EXISTS pool_state_changes (
    pool_id          FixedString(32),
    ledger_sequence  Int64,
    reserves         Array(Int128),
    plane_id         Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 5000000)
ORDER BY (pool_id, plane_id, ledger_sequence);

-- What a pool declares ABOUT ITSELF, from its own instance storage — the
-- pool contract is the ledger-authenticated owner of that entry, so these
-- values cannot be forged by a third party. A SIDE table (the asset_sac
-- pattern): a partial row in the RMT registry would clobber the full
-- registration on merge, and these facts arrive on a DIFFERENT clock than
-- the registration (instance rewrites, share-token migrations) which one
-- RMT version column on `liquidity_pools` could not track. Versioned by
-- sighting ledger so a migration (13 pools re-pointed their share token;
-- measured) converges on the newest — matching share_id() on chain.
--
-- `plane_id` is the AUTHORITY for reserve provenance (review #438). Reserve
-- rows in `pool_state_changes` carry the plane that wrote them, but a plane
-- entry names its pool in an attacker-writable KEY payload — any contract
-- can publish `[PoolData, Address(victim)]` under its own id. Reads must
-- therefore keep only reserve rows whose `plane_id` matches the plane the
-- POOL ITSELF declares here. `plane_id` is always populated — `Plane` is
-- the shape key that recognises an instance as a pool at all. `Router` is
-- OPTIONAL: five older deployments write none (23 pools), and their
-- registrations are accepted UNVERIFIED with a warn, and only when the
-- instance is CREATED in the registering ledger (see the acceptance arm
-- in `stage.rs`). `share_token_id = 0` is structural for concentrated
-- pools, which never mint one.
CREATE TABLE IF NOT EXISTS pool_instance_state (
    pool_id            FixedString(32),
    plane_id           Int64,
    share_token_id     Int64,
    total_shares       Int128,  -- raw u128 from instance TotalShares; 0 = key absent (structural: concentrated + elastic) — AND structurally 0 forever for the config-factory family (rows stage only on CONFIG writes; its live supply is the share token's own tracked supply, never this column). Read half of the soroban "Total shares" gap
    derived_at_ledger  Int64
)
ENGINE = ReplacingMergeTree(derived_at_ledger)
ORDER BY (pool_id);

-- `closed_at_ledger`: same lifecycle semantics as `balances` (ADR 0055) — a
-- withdrawn position was written as `shares = 0`, indistinguishable from a
-- position that still exists at zero.
CREATE TABLE IF NOT EXISTS lp_positions (
    pool_id              FixedString(32),
    account_id           Int64,
    shares               Decimal128(7),
    first_deposit_ledger Int64,
    last_updated_ledger  Int64,
    closed_at_ledger     Int64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(last_updated_ledger)
ORDER BY (pool_id, account_id);

----------------------------------------------------------------------
-- Append-only fact tables (ReplacingMergeTree, partitioned)
----------------------------------------------------------------------

-- transactions: surrogate `id Int64` for cheap FK joins from
-- operations_appearances, transaction_participants, soroban_events,
-- soroban_invocations_appearances, nft_ownership. ORDER BY
-- (ledger_sequence, application_order) for time-series scans.
CREATE TABLE IF NOT EXISTS transactions (
    id                Int64,
    hash              FixedString(32),
    ledger_sequence   Int64,
    application_order Int16,
    source_id         Int64,           -- FK to accounts.id
    fee_charged       Int64,
    inner_tx_hash     Nullable(FixedString(32)),
    successful        Bool,
    operation_count   Int16,
    has_soroban       Bool,
    parse_error       Bool,
    INDEX idx_tx_hash_bloom hash TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, application_order);

CREATE TABLE IF NOT EXISTS transaction_hash_index (
    hash            FixedString(32),
    ledger_sequence Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (hash);

-- `amount` is a **fold count** of identity-tuple duplicates (task 0163 /
-- ADR 0033 PG-side convention; CH inherits same semantic): the number of
-- on-chain operation envelope ops that collapsed into this single
-- appearance row by identity. NOT a stroop value or per-op amount. Real
-- per-op stroop values live in `result_meta_xdr` (Archive XDR overlay
-- on PG; read-time decode on CH via E03 statement C). API callers MUST
-- NOT interpret this column as a token amount.
CREATE TABLE IF NOT EXISTS operations_appearances (
    transaction_id    Int64,
    application_order Int16,
    type              Int16,
    source_id         Nullable(Int64),
    destination_id    Nullable(Int64),
    contract_id       Nullable(Int64),
    asset_code        LowCardinality(String),
    asset_issuer_id   Nullable(Int64),
    -- Crossed liquidity pools (task 0261/0268): single-element for LP
    -- deposit/withdraw, full crossed-pool list (result claim atoms) for
    -- path payments / offers, [] for no pool involvement (Array cannot be
    -- Nullable; has([], x) = 0 so empty arrays miss pool filters). Sorted +
    -- deduped by the stage fold. Filter with
    -- has(pool_ids, toFixedString(unhex(...), 32)).
    pool_ids          Array(FixedString(32)),
    amount            Int64,   -- fold count, see header comment
    ledger_sequence   Int64,
    -- Skip index for the `has(pool_ids, …)` pool filter (E20 /
    -- liquidity-pools/:id/transactions; task 0281 C). The read driver
    -- (fetch_pool_transactions) seeks via read-in-order `ORDER BY ledger DESC
    -- LIMIT`, so a POPULAR pool early-terminates near the tip; this bloom bounds
    -- the OTHER regime — a sparse pool whose last activity is far below the tip,
    -- where the driver must scan back to reach it. `bloom_filter(0.001)` (not the
    -- 0.025 default) keeps that scan's false-positive floor at ~0.1 % of the table
    -- (~6 M rows) instead of ~2.5 % (~155 M, box-measured 2026-06-17); same
    -- tight-FP rationale as the 0290 `idx_acc_id`.
    INDEX idx_oa_pool_ids pool_ids TYPE bloom_filter(0.001) GRANULARITY 1,
    -- Skip index for the contract-filtered transaction-list path (E03
    -- Statement B; task 0333). `contract_id` is NOT the ORDER BY prefix
    -- (unlike the `soroban_events` / `soroban_invocations_appearances` arms of
    -- the same UNION, which seek on `contract_id`), so this arm full-scanned.
    -- The read driver seeks via read-in-order `ORDER BY ledger DESC LIMIT`: a
    -- VERY active contract early-terminates near the tip (cheap), but a SPARSE
    -- contract — few/old appearances — forces a scan of the entire table to
    -- fill the page (box-measured: 42-appearance contract read 13.18 M / the
    -- whole table; this is the ~6.2 B-rows/query full scan that blew the prod
    -- `api_throttle.read_rows` quota on 2026-06-29, CH Code 201). This bloom
    -- bounds that sparse regime to the granules that actually hold the contract.
    -- `bloom_filter(0.001)` (not the 0.025 default) keeps the false-positive
    -- floor tight, same rationale as `idx_oa_pool_ids` / the 0290 `idx_acc_id`.
    -- contract_id is Nullable; `= <id>` never matches NULL rows, and granules
    -- holding only NULLs carry no value → skipped.
    INDEX idx_oa_contract_id contract_id TYPE bloom_filter(0.001) GRANULARITY 1
    -- idx_oa_asset_issuer_id (bloom on asset_issuer_id, was here for the E10
    -- asset-tx CLASSIC arm, task 0334) DROPPED 2026-07-13: task 0359 moved the
    -- asset-tx driver to the `operation_asset_appearances` fan-out seek, so no
    -- query filters `operations_appearances` by `asset_issuer_id` anymore — the
    -- bloom's sole consumer is gone (verified across api / audit-harness /
    -- backfill). Prod is an existing DB (this file is fresh-only): reclaim the
    -- ~97 MiB with `ALTER TABLE operations_appearances DROP INDEX idx_oa_asset_issuer_id`.
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, transaction_id, application_order);

CREATE TABLE IF NOT EXISTS transaction_participants (
    account_id      Int64,
    ledger_sequence Int64,
    transaction_id  Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (account_id, ledger_sequence, transaction_id);

-- operation_asset_appearances: per-(asset, transaction) presence index (task
-- 0359). The EXACT transaction_participants shape, with asset_id for account_id.
-- Fixes the single-asset-slot loss on operations_appearances -- offers store
-- ZERO assets there, path payments keep only destAsset, native XLM is an
-- empty-string sentinel. Here every asset DECLARED in any op body of a tx is one
-- row, keyed asset-first so a per-asset activity page is a PK-prefix seek.
-- asset_id = cityhash64 surrogate (ids::asset_id); native = hash64('native'), a
-- first-class key, never an empty sentinel. Pure presence: duplicate (asset, tx)
-- rows within a tx collapse in the RMT.
-- NOT FK-guaranteed against `assets`: a fact row can hold an asset_id with no
-- `assets` row (an asset touched only by failed txs, a rejected allow_trust with
-- source != issuer, or a trustline predating the ingest window). Harmless -- the
-- read path reaches a fact row only via an existing `assets` row, so such orphans
-- are unreachable dead weight, never wrong output.
-- `net_settled` (task 0393) REMOVED 2026-09-04: the per-(tx, asset) aggregate
-- was a lossy summary — it carried no direction and no account, so on an account
-- page an inbound and an outbound transfer rendered identically. Replaced by a
-- lossless per-transfer design (see the follow-up task); the reducer itself
-- (`persist::stage::ledger_deltas_net_settled` + `xdr_parser::ledger_balance_deltas`
-- + `xdr_parser::net_settled`) is KEPT — it reads the authoritative LEDGER balance
-- changes and is the input the replacement needs.
CREATE TABLE IF NOT EXISTS operation_asset_appearances (
    asset_id        Int64,
    ledger_sequence Int64,
    transaction_id  Int64
    -- idx_oaa_transaction_id (bloom on transaction_id, planned for the 0393
    -- "Net settled" per-tx read) REMOVED 2026-08-06: that read was withdrawn
    -- from the API before it ever shipped (see common/ch.rs; [[0411]] owns
    -- reinstating it), every live query on this table filters by asset_id
    -- (the leading key), and the bloom measured 19.87 GiB on prod (fpp 0.001
    -- over 11.25bn non-null near-unique values) for zero consumers. It was
    -- briefly added+materialized on prod the same day, then dropped after the
    -- consumer audit. 0411 decides between re-adding the bloom and the
    -- (ledger, tx)-leading companion table (which supersedes it); name the
    -- consumer here if it comes back.
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (asset_id, ledger_sequence, transaction_id);

-- operation_pools: per-(pool, transaction) presence index (task 0365). The
-- pool-dimension twin of operation_asset_appearances / transaction_participants,
-- keyed pool-first so GET /liquidity-pools/:id/transactions is a PK-prefix seek
-- instead of the density-dependent has(pool_ids, X) scan over
-- operations_appearances (0281-C read-in-order driver, superseded). pool_id = the
-- raw 32-byte pool hash (already how operations_appearances.pool_ids stores each
-- crossing -- no surrogate). Populated by arrayJoin(pool_ids) in staging; pure
-- presence, so duplicate (pool, tx) rows within a tx collapse in the RMT.
-- Plain Int64 columns, matching transaction_participants / operation_asset_appearances.
CREATE TABLE IF NOT EXISTS operation_pools (
    pool_id         FixedString(32),
    ledger_sequence Int64,
    transaction_id  Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence, transaction_id);

-- lp_operation_amounts: what each operation actually moved through a pool
-- (task 0279, issue #371) — the value twin of `operation_pools`, same
-- pool-leading key prefix. `operation_pools` stays the paging driver; this is
-- the value lookup for the page's (ledger, tx) set.
--
-- ROW GRAIN = (operation, pool, asset), with the op's claim atoms PRE-SUMMED
-- in Rust before the insert. NOT one row per atom: a single op can take the
-- same pool several times (CAP-38 interleaved order-book/AMM matching) and
-- every such atom carries the IDENTICAL ORDER BY tuple, so the RMT would keep
-- one and silently drop the rest of the fill. A per-op sum is deterministic on
-- replay, so live ingest and the historical re-parse emit byte-identical rows
-- for a key and the duplicate collapses cleanly (the single-writer argument:
-- one reducer, identical on the live and the re-parse path).
--
-- `amount` is SIGNED FROM THE POOL'S PERSPECTIVE: positive = the asset entered
-- the pool, negative = it left. The sign pattern therefore names the event
-- with no type column — trade `+/-`, deposit `+/+`, withdrawal `-/-` — and the
-- two rows of one (op, pool) are its two legs. RAW STROOPS in `Int64`, scaled
-- by 7 at read like every other amount here: classic AMM pools are 7-decimal
-- by definition, the XDR sources (`ClaimLiquidityAtom.amount_{sold,bought}`,
-- trustline balance deltas) ARE `int64`, and a per-op sum is bounded by the
-- pool's own `int64` reserve, so no overflow is reachable. Deliberately not
-- `Int128` (that width is for Soroban i128 token amounts, which a classic pool
-- cannot carry) and not `Decimal128(7)` (the read-model
-- choice in `liquidity_pool_snapshots` for the Lambda's USD math — fact tables
-- store raw ints, and the cross-check below is one cast away).
--
-- `asset_id` = the `ids::asset_id` surrogate shared with
-- `operation_asset_appearances` / `balances`; native XLM is the first-class
-- `NATIVE_ASSET_ID`, never an empty sentinel. A pool's rows are always its two
-- legs, so the read renders against the pool definition it already holds.
--
-- Two producers, one shape, both in `stage.rs`: trades from the claim atoms
-- `gross_volume_a_by_pool` already walks (it sums `amountA` away — this table
-- is that value KEPT), deposits/withdrawals from the op's own
-- `LedgerEntryChanges` (they carry no claim atoms; the op body holds the
-- caller's max/min bounds, not what actually moved).
--
-- Backfill gate (task 0279): `sum(abs(amount))` over the asset-A legs per
-- (pool, ledger) must equal `liquidity_pool_snapshots.gross_volume_a` for that
-- key — both derive from the same atoms, so one query validates the re-parse.
-- ABS, not the positive legs only: `gross_volume_a` is a GROSS figure, summing
-- each atom's A-side amount whichever way the swap went (`append_pool_claims`
-- takes `amount_sold` or `amount_bought` by canonical asset order, both
-- non-negative). A pool that only sold A that ledger has every A leg negative
-- here, and a positives-only sum would read 0 against a non-zero volume.
-- Known exception: an op crossing the SAME pool in BOTH directions nets out in
-- this table by construction (per-op grain) while `gross_volume_a` counts both
-- crossings gross, so such an op is a legitimate mismatch, not a bug.
--
-- No skip index: every read is a `pool_id` PK-prefix seek. This file is
-- FRESH-ONLY (prod is an existing DB), so the table must be CREATEd on prod
-- BEFORE the parser deploy — otherwise live ingest writes into nothing.
CREATE TABLE IF NOT EXISTS lp_operation_amounts (
    pool_id           FixedString(32),
    ledger_sequence   Int64,
    transaction_id    Int64,
    application_order Int16,
    asset_id          Int64,
    amount            Int64
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence, transaction_id, application_order, asset_id);

-- soroban_events: full-content per-event row (ADR 0044 §4a unfold).
-- ZSTD codecs on the ScVal-decoded JSON columns. `signature` is the
-- first-topic Symbol, lifted for cheap `WHERE signature = 'transfer'`.
CREATE TABLE IF NOT EXISTS soroban_events (
    contract_id     Int64,
    transaction_id  Int64,
    ledger_sequence Int64,
    -- OURS, NOT STELLAR'S — and deliberately so. Read this before "fixing"
    -- it to match the protocol.
    --
    -- `event_index` is a flat counter we assign per transaction while walking
    -- the event containers in order (`xdr_parser::event::extract_events`):
    -- tx-level → per-operation → diagnostic. Stellar defines no such number.
    -- CAP-67's V4 meta has three separate event lists and none of them carries
    -- an index; the official identity, the one `getEvents` returns, is
    -- TOID(ledger, tx position, operation position) + the event's position
    -- WITHIN that operation.
    --
    -- Two reasons ours stays:
    --
    -- 1. It is part of this table's ORDER BY, so it co-defines row identity
    --    for `ReplacingMergeTree` dedup. A deterministic per-tx counter is
    --    exactly what replay-idempotency needs — re-processing a ledger
    --    yields the same numbers and the merge collapses cleanly. The
    --    official key would change what counts as the same row.
    -- 2. The official key is NOT EXPRESSIBLE for much of this table. It needs
    --    an operation position, and `op_index` is absent for tx-level events
    --    (fee charge and refund, always) and for every diagnostic event, both
    --    of which this table stores. Adopting it would trade a total key for
    --    one that is null-bearing.
    --
    --    CORRECTION (task 0540, 2026-09-04): this bullet also claimed the same
    --    of "EVERY pre-Protocol-23 event", on the reasoning that V3 meta has no
    --    per-operation container. True of the protocol, FALSE of our input —
    --    the archive hands us `TransactionMeta::V4` across the whole ingested
    --    range. Measured by decoding three archive ledgers end to end
    --    (`xdr-parser/examples/event_op_index_audit.rs`): 1 265 of 1 265
    --    transactions are V4, at protocols 20, 22 and 27, the first being the
    --    ingest floor. So the operation position IS recoverable for all history
    --    — from S3, never from this table, which does not store it.
    --
    --    The narrower claim still holds and is why OUR index stays: for
    --    `soroban_events` as a whole the official key is null-bearing, because
    --    fee and diagnostic events have no operation. For the subset that is
    --    only token movements it is TOTAL (1 770 events audited, none at
    --    transaction level) — which is why task 0540's edge table can consider
    --    it and this table cannot.
    --
    -- So: ours is the better INTERNAL key, theirs is the better key for
    -- exchanging data with the outside world. Different jobs, not a defect.
    --
    -- Revisit only if one of these becomes true, and budget a full rewrite of
    -- the sort key (~10 B rows measured 2026-08-04):
    --   * we publish our own events API and callers need stable, portable
    --     event ids;
    --   * we reconcile our events against an external source by id rather
    --     than by content.
    -- The read path does not depend on it for meaning: the transaction page
    -- states an event's real position from `op_index` and CAP-67 `stage`.
    event_index     Int16,
    event_type      Int16,
    signature       LowCardinality(Nullable(String)),
    topics_xdr      String CODEC(ZSTD(3)),
    data_xdr        String CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, ledger_sequence, transaction_id, event_index);

-- `amount` is a **fold count of invocation-tree nodes** aggregated into
-- this (contract, transaction, ledger) trio (per ADR 0034 PG-side
-- convention; CH inherits same semantic). Multiple invocations of the
-- same contract within the same tx's call graph collapse into one row
-- with `amount` = how many call-graph nodes were folded. NOT a token
-- amount. Real per-invocation `function_name` / `args` / `return_value`
-- live in the Archive XDR (ADR 0029/0034).
CREATE TABLE IF NOT EXISTS soroban_invocations_appearances (
    contract_id          Int64,
    transaction_id       Int64,
    ledger_sequence      Int64,
    caller_id            Nullable(Int64),
    caller_contract_id   Nullable(Int64),
    amount               Int32   -- fold count, see header comment
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, ledger_sequence, transaction_id);

CREATE TABLE IF NOT EXISTS nft_ownership (
    contract_id      Int64,
    token_id         String,
    ledger_sequence  Int64,
    event_order      Int16,
    transaction_id   Int64,
    owner_id         Nullable(Int64),
    event_type       Int16
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, token_id, ledger_sequence, event_order);

-- Task 0217 — quarantine companion to `nft_ownership`. Same row shape +
-- partitioning so promotion (`INSERT … SELECT FROM nft_ownership_pending`)
-- copies parts cleanly. API endpoints never read this table.
CREATE TABLE IF NOT EXISTS nft_ownership_pending (
    contract_id      Int64,
    token_id         String,
    ledger_sequence  Int64,
    event_order      Int16,
    transaction_id   Int64,
    owner_id         Nullable(Int64),
    event_type       Int16
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (contract_id, token_id, ledger_sequence, event_order);

CREATE TABLE IF NOT EXISTS liquidity_pool_snapshots (
    pool_id         FixedString(32),
    ledger_sequence Int64,
    reserve_a       Decimal128(7),
    reserve_b       Decimal128(7),
    total_shares    Decimal128(7),
    -- tvl/volume/fee_revenue columns were removed from the write path (0374
    -- distillation): written as NULL since 0199 (USD is computed at read,
    -- ADR 0053) and read by nothing. DEPLOY ORDER IS LOAD-BEARING: the
    -- clickhouse-rs 0.15 client REFUSES an insert when the table still has a
    -- no-DEFAULT column the struct dropped (proven in the 0374 local e2e;
    -- the 0310 lesson), so prod must run
    --   ALTER TABLE liquidity_pool_snapshots DROP COLUMN tvl, DROP COLUMN volume, DROP COLUMN fee_revenue
    -- BEFORE the writer with this struct starts. (share_token_id had
    -- DEFAULT 0, so its drop has no such ordering constraint.)
    -- Gross trade volume in asset-A units per (pool, ledger), computed from
    -- path-payment claim atoms (task 0261 extractor; written by the 0266
    -- backfill / 0247 wiring) — READ by the chart + 24h volume (kept).
    gross_volume_a  Nullable(Decimal128(7))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (pool_id, ledger_sequence);

----------------------------------------------------------------------
-- Dictionary: hot path for `hash → ledger_sequence` lookups
----------------------------------------------------------------------

-- The dictionary SOURCE clause reads via an inner CH→CH client
-- connection. We use the dedicated `dict_reader` user defined in
-- `users.d/dict.xml` instead of `default`: that user has an empty
-- password (safe — restricted to the loopback interface by its
-- `<networks>` ACL) so this DDL stays free of any committed
-- credential, and a future password rotation on `default` does
-- not require rewriting the schema.
--
-- CONNECT_TIMEOUT / SEND_TIMEOUT / RECEIVE_TIMEOUT in the SOURCE
-- clause govern the dict-load connection's socket timeouts. CH
-- user-profile timeouts apply to QUERY execution but not to the
-- internal client connection an external dictionary opens — so
-- the safe place to bound a stuck dict load is here, in the
-- DDL. 60 s is generous for a loopback intra-container fetch of
-- a single-column index.
CREATE DICTIONARY IF NOT EXISTS transaction_hash_dict (
    hash            String,
    ledger_sequence Int64
)
PRIMARY KEY hash
SOURCE(CLICKHOUSE(
    HOST '127.0.0.1'
    PORT 9000
    TABLE 'transaction_hash_index'
    DB 'default'
    USER 'dict_reader'
    CONNECT_TIMEOUT 5
    SEND_TIMEOUT 60
    RECEIVE_TIMEOUT 60
))
LIFETIME(MIN 300 MAX 360)
LAYOUT(COMPLEX_KEY_CACHE(SIZE_IN_CELLS 1000000));
