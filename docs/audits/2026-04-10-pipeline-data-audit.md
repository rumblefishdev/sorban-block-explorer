# Data Pipeline Audit Report

**Date:** 2026-04-10  
**Author:** stkrolikiewicz  
**Scope:** Full pipeline data correctness audit + Deliverable 1 readiness assessment

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Code Audit Findings](#2-code-audit-findings)
3. [Deliverable 1 Requirements Matrix](#3-deliverable-1-requirements-matrix)
4. [Deliverable 1 Acceptance Criteria Status](#4-deliverable-1-acceptance-criteria-status)
5. [Nullable Fields Analysis](#5-nullable-fields-analysis)
6. [Technical Design Coverage Gaps](#6-technical-design-coverage-gaps)
7. [Manual Verification Checklist](#7-manual-verification-checklist)
8. [Third-Pass Audit Findings (F18-F25)](#8-third-pass-audit-findings-f18-f25)
9. [Enrichment Pipeline Gap](#9-enrichment-pipeline-gap)
10. [Parallel Backfill Safety](#10-parallel-backfill-safety)
11. [Recommendations](#11-recommendations)

---

## 1. Executive Summary

The data pipeline implementation is **largely correct** — 117/117 unit tests pass, atomic
per-ledger persistence works, idempotency constraints are sound, and the write ordering
respects FK dependencies.

However, a second independent audit found **2 HIGH severity** and **5 MEDIUM severity**
issues that the first pass missed. The most impactful is NFT false positives from fungible
token transfers (every SEP-0041 transfer creates a spurious `nfts` record).

Deliverable 1 is **~85% complete**. Two active tasks are hard blockers for acceptance
criteria. Several technical design requirements have no corresponding backlog task.

### Key Numbers

| Metric                         | Value                                                                |
| ------------------------------ | -------------------------------------------------------------------- |
| Unit tests passing             | 117/117                                                              |
| HIGH severity findings         | 2 (F7, F9)                                                           |
| MEDIUM severity findings       | 10 (F4,F6,F8,F12,F17-F22)                                            |
| LOW severity findings          | 3 (F23, F24, F25)                                                    |
| D1 completion                  | ~85%                                                                 |
| D1 hard blockers               | 4 (0118, 0119, 0130, 0134) — 0030 superseded by 0117; 0036 completed |
| Nullable fields with no plan   | 5                                                                    |
| Tech design reqs without tasks | 9 (all now have tasks)                                               |
| New tasks created              | 18 (0118-0135)                                                       |
| Intentionally not fixed        | 6 (F4, F6, F17, F23-F25)                                             |

> **Note on finding numbers:** Findings are numbered F4-F25. Numbers F1-F3 and F5 were
> used in internal working notes and consolidated into other findings before this report.

---

## 2. Code Audit Findings

### 2.1 HIGH Severity

#### F9: NFT False Positives from Fungible Token Transfers

**File:** `crates/xdr-parser/src/nft.rs:171-174`

`looks_like_token_id()` accepts `i128` data type, which is the standard amount type in
SEP-0041 fungible token transfers. Every standard token transfer (USDC, XLM wrapping, etc.)
creates a false-positive NFT record.

**Impact:** The `nfts` table will be flooded with spurious entries — potentially millions of
records from normal token activity.

**Root cause:** The function excludes `void`, `map`, `vec`, `error` but does not exclude
`i128`/`i64`/`u128` (fungible amounts).

**Current state (2026-04-13):** The code now has a doc comment (lines 162-170) explicitly
acknowledging this limitation — some NFT contracts (e.g. jamesbachini) use `i128` for token
IDs, so a blanket exclusion would cause false negatives. A test
`i128_token_id_not_excluded` confirms the current behavior is intentional pending a proper
fix via WASM spec analysis. The bug remains unfixed.

**Fix:** Add numeric ScVal types to the exclusion list in `looks_like_token_id()`, or
require WASM spec analysis to confirm NFT contract type before inserting.

#### F7: Account Balances — Only Native XLM Extracted

**File:** `crates/xdr-parser/src/state.rs:139-140`

`extract_account_states()` hardcodes a single native XLM balance:

```rust
let balances = serde_json::json!([{ "asset_type": "native", "balance": balance }]);
```

Trustline balances (credit_alphanum4, credit_alphanum12) are in separate `trustline`
LedgerEntry types that are never processed. Contract token balances are in `contract_data`
entries, also not processed.

**Impact:** Any UI showing account balances will only display native XLM. The `balances`
JSONB column schema was designed for multiple balance types but always contains a single
entry.

**Fix:** Process `trustline` entry types in `extract_ledger_entry_changes()` and merge them
into the account's `balances` array.

### 2.2 MEDIUM Severity

#### F4: root_return_value Shared Across All Auth Entries

**File:** `crates/xdr-parser/src/invocation.rs:60-95`

`SorobanTransactionMeta.return_value` is the return value of the host function call, not of
individual auth entries. If a transaction has multiple auth entries, each root invocation
gets the same return value, but only one actually produced it.

**Impact:** Incorrect `return_value` attribution for multi-auth-entry transactions. Common
single-auth-entry case is correct.

#### F8: Soroban-Native Tokens (Non-SAC) Not Detected

**File:** `crates/xdr-parser/src/state.rs:50-56`

`contract_type` classification is binary: SAC = "token", everything else = "other".
WASM-based token contracts implementing SEP-0041 are never added to the `tokens` table.

**Impact:** Soroban-native tokens won't appear in token listings or search results.

#### F6: CreateContractHostFn Propagates None as Caller

**File:** `crates/xdr-parser/src/invocation.rs:247-257`

Contract creation invocations have `contract_id = None`, which propagates as
`caller_account = None` to sub-invocations, breaking the caller chain.

**Impact:** Low in practice — contract creation rarely has sub-invocations in auth trees.

#### F12: Tokens ON CONFLICT Without Constraint Specification

**File:** `crates/db/src/soroban.rs:398`

`ON CONFLICT DO NOTHING` without specifying which constraint. Partial unique indexes may not
catch all duplicates for future token types.

**Impact:** Latent issue — no problem with current SAC-only token detection.

#### F17: NFT minted_at_ledger Immutable After First Insert

**File:** `crates/db/src/soroban.rs:452-458`

`minted_at_ledger` is only set on INSERT, never updated. Burn-then-remint scenario or
partial backfill starting after original mint leaves it as NULL permanently.

**Impact:** Minor data accuracy issue for re-minted NFTs.

### 2.3 Confirmed Correct

| Area                                       | Verdict                                             |
| ------------------------------------------ | --------------------------------------------------- |
| Atomic per-ledger DB transaction           | Correct — `pool.begin()` wraps all writes           |
| FK dependency ordering                     | Correct — ensure_contracts -> events -> invocations |
| WASM interface staging (2-ledger pattern)  | Correct — eventually consistent                     |
| Watermark upserts (accounts, NFTs, pools)  | Correct — WHERE clause prevents stale overwrites    |
| Idempotent replay (ON CONFLICT DO NOTHING) | Correct — no delete-then-reinsert patterns          |
| ScVal JSON encoding (18 types)             | Correct — large integers as strings                 |
| Empty ledger / empty transaction handling  | Correct — graceful no-op                            |
| Concurrent processing of same ledger       | Correct — idempotency handles it                    |

---

## 3. Deliverable 1 Requirements Matrix

### 3.1 Implemented (D1 scope)

| Requirement                                  | Evidence                   |
| -------------------------------------------- | -------------------------- |
| Galexie ECS Fargate on mainnet               | Tasks 0034, 0111 archived  |
| Lambda Ledger Processor                      | Task 0029 archived         |
| Parse ledgers, transactions, operations      | `crates/xdr-parser/src/`   |
| Parse Soroban invocations + CAP-67 events    | Tasks 0025, 0026 archived  |
| Parse accounts, tokens, NFTs, pools          | Task 0027 archived         |
| Contract interface extraction (WASM)         | Task 0104 archived         |
| Full DB schema (12 tables + staging)         | 9 migrations               |
| Idempotent write constraints                 | Migration 0007             |
| Rust API scaffolding (axum + utoipa)         | Task 0042 archived         |
| AWS CDK infrastructure (VPC, RDS, Lambda...) | Tasks 0031-0035, 0097-0100 |
| CI/CD pipeline (GitHub Actions)              | Task 0039 archived         |

### 3.2 Remaining (D1 scope)

| Requirement                        | Status  | Task | Blocker?                                                |
| ---------------------------------- | ------- | ---- | ------------------------------------------------------- |
| Historical backfill (local)        | Done    | 0117 | **YES — AC#2** (0030 superseded by 0117 backfill-bench) |
| CloudWatch dashboards + lag alarm  | Done    | 0036 | AC#5 — completed                                        |
| Fix NFT false positives            | Backlog | 0118 | **YES — before backfill**                               |
| Trustline balance extraction       | Backlog | 0119 | **YES — before backfill**                               |
| Historical partitions for backfill | Backlog | 0130 | **YES — blocks backfill**                               |
| Envelope/meta ordering validation  | Backlog | 0134 | **YES — data integrity**                                |
| Environment-specific CDK config    | Backlog | 0038 | Risk for AC#4                                           |
| Galexie config + testnet valid.    | Backlog | 0041 | Risk for AC#1                                           |
| CI workflow optimization           | Backlog | 0112 | No                                                      |

---

## 4. Deliverable 1 Acceptance Criteria Status

| AC# | Criterion                                                   | Met?       | Notes                                                                                             |
| --- | ----------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------- |
| 1   | S3 consecutive LedgerCloseMeta files matching mainnet times | Partially  | Live ingestion works. Formal validation (0041) TBD.                                               |
| 2   | RDS ledgers — no gaps from backfill start to current tip    | **No**     | **Blocked by backfill** — tool ready (0117), pre-backfill tasks (0118, 0119, 0130, 0134) pending. |
| 3   | soroban_events spot-check by known Soroswap/Aquarius hashes | Unverified | Requires manual spot-check. See Section 7.                                                        |
| 4   | cdk deploy from clean account — no manual steps             | Partially  | Core CDK works. Task 0038 (env config) not done.                                                  |
| 5   | CloudWatch dashboard + Galexie lag alarm on staging         | **Yes**    | Task 0036 completed. Verify dashboards on staging.                                                |

---

## 5. Nullable Fields Analysis

### 5.1 Fields With Full Coverage (35 of 37)

These nullable fields are populated at ingestion (D1, implemented) and consumed by planned
API (D2, backlog) and frontend (D2, backlog) tasks.

| Table               | Field              | Why Nullable                    | D2 Task    |
| ------------------- | ------------------ | ------------------------------- | ---------- |
| transactions        | result_code        | Defensive; always populated     | 0046       |
| transactions        | result_meta_xdr    | Only V3/V4 metadata             | (internal) |
| transactions        | memo_type          | Optional in Stellar protocol    | 0046       |
| transactions        | memo               | Optional in Stellar protocol    | 0046       |
| transactions        | parse_error        | Defensive; always populated     | 0046       |
| transactions        | operation_tree     | NULL for classic tx             | 0070       |
| soroban_contracts   | wasm_hash          | SACs have no WASM               | 0050       |
| soroban_contracts   | deployer_account   | Defensive                       | 0050       |
| soroban_contracts   | deployed_at_ledger | Defensive                       | 0050       |
| soroban_contracts   | contract_type      | Defensive                       | 0050       |
| soroban_contracts   | is_sac             | DEFAULT FALSE; always populated | 0050       |
| soroban_contracts   | metadata           | 2-ledger staging pattern        | 0050       |
| soroban_invocations | contract_id        | NULL for contract creation      | 0050       |
| soroban_invocations | caller_account     | Edge case root extraction       | 0050       |
| soroban_invocations | function_args      | Always Some in practice         | 0071       |
| soroban_invocations | return_value       | NULL for sub-invocations        | 0071       |
| soroban_events      | contract_id        | System events have no contract  | 0050       |
| accounts            | home_domain        | Optional Stellar field          | 0048       |
| tokens              | asset_code         | Soroban-native tokens           | 0049       |
| tokens              | issuer_address     | Soroban-native tokens           | 0049       |
| tokens              | contract_id        | Classic tokens                  | 0049       |
| tokens              | name               | Optional metadata               | 0049       |
| tokens              | total_supply       | Derived state                   | 0049       |
| tokens              | holder_count       | Derived state                   | 0049       |
| nfts                | collection_name    | Sparse metadata                 | 0051       |
| nfts                | owner_account      | Burned NFTs                     | 0051       |
| nfts                | name               | Sparse metadata                 | 0051       |
| nfts                | media_url          | Sparse metadata                 | 0051       |
| nfts                | metadata           | Sparse metadata                 | 0051       |
| nfts                | minted_at_ledger   | Set on mint only                | 0051       |

### 5.2 Fields WITHOUT Plan (5 of 37)

| Table                    | Field       | Why Nullable                    | Problem                                                              |
| ------------------------ | ----------- | ------------------------------- | -------------------------------------------------------------------- |
| tokens                   | metadata    | Token description, icon, domain | `convert.rs:168` hardcodes `None`. No task populates it.             |
| liquidity_pools          | tvl         | Requires USD price oracle       | Extraction outputs `Option` but likely always `None`. No price feed. |
| liquidity_pool_snapshots | tvl         | Same as above                   | Same — no external pricing.                                          |
| liquidity_pool_snapshots | volume      | Trade-level aggregation needed  | Not implemented in XDR parser. Likely always NULL.                   |
| liquidity_pool_snapshots | fee_revenue | Derived from volume \* fee_bps  | NULL whenever volume is NULL.                                        |

---

## 6. Technical Design Coverage Gaps

### 6.1 Requirements Without Backlog Tasks

| Requirement                                    | Tech Design Section | Deliverable | Impact                                                                       |
| ---------------------------------------------- | ------------------- | ----------- | ---------------------------------------------------------------------------- |
| NFT transfer history (GET /nfts/:id/transfers) | 1.3, 2.3            | D2          | Schema gap — no `nft_transfers` table.                                       |
| Transaction signatures (signer, weight, hex)   | 1.3                 | D2          | XDR parser does not extract signatures.                                      |
| XDR decoding service (on-demand decode)        | 2.2, 5.1            | D2          | 4 estimated days, no task.                                                   |
| Pool participants (providers + shares)         | 1.3                 | D2          | No per-provider tracking in schema.                                          |
| Token holder_count ongoing updates             | 1.3                 | D2          | Initial detection only, no ongoing updates.                                  |
| ~~Network TPS computation~~                    | 1.3                 | D2          | **Already covered** by task 0045 (`GET /v1/network/stats` with `tps` field). |
| 7-day post-launch monitoring report            | D3 AC#6             | D3          | Contractual obligation — no task.                                            |
| Public GitHub repository                       | D3 AC#2             | D3          | Contractual obligation — no task.                                            |
| Stellar team read-only IAM access              | D3 AC#3             | D3          | Contractual obligation — no task.                                            |

### 6.2 Potential Dead Weight in Schema

| Element                                   | Notes                                                                 |
| ----------------------------------------- | --------------------------------------------------------------------- |
| `transactions.result_meta_xdr`            | Stored but never returned by any planned endpoint. Internal use only. |
| `wasm_interface_metadata` table           | Internal staging table — correctly consumed by indexer, not exposed.  |
| `liquidity_pool_snapshots.volume/fee_rev` | Schema columns exist but likely always NULL without trade tracking.   |

---

## 7. Manual Verification Checklist

### 7.1 On Staging Database (via bastion host / SSM tunnel)

**AC#2 — Ledger gap check:**

```sql
SELECT l1.sequence + 1 AS gap_start,
       MIN(l2.sequence) - 1 AS gap_end
FROM ledgers l1
LEFT JOIN ledgers l2 ON l2.sequence > l1.sequence
GROUP BY l1.sequence
HAVING MIN(l2.sequence) - l1.sequence > 1
LIMIT 20;
```

**AC#3 — Spot-check known Soroswap/Aquarius events:**

```sql
SELECT t.hash, COUNT(e.id) AS event_count
FROM transactions t
JOIN soroban_events e ON e.transaction_id = t.id
WHERE t.hash IN (
  '<known_soroswap_hash>',
  '<known_aquarius_hash>',
  '<known_phoenix_hash>'
)
GROUP BY t.hash;
```

**Verify F9 — NFT false positives:**

```sql
SELECT COUNT(*) AS total_nfts FROM nfts;
-- If >10K, likely flooded with false positives from fungible transfers
```

**Verify F7 — Account balances completeness:**

```sql
SELECT balances FROM accounts LIMIT 10;
-- If all entries show only [{"asset_type":"native",...}], confirms F7
```

**Verify F8 — Token type coverage:**

```sql
SELECT asset_type, COUNT(*) FROM assets GROUP BY asset_type;
-- If no 'soroban' type, confirms F8
```

**Verify assets.metadata always NULL:**

```sql
SELECT COUNT(*) FROM assets WHERE description IS NOT NULL;
-- Expected: 0
```

**LP snapshot quality:**

```sql
SELECT
  COUNT(*)          AS total,
  COUNT(tvl)        AS has_tvl,
  COUNT(volume)     AS has_volume,
  COUNT(fee_revenue) AS has_fee_revenue
FROM liquidity_pool_snapshots;
-- Expected: has_tvl/volume/fee_revenue likely 0
```

### 7.2 On AWS Console

| Check | How                                                  | Expected                                |
| ----- | ---------------------------------------------------- | --------------------------------------- |
| AC#1  | S3 console -> `stellar-ledger-data/` -> sort by date | Consecutive files with ~5-6s intervals  |
| AC#4  | Run `cdk deploy` on clean staging account            | Complete without manual steps           |
| AC#5  | CloudWatch console -> Dashboards                     | Task 0036 completed — verify on staging |

### 7.3 In Code

| Check           | File                                 | What to look for                   |
| --------------- | ------------------------------------ | ---------------------------------- |
| F9 confirmation | `crates/xdr-parser/src/nft.rs:171`   | `looks_like_token_id` accepts i128 |
| F7 confirmation | `crates/xdr-parser/src/state.rs:139` | Hardcoded `"native"` only          |
| F8 confirmation | `crates/xdr-parser/src/state.rs:50`  | Binary SAC/"other" classification  |

---

## 8. Third-Pass Audit Findings (F18-F25)

A pessimistic third-pass audit found 8 additional issues not covered by F4-F17.

### 8.1 MEDIUM Severity

#### F18: No Validation of Envelope/Meta Ordering

**Files:** `envelope.rs`, `process.rs:22-48`, `transaction.rs:34`

No assertion that `envelopes.len() == tx_metas.len()`. No hash-based cross-check that each
envelope matches its meta. For V1/V2 parallel Soroban phases, ordering relies on protocol
convention with no runtime verification. Mismatch produces silently corrupted data (wrong
operations attributed to wrong transactions).

#### F19: Historical Partitions Missing (2023-11 through 2026-03)

**Files:** `migrations/0004`, `migrations/0006`

Only Apr-Jun 2026 partitions exist. Backfill data (29 months) lands in DEFAULT partition,
defeating partitioning purpose. Splitting populated DEFAULT later requires exclusive locks.

**Task created:** 0130 (milestone 1 — must run before backfill)

#### F20: Operations Partition Strategy Useless at Scale

**File:** `migrations/0002_create_operations.sql`

Partitioned by `transaction_id` range with only 0-10M bucket. Mainnet has hundreds of
millions of transactions — virtually all data in DEFAULT. Partition pruning never activates.

**Task created:** 0131 (milestone 2)

#### F21: Missing Database Indexes for Planned API Queries

Events lack composite `(contract_id, event_type, created_at)` index. Operations lack index
on `type` column.

**Task created:** 0132 (milestone 2)

#### F22: Full-Text Search Only on soroban_contracts (Usually NULL)

`search_vector` GIN index exists only on `soroban_contracts.metadata->>'name'`, which is
NULL for most contracts. No search capability on tokens, accounts, or NFTs.

**Task created:** 0133 (milestone 2)

### 8.2 LOW Severity

#### F23: Duplicated soroban_return_value Function

**Files:** `operation.rs:55`, `invocation.rs:279`

Identical function in two files — divergence risk if one is updated without the other.

#### F24: Account Removal (Merge) Not Tracked

**File:** `state.rs:117-120`

`extract_account_states()` skips `"removed"` changes. Merged accounts remain in DB as if
active. Not critical (Horizon does the same) but misleading on detail pages.

#### F25: token_id_to_string Edge Case for Large Numbers

**File:** `state.rs:333-350`

Numbers larger than `i64::MAX` may produce inconsistent formatting. Unlikely edge case.

### 8.3 Task Review Notes

| Task | Issue Found                                                             |
| ---- | ----------------------------------------------------------------------- |
| 0118 | Updated: added nuance about NFT contracts using i128 as token IDs       |
| 0121 | Updated: added dependency on 0118 (false positives corrupt history too) |
| 0123 | Updated: added AC requiring ADR 0004 amendment                          |
| 0128 | Note: git history scrubbing effort may be significant                   |

### 8.4 Intentionally Not Fixed

These findings were evaluated and deliberately left without tasks. The cost of fixing
exceeds the user impact, or the behavior is by design.

| Finding | Severity | Rationale                                                                                                              |
| ------- | -------- | ---------------------------------------------------------------------------------------------------------------------- |
| F4      | MEDIUM   | By Soroban design — one return_value per host function call, not per auth entry. Common single-auth case is correct.   |
| F6      | MEDIUM   | Edge case — contract creation rarely has sub-invocations. Low user impact.                                             |
| F12     | MEDIUM   | Latent risk only — harmless with current SAC-only detection. Fix scoped in task 0120 (soroban-native token detection). |
| F17     | MEDIUM   | Minor — minted_at_ledger stays from original mint. Acceptable for explorer.                                            |
| F23     | LOW      | Code hygiene — duplicated function. Refactor when touched next.                                                        |
| F24     | LOW      | Horizon does the same — merged accounts shown as last known state.                                                     |
| F25     | LOW      | Theoretical — numbers > i64::MAX as token IDs extremely unlikely.                                                      |

### 8.5 Security Note

**Before task 0128 (public repo):** AWS Account ID `750702271865` and a full ACM certificate
ARN are committed in `infra/envs/staging.json` and a worklog file. These must be scrubbed
from git history before the repo is made public.

---

## 9. Enrichment Pipeline Gap

### 9.1 Problem

The technical design specifies data that **cannot be extracted from XDR alone**, but does
not define how or when this data is computed. The design describes three layers (Ingestion,
API, Frontend) but is missing a fourth — **Enrichment**:

> Ingestion (XDR to DB) **-> Enrichment ->** API (DB to JSON) -> Frontend

Six columns are always NULL because they require data from outside the XDR:

| Column                     | Tech Design Ref | What It Needs                | Why Indexer Cannot Do It                          |
| -------------------------- | --------------- | ---------------------------- | ------------------------------------------------- |
| `assets.holder_count`      | L166, L175      | Count of all token holders   | Requires global state aggregation, not per-ledger |
| `tokens.metadata`          | L176            | Name, icon, domain           | Data in external stellar.toml (SEP-1), not in XDR |
| `liquidity_pools.tvl`      | L223, L409      | Reserves x USD price         | USD prices not on-chain, requires price oracle    |
| `lp_snapshots.tvl`         | L409            | Time-series TVL              | Same as above                                     |
| `lp_snapshots.volume`      | L223, L409      | Sum of swaps per pool/period | Requires event aggregation across time windows    |
| `lp_snapshots.fee_revenue` | L223, L409      | volume x fee_bps / 10000     | Derived, NULL when volume is NULL                 |

### 9.2 Why the Tech Design Omits This

1. **No "enrichment" section.** The data pipeline (Section 5) covers XDR parsing and DB
   writes as the only data population step.
2. **Only hint:** L601 — "Materialized views for aggregated statistics" in the scaling
   table. Suggests the authors considered DB-side aggregation but never elaborated.
3. **Risk section acknowledges the gap without solving it:** L1265 — "LP chart data
   requires aggregation. Mitigated by building these pages last." — i.e., defer it.

### 9.3 Proposed Enrichment Architecture

The enrichment pipeline sits between ingestion and API. It has three execution modes:

**Inline (synchronous, per ledger):**
Runs inside the indexer Lambda during normal ledger processing. No extra infrastructure.
Used for: `holder_count` increment/decrement at each trustline change.

**Scheduled (asynchronous, EventBridge cron):**
A dedicated "Enrichment Worker" Lambda triggered on a schedule. Fetches external data and
computes aggregates. Used for: token metadata (daily TOML fetch), LP TVL (every 5 min,
price x reserves), LP volume (every 5 min, swap aggregation).

**One-time (post-backfill):**
Manual or deploy-triggered run after historical backfill completes. Used for: full
holder_count recount, initial TVL snapshot.

| Type      | Trigger                     | Infrastructure                     | Tasks      |
| --------- | --------------------------- | ---------------------------------- | ---------- |
| Inline    | Every ledger (in indexer)   | None extra, code in indexer Lambda | 0135       |
| Scheduled | EventBridge cron (5m/daily) | New Enrichment Worker Lambda       | 0124, 0125 |
| One-time  | After backfill              | Manual run or deploy-triggered     | 0135       |

Estimated cost: ~$1-3/mo (Lambda ARM64 256MB, free-tier EventBridge + CoinGecko API).

### 9.4 Write-Only Columns (populated but no API reads them)

These columns are populated by the indexer but not referenced in any API response spec.
None should be removed — all have valid use cases:

| Column                              | Tech Design Ref                | Recommendation                                                                                     |
| ----------------------------------- | ------------------------------ | -------------------------------------------------------------------------------------------------- |
| `transactions.result_meta_xdr`      | L306, L767, L815               | Keep. L815: "preserved for advanced decode/debug". Large storage cost — consider S3 cold archival. |
| `transactions.result_code`          | Not in API spec                | Keep + add to API. Useful for filtering failed tx by error type.                                   |
| `transactions.parse_error`          | Not in design                  | Keep. Internal diagnostic, zero storage cost.                                                      |
| `soroban_invocations.function_args` | L921 (schema only)             | Keep. Enables per-contract invocation queries with args.                                           |
| `soroban_invocations.return_value`  | L922 (schema only)             | Keep. Same reasoning.                                                                              |
| `accounts.home_domain`              | L668, L801 but not in API spec | Keep + add to API. Standard Stellar field. Likely a tech design omission.                          |

### 9.5 Column Removal Assessment

**No columns should be removed.** Every column either:

- Is required by the tech design (placeholder columns — tasks 0124, 0125, 0135)
- Is explicitly preserved by the tech design (result_meta_xdr — L815)
- Has operational value (parse_error, result_code)
- Enables query patterns not served by other columns (function_args, return_value)

---

## 10. Parallel Backfill Safety

### 10.1 Current State

The indexer is designed for idempotency. Parallel backfill (multiple workers processing
different ledger ranges concurrently, out-of-order) is **safe** for the current codebase
with two known post-correction issues.

### 10.2 Table-by-Table Assessment

**Safe for out-of-order processing (no issues):**

| Table                     | Mechanism                             | Why Safe                                     |
| ------------------------- | ------------------------------------- | -------------------------------------------- |
| `ledgers`                 | ON CONFLICT DO NOTHING                | Immutable, first writer wins                 |
| `transactions`            | ON CONFLICT (hash) DO UPDATE          | Idempotent, same data                        |
| `operations`              | ON CONFLICT DO NOTHING                | Immutable                                    |
| `soroban_events`          | ON CONFLICT DO NOTHING                | Immutable                                    |
| `soroban_invocations`     | ON CONFLICT DO NOTHING                | Immutable                                    |
| `soroban_contracts`       | COALESCE(existing, new)               | First-write-wins, metadata merges additively |
| `wasm_interface_metadata` | UPSERT by wasm_hash                   | Eventually consistent across ledgers         |
| `tokens`                  | ON CONFLICT DO NOTHING                | First-write-wins                             |
| `accounts`                | WHERE last_seen_ledger <= EXCLUDED    | Watermark: newer ledger wins, stale rejected |
| `liquidity_pools`         | WHERE last_updated_ledger <= EXCLUDED | Same watermark pattern                       |
| `nfts`                    | WHERE last_seen_ledger <= EXCLUDED    | Same watermark pattern                       |

**Known issues requiring post-backfill correction:**

| Issue               | Table      | Cause                                                                                                                | Fix                            |
| ------------------- | ---------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| `first_seen_ledger` | `accounts` | Set on INSERT only. If transfer (n+200) before creation (n), value is wrong. Watermark rejects the later correction. | Post-backfill correction query |
| `minted_at_ledger`  | `nfts`     | INSERT only, not in UPDATE SET. Transfer before mint = NULL permanently.                                             | Post-backfill correction query |

**Post-backfill correction SQL:**

```sql
-- Fix accounts.first_seen_ledger
UPDATE accounts a SET first_seen_ledger = sub.min_ledger
FROM (
  SELECT source_account, MIN(ledger_sequence) AS min_ledger
  FROM transactions GROUP BY source_account
) sub
WHERE a.account_id = sub.source_account
  AND a.first_seen_ledger > sub.min_ledger;
```

### 10.3 Impact of New Tasks on Parallel Safety

| Task | Parallel-safe? | Issue                                                    | Mitigation                                                               |
| ---- | -------------- | -------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0118 | Yes            | Filter change only, no state                             | None needed                                                              |
| 0119 | Conditional    | Trustline for account that does not exist yet (FK)       | Add ensure_account_exists before upsert                                  |
| 0130 | Yes            | DDL only, run before backfill                            | None needed                                                              |
| 0134 | Yes            | Validation logic, no state                               | None needed                                                              |
| 0135 | **No**         | holder_count +1/-1 is not safe for concurrent increments | Disable inline during parallel backfill. Use post-backfill recount only. |

### 10.4 Recommended Backfill Execution Order

| Step | When            | What                                                                      |
| ---- | --------------- | ------------------------------------------------------------------------- |
| 1    | Before backfill | Run tasks 0118, 0119, 0130, 0134                                          |
| 2    | Before backfill | Disable inline holder_count increment (task 0135)                         |
| 3    | During backfill | Run N parallel workers on non-overlapping ledger ranges                   |
| 4    | After backfill  | Post-correction: first_seen_ledger, minted_at_ledger                      |
| 5    | After backfill  | One-time holder_count full recount (task 0135)                            |
| 6    | After backfill  | Enrichment jobs: token metadata (0124), LP analytics (0125)               |
| 7    | Ongoing         | Switch to live ingestion (single worker) with inline holder_count enabled |

### 10.5 NFT Table: Field-by-Field Mutation Analysis

| Field              | Source            | Mint      | Transfer  | Burn | Mutates?                           |
| ------------------ | ----------------- | --------- | --------- | ---- | ---------------------------------- |
| `contract_id`      | event.contract_id | yes       | yes       | yes  | No (PK)                            |
| `token_id`         | event.data        | yes       | yes       | yes  | No (PK)                            |
| `collection_name`  | hardcoded None    | null      | null      | null | Never — placeholder for enrichment |
| `owner_account`    | event topics (to) | recipient | new owner | None | Yes, every transfer/burn           |
| `name`             | hardcoded None    | null      | null      | null | Never — placeholder for enrichment |
| `media_url`        | hardcoded None    | null      | null      | null | Never — placeholder for enrichment |
| `metadata`         | hardcoded None    | null      | null      | null | Never — placeholder for enrichment |
| `minted_at_ledger` | event.ledger_seq  | Some(seq) | None      | None | INSERT only — not in UPDATE SET    |
| `last_seen_ledger` | event.ledger_seq  | yes       | yes       | yes  | Yes, every interaction (watermark) |

NFT metadata (name, media_url, metadata, collection_name) requires `token_uri()` RPC calls
to the contract — not available from XDR events. This is an enrichment job (same pattern as
token metadata in task 0124). Parallel backfill has zero impact on NFT data correctness
beyond the known `minted_at_ledger` INSERT-only issue.

---

## 11. Recommendations

### 11.1 Immediate (Before D1 Closeout)

1. **Complete task 0118** (NFT false positives fix) — before backfill, prevents millions of spurious records
2. **Complete task 0119** (trustline balance extraction) — before backfill, dormant accounts won't self-fix
3. **Complete task 0130** (historical partitions) — before backfill, prevents DEFAULT partition bloat
4. **Complete task 0134** (envelope/meta validation) — data integrity guard
5. **Run backfill** via backfill-bench (task 0117, tool ready) — hard blocker for AC#2. Task 0030 (Fargate) superseded.
6. ~~Complete task 0036~~ — **Done.** Verify CloudWatch dashboards exist on staging.
7. **Run AC#3 spot-check** — manual DB query with known Soroswap/Aquarius hashes

### 11.2 Before D2 Start

8. **Fix F8** (task 0120, Soroban-native token detection) — core feature gap for token listings

### 11.3 All Created Tasks (0118-0135)

| Task | Title                                   | Severity | Milestone | Effort |
| ---- | --------------------------------------- | -------- | --------- | ------ |
| 0118 | Fix NFT false positives (F9)            | HIGH     | **1**     | Small  |
| 0119 | Trustline balance extraction (F7)       | HIGH     | **1**     | Medium |
| 0120 | Soroban-native token detection (F8)     | MEDIUM   | 2         | Medium |
| 0121 | NFT transfer history schema + API       | MEDIUM   | 2         | Medium |
| 0122 | Transaction signatures extraction       | LOW      | 2         | Small  |
| 0123 | XDR decoding service (API)              | MEDIUM   | 2         | Medium |
| 0124 | Token metadata enrichment (scheduled)   | LOW      | 2         | Medium |
| 0125 | LP TVL / volume / fee revenue (sched.)  | LOW      | 2         | Large  |
| 0126 | Pool participants tracking              | LOW      | 2         | Medium |
| 0127 | D3: Post-launch monitoring report       | LOW      | 3         | Small  |
| 0128 | D3: Public GitHub repo setup            | LOW      | 3         | Small  |
| 0129 | D3: Stellar team IAM access             | LOW      | 3         | Small  |
| 0130 | Historical partition gaps (F19)         | HIGH     | **1**     | Small  |
| 0131 | Operations partition strategy (F20)     | MEDIUM   | 2         | Medium |
| 0132 | Missing DB indexes (F21)                | MEDIUM   | 2         | Small  |
| 0133 | Full-text search indexes (F22)          | MEDIUM   | 2         | Medium |
| 0134 | Envelope/meta ordering validation (F18) | MEDIUM   | **1**     | Small  |
| 0135 | Token holder_count tracking (inline)    | MEDIUM   | 2         | Medium |
