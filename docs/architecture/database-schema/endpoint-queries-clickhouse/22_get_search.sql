-- Endpoint:     GET /search?q=&type=transaction,contract,asset,account,nft,pool
-- Purpose:      Unified search across all entity types. The API classifies
--               the query (hash-shape, StrKey-shape, plain text) and may
--               restrict via the `type` parameter; this SQL accepts the
--               classification + (optional) type allowlist as inputs and
--               returns grouped, capped result sets per entity type.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.15
-- Schema:       ADR 0044 (CH pilot), parallel to PG ADR 0037
-- Impl:         crates/api/src/search/queries_ch.rs (task 0318)
-- Data sources: DB-only.
-- Inputs:
--   :q                 String           the raw user query (UTF-8)
--   :hash_bytes        FixedString(32)  parsed 32-byte hash if hash-shaped
--                                        (or NULL); passed via unhex(hex)
--   :strkey_prefix     String           StrKey or prefix (G…/C…), NULL otherwise
--   :per_group_limit   Int              cap per entity bucket
--
-- CH PATTERN (task 0318): NOT one UNION-of-six like the PG path. The API fires
-- only the buckets the classifier proves can match, CONCURRENTLY, and skips the
-- rest. This is the minimal-rows-scanned shape — the common hash lookup touches
-- two point-seeks and nothing else — and avoids CH evaluating WHERE-gated
-- branches that would scan state tables for a needle that cannot match.
--
--   classifier mode            buckets fired
--   ------------------------   --------------------------------------------
--   hash_bytes (hex / L-key)   transaction, pool
--   strkey_prefix (G… / C…)    account, contract(prefix), asset, nft
--   code_issuer (CODE:ISSUER)  asset (exact), contract(name), nft
--   plain text                 contract(name), asset, nft
--
-- Intentional divergence from PG: in hash_bytes mode the small-table substring
-- buckets are skipped — an asset_code (≤12 chars) can never contain a 64-hex /
-- 56-char needle (provably empty), and a contract/NFT *named* after a full tx
-- hash is not a real search intent.
--
-- CRITICAL CH-vs-canonical-SQL CORRECTIONS (verified against schema/init.sql
-- and the live CH read modules — the earlier draft of this file was stale):
--   • NFT name lives in `nft_enrichment`, NOT `nfts.name` (vestigial NULL on CH
--     — the live indexer rewrites whole `nfts` rows with metadata NULL; 0231).
--   • Contract name lives in `soroban_contract_metadata`, NOT the dead
--     `soroban_contracts.name` (no writer since task 0297).
--   • Asset issuer StrKey is resolved by a bloom-pruned `accounts WHERE id IN
--     (page ids)` key-seek (`idx_acc_id`), NEVER `LEFT JOIN accounts` — the
--     full-table hash-side build OOMs (CH Code 241, the 0317 trap).
--   • Transaction lookup is a `transaction_hash_index` PK seek (the
--     `transaction_hash_dict` Dictionary is a deferred O(1) optimisation).
--   • All Replacing state tables read FINAL (or argMax for the enrichment
--     side-tables); `nullIf(...)` maps a JOIN miss to NULL (api_reader runs
--     readonly=1 → no `SETTINGS join_use_nulls`).
--
-- The per-bucket reference queries below mirror the Rust implementation 1:1.

-- ── transaction bucket (hash_bytes mode) ────────────────────────────────────
-- Step 1: hash → ledger_sequence (immutable mapping, no FINAL).
SELECT ledger_sequence FROM transaction_hash_index
WHERE hash = unhex(:q_hex) LIMIT 1;
-- Step 2: successful + ledger closed_at (PG `tx_hits` enrichment). Single-row
-- seek: leading-PK `ledger_sequence` + partition prune + `idx_tx_hash_bloom`.
-- closed_at via a BOUNDED `ledgers WHERE sequence = :ledger` sub-select (PK point
-- seek) — NOT a plain `INNER JOIN ledgers`, which builds its hash side from the
-- whole ~3.6M-row ledgers table on prod.
SELECT t.successful, l.closed_at
FROM transactions t
INNER JOIN (SELECT sequence, closed_at FROM ledgers WHERE sequence = :ledger) l
        ON l.sequence = t.ledger_sequence
WHERE t.ledger_sequence = :ledger
  AND intDiv(t.ledger_sequence, 500000) = :partition
  AND t.hash = unhex(:q_hex)
ORDER BY t.application_order LIMIT 1;
-- → identifier = q_hex, label = '' (PG parity), successful/last_activity_at set.

-- ── pool bucket (hash_bytes mode) ───────────────────────────────────────────
-- pool_id is the full ORDER BY key → point seek. Codes are version-stable, so
-- `ORDER BY last_updated_ledger DESC LIMIT 1` collapses versions (no FINAL).
SELECT
    lower(hex(pool_id)) AS pool_hex,
    concat(if(asset_a_code = '', 'XLM', toString(asset_a_code)), ' / ',
           if(asset_b_code = '', 'XLM', toString(asset_b_code))) AS label
FROM liquidity_pools
WHERE pool_id = unhex(:q_hex)
ORDER BY last_updated_ledger DESC LIMIT 1;
-- → identifier = L-StrKey (pool_id_hex_to_strkey at the row boundary).

-- ── account bucket (strkey_prefix mode) ─────────────────────────────────────
-- account_id IS the ORDER BY key → `startsWith() ORDER BY account_id LIMIT` is
-- an EARLY-TERMINATING PK range read (~constant few granules regardless of
-- prefix breadth). NO FINAL: a FINAL merge over the prefix range scales with
-- breadth (measured 98k/246k for `GA` → ~9M on prod 23M); the plain read stays
-- ~32k. Re-ingest dupes of one account_id are contiguous → collapsed in Rust.
-- A C… prefix matches nothing (cheap empty range).
SELECT account_id, ifNull(home_domain, '') AS label
FROM accounts
WHERE startsWith(account_id, :strkey_prefix)
ORDER BY account_id
LIMIT :per_group_limit;

-- ── contract bucket — prefix sub-mode (strkey_prefix mode), two-step ─────────
-- Step 1: early-terminating PK range read for the matching ids (NO FINAL, same
-- reasoning as accounts; contract_id is the immutable identity). Dedupe in Rust.
SELECT contract_id
FROM soroban_contracts
WHERE startsWith(contract_id, :strkey_prefix)
ORDER BY contract_id
LIMIT :per_group_limit;
-- Step 2: bounded name resolution for the ≤limit matched ids — contract_id is
-- the metadata PK, so IN(...) is a key-seek (NOT a whole-metadata aggregation,
-- which cost ~24 MiB/query). soroban_contracts.name is dead → metadata is the
-- live name source. `ifNull(argMax(...), '')` keeps the column non-nullable
-- (name is Nullable(String)) so it decodes into a non-Option Rust field.
SELECT contract_id, ifNull(argMax(name, version), '') AS name
FROM soroban_contract_metadata
WHERE contract_id IN (:matched_contract_ids)
GROUP BY contract_id;

-- ── contract bucket — name sub-mode (plain-text mode) ────────────────────────
-- Substring over on-chain metadata name (Soroban-native tokens; SACs excluded
-- from that table by design). Subquery so argMax is computed once.
SELECT contract_id, name AS label FROM (
    SELECT contract_id, argMax(name, version) AS name
    FROM soroban_contract_metadata GROUP BY contract_id
)
WHERE positionCaseInsensitive(ifNull(name, ''), :q) > 0
LIMIT :per_group_limit;

-- ── asset bucket (strkey_prefix OR plain-text mode), two-step ────────────────
-- Step 1: page matching assets (small state table, FINAL). Join the SMALLER
-- soroban_contracts in-statement for the contract StrKey; do NOT join accounts.
--
-- CODE:ISSUER mode (task 0534) takes its own arm: the pair names exactly one
-- asset, so it is an equality lookup with NO ranking and no holder join — the
-- most precise query is also the cheapest. The issuer resolves through the
-- `accounts` ORDER BY key (point seek, never the ~23M-row hash join).
SELECT a.asset_type,
       nullIf(a.asset_code, '')   AS asset_code,
       nullIf(sc.contract_id, '') AS contract_strkey,
       a.issuer_id
FROM assets a FINAL
LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id
WHERE lower(toString(a.asset_code)) = lower(:code)
  AND a.issuer_id IN (SELECT id FROM accounts WHERE account_id = :issuer)
LIMIT :per_group_limit;

-- Every other mode takes the substring arm below.
-- RANKED (task 0485). Before it, this ended in a bare LIMIT with no ORDER BY,
-- so `USDC` answered with ten `IUSDC` rows and no USDC, and two identical calls
-- could disagree. The tier is exact code > prefix > anywhere — the order
-- follows from what MATCHED, not from an invented weighting — with holder
-- count breaking ties inside a tier (441 assets carry the code `USDC`; the
-- real one has ~691k holders vs ~3k for the runner-up), and the trailing key
-- columns making the order total.
--
-- Both the match and the tier compare the DISPLAYED code: native XLM stores an
-- empty code and renders as `XLM`. The same expression appears in
-- 08_get_assets_list.sql and in the pools predicate — three literal copies,
-- each with a test that fails if it drifts.
--
-- No synonyms: `native` matches the 68 assets whose code contains NATIVE, and
-- nothing else. Native XLM answers to `xlm`, which is what it displays as.
SELECT a.asset_type,
       nullIf(a.asset_code, '')   AS asset_code,
       nullIf(sc.contract_id, '') AS contract_strkey,
       a.issuer_id
FROM assets a FINAL
LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id
LEFT JOIN balance_aggregates bagg ON bagg.asset_id = a.id
WHERE position(lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code))), lower(:q)) > 0
ORDER BY multiIf(lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code))) = lower(:q), 0,
                 startsWith(lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code))),
                            lower(:q)), 1,
                 2) ASC,
         bagg.holder_count DESC NULLS LAST,
         a.asset_type ASC, a.asset_code ASC, a.issuer_id ASC
LIMIT :per_group_limit;
-- Step 2: resolve the page's issuer surrogates → G-StrKey (bloom-pruned seek).
SELECT id, account_id FROM accounts WHERE id IN (:page_issuer_ids) LIMIT 1 BY id;
-- → identifier = COALESCE(asset_code, 'XLM'); label = asset_family_name;
--   route_token = contract StrKey | CODE-ISSUER | native (composed in Rust).

-- ── asset bucket, code_issuer mode (CODE:ISSUER / CODE-ISSUER) ───────────────
-- A fully-qualified asset names exactly one row, so this arm is an equality
-- lookup and needs no ranking — whatever ordering 0485 adds to the substring
-- arm above does not apply here. The classifier only sets this mode when the issuer
-- decodes as a CRC-valid G-StrKey, so a typo falls back to the substring arm
-- above rather than answering with a confidently empty page.
-- accounts is keyed ORDER BY account_id, making the subquery a point seek and
-- never the ~23M-row hash join that OOMs (Code 241).
SELECT a.asset_type,
       nullIf(a.asset_code, '')   AS asset_code,
       nullIf(sc.contract_id, '') AS contract_strkey,
       a.issuer_id
FROM assets a FINAL
LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id
WHERE lower(toString(a.asset_code)) = lower(:code)
  AND a.issuer_id IN (SELECT id FROM accounts WHERE account_id = :issuer)
LIMIT :per_group_limit;
-- Step 2 (issuer resolve) and the route_token composition are unchanged.

-- ── nft bucket (strkey_prefix OR plain-text mode) ────────────────────────────
-- Name + collection come from nft_enrichment (argMax); contract surrogate →
-- C-StrKey via a page-scoped soroban_contracts lookup. Routes on the
-- (contract_id, token_id) composite (ADR 0030).
WITH
enr AS (
    SELECT contract_id, token_id,
           argMax(name, version)            AS name,
           argMax(collection_name, version) AS collection_name
    FROM nft_enrichment GROUP BY contract_id, token_id
),
page AS (
    SELECT n.contract_id AS contract_surrogate, n.token_id AS token_id,
           e.name AS e_name, e.collection_name AS e_collection_name
    FROM nfts n FINAL
    LEFT JOIN enr e ON e.contract_id = n.contract_id AND e.token_id = n.token_id
    WHERE positionCaseInsensitive(ifNull(e.name, ''), :q) > 0
    LIMIT :per_group_limit
),
sc AS (
    SELECT id, any(contract_id) AS contract_id
    FROM soroban_contracts WHERE id IN (SELECT contract_surrogate FROM page) GROUP BY id
)
SELECT ifNull(p.e_name, '') AS identifier, ifNull(p.e_collection_name, '') AS label,
       sc.contract_id AS contract_strkey, p.token_id AS token_id
FROM page p INNER JOIN sc ON sc.id = p.contract_surrogate;
