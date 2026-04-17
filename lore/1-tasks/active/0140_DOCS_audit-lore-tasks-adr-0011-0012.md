---
id: '0140'
title: 'Audit & update lore tasks per ADR 0011/0012 (S3 offload + zero-upsert schema)'
type: DOCS
status: active
related_adr: ['0011', '0012']
related_tasks:
  # Umbrella follow-ups spawned by this audit
  - '0141'
  - '0142'
  # Supersede → archive as status: superseded
  - '0131'
  # Banner + blocked_by 0142 (schema-dependent)
  - '0045'
  - '0046'
  - '0047'
  - '0048'
  - '0049'
  - '0050'
  - '0051'
  - '0052'
  - '0053'
  - '0116'
  - '0121'
  - '0122'
  - '0124'
  - '0125'
  - '0126'
  - '0130'
  - '0132'
  - '0133'
  - '0135'
  - '0136'
  - '0138'
  # Banner only (not hard-blocked by migration)
  - '0073'
  - '0077'
  - '0118'
  - '0120'
  - '0123'
  - '0139'
  # Link only (parser-neutral)
  - '0134'
  # Archive — frontmatter flag (implementation superseded, body retained historical)
  - '0010'
  - '0011'
  - '0012'
  - '0016'
  - '0017'
  - '0018'
  - '0019'
  - '0020'
  - '0022'
  - '0024'
  - '0025'
  - '0026'
  - '0028'
  - '0029'
  - '0030'
  - '0102'
  - '0104'
  - '0117'
  - '0119'
  # Research archive dirs (notes describe pre-ADR-0012 flow)
  - '0002'
  - '0007'
  - '0008'
tags: [layer-docs, priority-high, effort-medium]
milestone: 1
links:
  - lore/2-adrs/0011_s3-offload-lightweight-db-schema.md
  - lore/2-adrs/0012_zero-upsert-schema-full-fk-graph.md
  - lore/3-wiki/project/architecture-snapshot-rust-backend.md
  - lore/3-wiki/partition-pruning-runbook.md
history:
  - date: '2026-04-17'
    status: active
    who: stkrolikiewicz
    note: >
      Task created — reconcile active/blocked/backlog/archive tasks and wiki docs
      with new ADR 0011 (superseded) and 0012 (proposed). Three-tier defense to
      prevent /lore-framework from loading OLD schema/flow patterns into future
      context: (1) archive flag-only, (2) active/backlog banner + optional
      blocked_by 0142, (3) supersede-move for truly redundant tasks.
---

# Audit & update lore tasks per ADR 0011/0012

## Summary

ADR 0011 (S3 offload) is superseded by ADR 0012 (zero-upsert schema with full FK graph,
activity projections, complete index strategy). The schema, ingestion flow, and service
boundaries change significantly. Existing lore tasks and wiki docs are reconciled so
future `/lore-framework` context loads do not pollute planning sessions with pre-ADR
patterns (JSONB balances, upserts on mutable state, `transaction_id` partitioning,
DB-resident XDR blobs, etc.).

Only ADR 0012 should be linked going forward; ADR 0011 remains in the repo as the
superseded predecessor.

## Status: Active

**Current state:** Audit complete. Execution in progress on branch
`docs/0140_audit-lore-tasks-adr-0011-0012`.

## Context

Schema/flow changes from ADR 0012 that drive the audit:

- Zero-upsert — mutable state moves to insert-only history tables
  (`account_balances`, `account_home_domain_changes`, `token_supply_snapshots`,
  `nft_ownership`, `liquidity_pool_snapshots`).
- Activity projections — `account_activity`, `token_activity`,
  `nft_current_ownership`, `token_current_supply`, `liquidity_pool_current`,
  `contract_stats_daily`, `search_index`.
- Full FK graph with `ON DELETE RESTRICT`; `ledgers` is a dimension (no incoming FKs).
- `operations` repartitioned by `RANGE(created_at)` (aligned with
  `soroban_events` / `soroban_invocations`).
- S3 offload of heavy parsed JSON: `envelope_xdr`, `result_xdr`, `result_meta_xdr`,
  `operation_tree`, `signatures`, `memo`, `result_code`, `metadata` blobs, NFT
  attribute payload.
- `BIGSERIAL` promotion for `tokens.id`, `nfts.id`.
- Identity-first parallel backfill with COALESCE progressive fill.
- Deferred post-backfill `CREATE INDEX CONCURRENTLY`; BRIN; partial indexes;
  `pg_trgm` + `hll` extensions.

User requirement: `/lore-framework` must not load OLD flow/infra patterns related to
XDR parsing and DB writes into future context — so schema, API, and frontend planning
sessions start from ADR 0012 cleanly.

## Buckets (v10 — final, after tech-design cross-check and deep scan)

### 1. Supersede → archive (1)

| ID     | Why                                                                      |
| ------ | ------------------------------------------------------------------------ |
| `0131` | `operations` repartition by `created_at` locked in ADR 0012 §3 Rationale |

### 2. Active/Blocked/Backlog — banner + `blocked_by: ['0142']` (21)

Schema-dependent tasks (implementation requires new tables / ingestion pattern).
Stays in current directory (backlog or blocked); `0135` moved active → blocked.

Backend modules: `0045`, `0046`, `0047`, `0048`, `0049`, `0050`, `0051`, `0052`, `0053`.
Indexer / write-path: `0122`, `0124`, `0125`, `0135`, `0138`.
DB / migration-folded: `0130`, `0132`, `0133`, `0136`.
Infra: `0116`.
Existing blocked: `0121` (append 0142 to blockers), `0126` (same).

### 3. Active/Blocked/Backlog — banner only, no hard block (6)

Logic schema-adjacent but implementable against pre- or post-migration state.

| ID     | Reason                                                                |
| ------ | --------------------------------------------------------------------- |
| `0118` | NFT false positives — classification logic, schema-agnostic           |
| `0120` | Soroban-native token detection — classification logic                 |
| `0139` | Partition Lambda hotfix — operational fix for current-schema incident |
| `0123` | XDR decoding service — potentially obsoleted by S3 offload            |
| `0073` | Frontend account detail — consumes API response (preserved per spec)  |
| `0077` | Frontend LP list/detail — same reasoning                              |

### 4. Link only (1)

| ID     | Reason                                          |
| ------ | ----------------------------------------------- |
| `0134` | Envelope/meta ordering — parser-only, no schema |

### 5. Archive — frontmatter flag only (22)

Bodies retained as historical record; tag + `related_adr: ['0012']` + history note
mark them as superseded references for future context loads.

Tasks: `0010`, `0011`, `0012`, `0016`, `0017`, `0018`, `0019`, `0020`, `0022`,
`0024`, `0025`, `0026`, `0028`, `0029`, `0030`, `0102`, `0104`, `0117`, `0119`.
Research dirs (README.md): `0002`, `0007`, `0008`.

### 6. Wiki banners (2)

| File                                                   | Reason                                                |
| ------------------------------------------------------ | ----------------------------------------------------- |
| `3-wiki/project/architecture-snapshot-rust-backend.md` | Pre-ADR-0012 DB access + response-source mapping      |
| `3-wiki/partition-pruning-runbook.md`                  | Section "Operations Table (transaction_id range)" OLD |

### 7. ADR metadata updates (3)

| File       | Change                                                                      |
| ---------- | --------------------------------------------------------------------------- |
| `ADR 0011` | Populate `related_tasks` with `0140`                                        |
| `ADR 0012` | Populate `related_tasks` with full affected-task set (~50 entries)          |
| `ADR 0008` | History entry — cursor stability note (BIGSERIAL promotion at 0142 cutover) |

### 8. New umbrella tasks

- **`0141 RESEARCH`** — finalize ADR 0012 (proposed → accepted), resolve open schema
  questions. Blocker for `0142`.
- **`0142 FEATURE`** — schema migration implementing ADR 0012. Blocks 21 downstream
  tasks (bucket 2). Blocked by `0141`.

### 9. Spawned cleanup tasks

- **`0118` directory/.md duplication** — separate backlog task to convert to single
  format per `lore/1-tasks/CLAUDE.md`.

## Acceptance Criteria

- [x] Bucket 1 supersede: `0131` moved to archive with `status: superseded, by: ['0012']`
- [x] Bucket 2 banner + `blocked_by` on 21 tasks (19 files in first batch + 0135 moved + 0121/0126/0132 appended)
- [x] Bucket 3 banner on 6 tasks
- [x] Bucket 4 link-only on 0134
- [x] Bucket 5 archive flag on 22 tasks
- [x] Bucket 6 wiki banners on 2 files
- [x] Bucket 7 ADR metadata populated (0011, 0012, 0008)
- [x] Bucket 8 new tasks 0141/0142 created
- [x] Bucket 9 cleanup task for 0118 format spawned (0144)
- [x] `lore/README.md` regenerated
- [x] Commit split + PR to develop ([#96](https://github.com/rumblefishdev/soroban-block-explorer/pull/96))
- [x] 0143 in-scope decision made — deferred to 0141 (decision: revisit after ADR 0012 accepted)

## Notes

- Audit does not modify application code. Only lore metadata, task reorganization,
  two wiki docs, three ADR frontmatters, and two new umbrella tasks.
- Active task `0139` (partition Lambda) has uncommitted code changes stashed under
  `stash@{0}` pending separate completion on its own branch.
- Archived tasks `0016–0020, 0021, 0022, 0024–0030, 0102, 0104, 0117, 0119` describe
  the code that ships today. Their bodies are not rewritten — new schema implementation
  comes via `0142`, not as amendments to archived work.
- Archived research dirs `0002, 0007, 0008` contain notes that embed pre-ADR-0012
  assumptions (field mappings, partitioning strategy, event-interpreter schemas). Flag
  is on the task README only; individual note files are not edited.
