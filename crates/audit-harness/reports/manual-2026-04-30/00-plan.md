# Manual endpoint audit — E06–E23 plan

**Date:** 2026-04-30
**Auditor:** stkrolikiewicz
**Runbook:** [`lore/3-wiki/manual-endpoint-audit.md`](../../../../lore/3-wiki/manual-endpoint-audit.md)
**Dataset:** 30k smoke (mainnet ledgers 62016000–62046000), develop binary
post the entire fix stack (lore-0173 + lore-0177 + lore-0181 + lore-0182
+ lore-0183 + lore-0178 + lore-0179 + lore-0184 + Phase 2c LP verifier).

## Continuation context

E01 was never audited under this runbook (the `/network/stats` endpoint
is statistics — easier validated via Phase 1 invariants than per-row
Horizon cross-check). E02–E05 audited 2026-04-28 against the pre-fix
30k smoke and surfaced 0177/0178/0179/0181 — all four now resolved on
develop. Continue from E06.

## Endpoint matrix

| # | Endpoint | Source | Sample picker | Primary cross-check | Notes |
| --- | --- | --- | --- | --- | --- |
| 06 | `GET /accounts/:account_id` | DB-only | 3 G-keys: 1 high-activity classic, 1 Soroban-active, 1 trustline-rich | Horizon `/accounts/:id`, stellar.expert | snapshot vs current state drift expected |
| 07 | `GET /accounts/:account_id/transactions` | DB-only | same 3 accounts | Horizon `/accounts/:id/transactions?limit=10` | keyset cursor; tx visible to participant set |
| 08 | `GET /assets` | DB-only | first page (top by holder_count or similar) | stellar.expert `/assets`, SDF Anchored Assets | XLM-SAC singleton row check |
| 09 | `GET /assets/:id` | DB + S3 entity | 3 assets: 1 classic credit, 1 SAC-wrapped, 1 Soroban-only | stellar.expert `/asset/:code-:issuer` | description/home_page from S3 (task 0164) |
| 10 | `GET /assets/:id/transactions` | DB-only | same 3 assets | stellar.expert per-asset tx | partition-pruned per `created_at` |
| 11 | `GET /contracts/:contract_id` | DB-only | 3 contract IDs: 1 SAC, 1 token, 1 NFT, 1 router/protocol | stellar.expert `/contract/:id` | contract_type SMALLINT decoded |
| 12 | `GET /contracts/:contract_id/interface` | DB-only | 1 with WASM uploaded, 1 missing WASM | stellar.expert WASM tab | wasm_interface_metadata join |
| 13 | `GET /contracts/:contract_id/invocations` | DB-only | 1 high-traffic router (auth-less), 1 SAC | stellar.expert per-contract invocations | post-0183 should cover ≥99 % vs pre-0183 47 % |
| 14 | `GET /contracts/:contract_id/events` | DB + Archive | same 2 | stellar.expert events tab | full topics from archive XDR |
| 15 | `GET /nfts` | DB-only | first page | stellar.expert NFT collection list | filter false-positive guards (task 0118) |
| 16 | `GET /nfts/:id` | DB-only | 3 NFTs: 1 minted, 1 transferred, 1 burned | stellar.expert NFT detail | current_owner from latest event |
| 17 | `GET /nfts/:id/transfers` | DB-only | same 3 | stellar.expert NFT history | event_order monotonic |
| 18 | `GET /liquidity-pools` | DB-only | first page | stellar.expert LP list | post-0179 canonical order |
| 19 | `GET /liquidity-pools/:id` | DB-only | 3 LPs: 1 native+credit, 1 credit+credit same-issuer, 1 credit+credit cross-issuer | stellar.expert LP detail | pool_id verified by archive-diff PR #151 |
| 20 | `GET /liquidity-pools/:id/transactions` | DB-only | same 3 | stellar.expert LP tx | partition prune |
| 21 | `GET /liquidity-pools/:id/chart` | DB-only | same 3 | stellar.expert LP chart | snapshot aggregation |
| 22 | `GET /search` | DB-only | 3 queries: account prefix, contract prefix, asset code | stellar.expert search bar | trigram GIN index |
| 23 | `GET /liquidity-pools/:id/participants` | DB-only | same 3 LPs | (no external — DB ground-truth + participant computation) | task 0126 + 0162 |

## Sample picking strategy (post-backfill)

Run these queries against the 30k DB to pick diverse samples:

```sql
-- Accounts: 3 by activity profile
SELECT account_id, last_seen_ledger,
  (SELECT COUNT(*) FROM transactions t WHERE t.source_id = a.id
     AND t.created_at >= '2026-04-07' AND t.created_at < '2026-04-10') AS tx_count_5d
FROM accounts a
ORDER BY tx_count_5d DESC LIMIT 5;

-- Contracts: top by tx volume
SELECT c.contract_id, c.contract_type,
  (SELECT COUNT(*) FROM soroban_invocations_appearances sia
     WHERE sia.contract_id = c.id AND sia.created_at >= '2026-04-07') AS inv_count
FROM soroban_contracts c
ORDER BY inv_count DESC LIMIT 5;

-- LPs: 3 by reserves
SELECT encode(pool_id, 'hex'), asset_a_type, asset_b_type, fee_bps
FROM liquidity_pools ORDER BY created_at_ledger LIMIT 5;

-- NFTs: 3 by recency
SELECT contract_id, token_id, current_owner_id
FROM nfts ORDER BY minted_at_ledger DESC LIMIT 5;

-- Assets: top by holder_count
SELECT asset_code, holder_count, total_supply
FROM assets ORDER BY holder_count DESC NULLS LAST LIMIT 5;
```

Bias toward diversity (different op types, fee-bump vs plain, mainnet-
prominent contracts) per Filip's runbook §"Step 3 — Pick N random rows".

## Cross-check priority order (per runbook §"External sources")

1. **stellarchain.io** — first choice for Soroban-heavy endpoints (E11–14, E15–17, E18–21, E23)
2. **stellar.expert** — when stellarchain doesn't render the field needed
3. **Horizon API** — fallback for accounts/ledgers/classic tx (E06–07, E08–10 partial)
4. **Soroban RPC** — avoid (24h retention; doesn't help on retrospective check)
5. **Local DB** — the source under test

## Per-endpoint report template

Each `EXX.md` follows the runbook step pattern:

```
# EXX — `<HTTP method + path>`

**Date:** 2026-04-30
**Auditor:** stkrolikiewicz
**Dataset:** 30k smoke (mainnet 62016000–62046000)
**SQL spec:** [...].sql
**Frontend spec:** frontend-overview.md §6.X

## Step 1 — Run endpoint SQL
## Step 2 — Response shape vs frontend §6.X
## Step 3 — Sample N rows
## Step 4 — Cross-check
## Step 5 — Findings
   ✓ Matches
   ⚠ Drift (acceptable)
   ✗ Mismatches (bug → spawn task)

## Bug spawns
## Hand-off
## Cross-link to automated harness
```

## Out-of-scope (this audit pass)

- E01 (`/network/stats`) — covered by Phase 1 invariants more thoroughly than per-row cross-check
- Phase 2b (Soroban RPC) — 24h retention makes retrospective comparison meaningless; deferred per task 0175 description
- Phase 3 (aggregate sanity for continuous monitoring) — separate concern

## Stop-and-confirm rule

Per runbook §"Step 5 — Stop, report findings, hand back": after each
endpoint, write findings, stop, hand back. User runs the same audit on
real wallet StrKeys / contract IDs they care about. Then proceed to
the next endpoint. Do **not** sprint through E06→E23 without
confirmation between each — that defeats the purpose.

## Bug-task spawning

When a real mismatch surfaces (DB disagrees with both external sources
on a value), spawn a backlog bug per `manual-endpoint-audit.md §"Bug
task spawning"` shape — same template Filip used for 0167→0181/0177/0173.
