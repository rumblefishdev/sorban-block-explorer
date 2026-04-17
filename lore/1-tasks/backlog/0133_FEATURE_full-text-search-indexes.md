---
id: '0133'
title: 'DB: add full-text search indexes for tokens, accounts, NFTs'
type: FEATURE
status: backlog
related_adr: ['0012']
related_tasks: ['0053', '0132', '0140', '0142']
blocked_by: ['0142']
tags:
  [
    priority-medium,
    effort-medium,
    layer-db,
    audit-F22,
    pending-adr-0012-rewrite,
  ]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F22 (MEDIUM). Global search (task 0053) depends on this.'
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

---

# DB: add full-text search indexes for tokens, accounts, NFTs

## Summary

Full-text search only covers `soroban_contracts` (via `search_vector` GIN index on
`metadata->>'name'`). The planned global search feature (task 0053) needs search across
tokens, accounts, and NFTs — none of which have search_vector columns or GIN indexes.

Additionally, `soroban_contracts.search_vector` is generated from `metadata->>'name'`, but
most contracts have `metadata = NULL` (populated only via WASM interface staging), making
it useless for the majority of contracts.

## Implementation

New migration:

1. **tokens**: Use `pg_trgm` GIN index on `name` and `asset_code` for ILIKE/similarity
   search. TSVECTOR is not appropriate here — `asset_code` is max 12 chars and `name` is
   typically short; FTS tokenization/stemming actively hurts short-string search (e.g.,
   stop-word elimination removes common codes). Add B-tree index on `asset_code`.
2. **accounts**: Add index on `home_domain` for domain-based lookup.
3. **nfts**: Add `search_vector` TSVECTOR GENERATED from `name` + `collection_name`. Add
   GIN index.
4. **soroban_contracts**: Extend `search_vector` generation to include `contract_id` prefix
   matching.

## Acceptance Criteria

- [ ] Tokens searchable by name and asset_code via `pg_trgm` ILIKE/similarity search
- [ ] NFTs searchable by name and collection_name via full-text search
- [ ] Accounts findable by home_domain
- [ ] Global search (task 0053) has index support for all entity types
