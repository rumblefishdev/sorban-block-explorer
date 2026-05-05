---
id: '0138'
title: 'Indexer: extract contract token balances from contract_data entries'
type: FEATURE
status: archive
related_adr: ['0023', '0035', '0037']
related_tasks: ['0119', '0120', '0135', '0156']
superseded_by: []
tags: [layer-indexer, layer-db, audit-F7, superseded, scope-out]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-15'
    status: backlog
    who: FilipDz
    note: 'Spawned from 0119 out-of-scope. Audit finding F7 covers both trustline and contract token balances; 0119 handled trustlines, this handles Soroban tokens.'
  - date: '2026-05-04'
    status: backlog
    who: stkrolikiewicz
    note: >
      Unblocked: 0120 (Soroban-native token detection) archived 2026-04-24
      via PR #113. `blocked_by` cleared. Recommended ordering before pickup:
      run 0156 first (populates Symbol("decimals") so balance values can be
      scaled correctly during contract_data extraction).
  - date: '2026-05-05'
    status: archive
    who: stkrolikiewicz
    note: >
      Closed as scope-out per current technical design overview.

      Three independent sources confirm classic-only account balance scope:
      (a) tech-design-general-overview.md §1.3 Account page: "Balances —
      native XLM balance and credit asset balances"; (b) §4.2 LedgerCloseMeta
      entry-type map routes "Account balance changes" to LedgerEntryChanges
      (ACCOUNT type) only — CONTRACT_DATA entries are not balance source;
      (c) database-schema-overview.md §6.3 Accounts: "Balances are not
      persisted on this row; see account_balances_current". ADR 0035
      dropped account_balance_history; the residual `account_balances_current`
      table schema (§4.17) carries only typed classic columns
      (asset_type 0/1, asset_code, issuer_id) with no slot for contract_id.

      Per ADR 0023 narrowing (codified in ADR 0037): typed columns preferred
      over JSONB for closed domains. The 0138 plan (write to a presumed
      `accounts.balances` JSONB array) was based on a now-stale assumption —
      the JSONB column never landed; 0119 trustline extraction was
      re-targeted to `account_balances_current` during implementation.

      Audit finding F7 was data-extraction completeness, not a user-facing
      feature gap. Tech design intentionally excludes Soroban ContractData
      Balance entries from per-account state. To re-open this scope, an
      explicit ADR amendment to ADR 0035 plus tech-design + frontend +
      SCF wording updates would be required first; spawn a fresh task
      with proper architectural alignment if that ever becomes priority.
---

# Indexer: extract contract token balances from contract_data entries

## Summary

Task 0119 added trustline balance extraction for classic Stellar assets (credit_alphanum4/12).
Soroban token balances live in `contract_data` ledger entries, not trustlines, and require
per-contract storage layout parsing to extract. This task completes audit finding F7 by
adding contract token balances to the account `balances` JSONB array.

## Context

Soroban tokens (SEP-0041 compliant) store balances as `ContractData` entries keyed by
`(contract_id, Balance, address)`. Unlike trustlines which have a fixed XDR schema, contract
storage layouts vary — task 0120 (soroban-native token detection) must land first to identify
which contracts are tokens and how to parse their storage.

Once 0120 provides token contract detection, this task can extract balance values from
`contract_data` changes and merge them into the account's `balances` array alongside native
XLM and trustline balances.

## Implementation

1. Depend on task 0120's token contract registry to identify which `contract_data` entries
   represent token balances.
2. Parse balance values from `contract_data` entries (key structure: `Balance` + account address).
3. Associate extracted balances with the parent account.
4. Merge into the account's `balances` JSONB array using the existing JSONB merge SQL from 0119.
5. Handle balance creation, update, and removal (contract_data deletion).
6. Format: `{"asset_type": "contract", "contract_id": "C...", "balance": "X.XXXXXXX"}`.
7. Decide on decimal precision: Soroban tokens define their own `decimals()` (could be 6, 7, 8,
   18, etc.) — unlike native XLM which is always 7. Store raw i128 as string? Or normalize
   using the token's declared decimals? Needs design decision.

## Acceptance Criteria

- [ ] `balances` JSONB contains contract token balances alongside native + trustline balances — N/A: `accounts.balances` JSONB column does not exist; classic balances live in `account_balances_current` typed table per ADR 0035 + ADR 0037
- [ ] Contract balance format: `{"asset_type": "contract", "contract_id": "C...", "balance": "X.XXXXXXX"}` — N/A: schema scope-out
- [ ] Balance removal on contract_data deletion — N/A: schema scope-out
- [ ] Watermark prevents stale contract data from overwriting newer state — N/A: schema scope-out
- [ ] Tests: account with native + trustline + contract token produces correct balances array — N/A: scope-out

## Implementation Notes

No code or schema work performed under this task ID. Triage on 2026-05-05 surfaced three architectural misalignments that block this task as written:

1. **Schema mismatch:** `accounts.balances` JSONB array does not exist. The `accounts` row carries only identity + sequence + home_domain (database-schema-overview.md §4.11). Classic balances live in the dedicated `account_balances_current` table (§4.17), per ADR 0026 surrogate-FK plan and ADR 0035 (which dropped the parallel `account_balance_history` partitioned companion). The 0138 plan and the parent 0119 task both used "balances JSONB" wording; the JSONB approach was abandoned during 0119 implementation in favour of typed columns, but the wording was never revised.

2. **Architectural mismatch (ADR 0023 narrowing):** typed columns are preferred over JSONB for closed domains. Account balances are closed-shape `(asset_type, identifier, amount)`. Adding a JSONB array now would re-open a question ADR 0023 + ADR 0037 had already settled.

3. **Tech design scope mismatch:** `technical-design-general-overview.md` consistently restricts account-balance scope to classic Stellar balances:
   - §1.3 Account page: "Balances — **native XLM balance and credit asset balances**"
   - §2.3 endpoint inventory: `GET /accounts/:account_id` returns "current balances" — schema-bound to `account_balances_current` (classic)
   - §4.2 LedgerCloseMeta entry-type map routes "Account balance changes" to **`LedgerEntryChanges` (ACCOUNT type)** only; **CONTRACT_DATA** is not mapped to balance state
   - §6.3 schema reference: "**Balances are not persisted on this row**; see `account_balances_current`" — and that table has no `contract_id` column nor any `asset_type` value reserved for contract balances

Audit finding F7 (pipeline-data-audit, 2026-04-10) flagged "contract token balances not extracted" as a data-extraction gap. That observation is correct factually — `ContractData` entries with `Balance(address)` keys exist in XDR — but tech design intentionally excludes this data from per-account state. F7 is therefore a known-and-accepted divergence, not a missing feature.

## Design Decisions

### From Plan

1. **Closed as scope-out rather than re-scoped to schema migration**: making 0138 implementable would require:

   - schema migration (`account_balances_current` ADD COLUMN `contract_id` + relax CHECK + new unique index `uidx_abc_contract`)
   - ADR amendment overriding ADR 0035's narrowing
   - tech-design overview update (§1.3, §2.3, §4.2, §5.x, §6.3)
   - frontend overview update (account detail balances section)
   - SCF wording update (account.balances scope)
   - indexer extension (parse `Balance(address)` entries from ContractData changes)
   - endpoint query update (`09_get_accounts_by_id.sql` projection)

   Reusing this task ID for that effort would obscure the architectural decision behind a routine "implement F7" task. A fresh task with explicit architecture-first phasing is the cleaner path if scope is ever re-opened.

### Emerged

2. **Discovered `accounts.balances` JSONB never landed**: 0138 (and earlier 0119 task body) referenced an `accounts.balances` JSONB column. Schema audit on 2026-05-05 confirmed the column does not exist and has not existed since at least ADR 0035 (2026-04-XX). 0119 implementation pivoted to `account_balances_current` typed table during work; task body wording was never updated. This is a documentation drift that misled subsequent task design.

3. **Tech-design overview line 845 wording drift**: §4.x ingestion bullet listed `account_balances_current` alongside `assets`, `nfts`, `nft_ownership` as targets of "SEP-41 / NFT transfer pattern derived-state upserts". This wording is inconsistent with the schema (CHECK constraint rejects contract balances) and with §4.2 entry-type mapping. Corrected in the same commit that archives this task.

4. **Knock-on simplification of 0156 Future Work**: with 0138 scope-out, the open question about `decimals` extraction loses its consumer. 0156's Future Work bullet about spawning a `decimals` follow-up if 0138 needs it is no longer relevant. Removed in the same commit.

## Issues Encountered

- **Two-week recognition lag**: the `accounts.balances` JSONB / `account_balances_current` typed-table divergence was carried in 0119 task wording → 0138 task wording → triage. Preventive guidance for future sessions: when reviewing a task that references a JSONB column, grep `crates/db/migrations/*.sql` for the column name before assuming the spec is current.

- **Audit-finding-as-task anti-pattern**: F7 was an audit observation of data-extraction completeness. Spawning a task for every audit finding without first checking whether tech design includes the finding's scope creates pressure to expand the architecture. Future guideline: audit findings → explicit "in scope" / "out of scope" verdict before task spawn, not after.

## Future Work

None spawned. If a future product decision adds Soroban token holdings to account detail (e.g. portfolio view post-launch), the work begins with an ADR amendment to ADR 0035 and a tech-design overview revision. Implementation is downstream of that architectural decision; do not re-use this task ID — start fresh with the architecture-first phasing.
