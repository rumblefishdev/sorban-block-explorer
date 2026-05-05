---
id: '0041'
title: 'Replace soroban_contracts.metadata JSONB with typed name VARCHAR column'
status: proposed
deciders: [stkrolikiewicz]
related_tasks: ['0156']
related_adrs: ['0023', '0037']
tags: [schema, soroban, narrowing, typed-columns]
links: []
history:
  - date: 2026-05-05
    status: proposed
    who: stkrolikiewicz
    note: 'ADR created alongside task 0156 implementation.'
---

# ADR 0041: Replace `soroban_contracts.metadata` JSONB with typed `name` VARCHAR column

**Related:**

- [Task 0156: Indexer: extract Soroban token name from ContractData (typed `name` column)](../1-tasks/active/0156_FEATURE_soroban-contract-name-extraction.md)
- [ADR 0023: Tokens typed metadata columns](./0023_tokens-typed-metadata-columns.md)
- [ADR 0037: Current schema snapshot](./0037_current-schema-snapshot.md)

---

## Context

`soroban_contracts.metadata JSONB` was added during the early schema design as a forward-looking container for an unspecified set of contract-level metadata fields (eventually expected to include `name`, `symbol`, `decimals`, and possibly more). In practice, only one field has ever needed to live there — `Symbol("name")` extracted from the contract's persistent `ContractData` storage at deploy time — and even that field was never written: the parser placeholder at `crates/xdr-parser/src/state.rs:69` writes `metadata = json!({})` for every deployment.

`soroban_contracts.search_vector` is `TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(metadata->>'name', '') || ' ' || contract_id)) STORED` (migration `0002:58-60`). The full-text search index therefore reads `metadata->>'name'`, but because the field is never populated, FTS returns the contract id only — the human-readable name half of the search vector is dead.

Task 0156 closes the extraction gap. As the first real consumer surfaces, we have to choose persistence shape:

1. Keep `metadata` JSONB and write `{"name": ...}` into it, OR
2. Replace `metadata` JSONB with a typed `name VARCHAR(256)` column.

ADR 0023 already articulated the principle for the analogous `tokens` (now `assets`) table: typed columns are preferred over JSONB for closed-shape data; JSONB is reserved for genuinely open metadata shapes. ADR 0037 codified that narrowing into the current schema snapshot. The same principle applies one-to-one to `soroban_contracts.metadata`.

---

## Decision

Replace `soroban_contracts.metadata JSONB` with a typed `soroban_contracts.name VARCHAR(256)` column.

Concretely:

- Drop the `metadata` JSONB column.
- Drop the dependent generated `search_vector` column and its `idx_contracts_search` GIN index.
- Add `name VARCHAR(256)` (nullable — most contracts will never store an on-chain name).
- Recreate `search_vector` as `TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)) STORED`, plus its GIN index.
- Update every consumer (search query, contract detail endpoint, schema docs) to read the typed column.

The migration is shipped together with task 0156's extraction code in a single PR so the column never exists in a state where it has a write path but no extraction logic, or vice versa.

---

## Rationale

- **ADR 0023 narrowing principle.** Closed-shape data → typed columns; JSONB → genuinely open metadata. `name` is a single string with predictable semantics. There is no scenario where a contract metadata "name" entry would be a non-string, a list, or a nested object. The JSONB envelope adds no expressive power here, only overhead.

- **Storage and read-path overhead.** A single-field JSONB pays a per-row header cost (the `{"name":"X"}` envelope is ~22 B vs ~10 B for the same string in a VARCHAR column) and a per-read parse cost on every `metadata->>'name'` lookup. At scale (~10M contracts plausible long-term) this is ~120 MB of pure overhead. Search FTS recomputes the generated `search_vector` on every UPDATE, so the parse cost is paid both on write and on planner re-evaluation. The typed column eliminates both.

- **Type-safety.** `VARCHAR(256)` is a database-checkable constraint. JSONB allows any shape, so a buggy writer could insert `{"name": [1,2,3]}` or `{"name": null}` and the schema would accept it; the search index and the API consumers would both silently misbehave.

- **Extensibility discipline.** If a future need arises for `decimals`, `symbol`, or some other contract-level metadata field, the path is `ADD COLUMN <field> <typed>` — which forces an explicit schema change with a code review covering every consumer. Re-introducing a catch-all JSONB would let fields silently accrete without that review gate.

- **Audit alignment.** The current rows all have `metadata = '{}'` or `NULL`. The migration's data move is an effective no-op (`UPDATE … SET name = metadata->>'name' WHERE metadata ? 'name'` rewrites zero rows in production state). The change is therefore safe to ship without a long migration window.

---

## Alternatives Considered

### Alternative 1: Keep `metadata` JSONB and start writing `{"name": ...}` into it

**Description:** Task 0156 only adds the extraction logic; the schema stays as-is.

**Pros:**

- No schema migration; smaller diff.
- Future fields (`symbol`, `decimals`, etc.) could be added without further migrations.

**Cons:**

- Carries the per-row overhead and per-read parse cost forever for one closed-shape field.
- Diverges from ADR 0023 narrowing without a stated reason.
- Lets future fields accrete inside the JSONB without code review (the failure mode that ADR 0023 was written to prevent).
- Inconsistent with the analogous `assets.name` shape (typed VARCHAR), creating an unjustified split in convention.

**Decision:** REJECTED — the schema bloat is permanent; the migration cost is one-shot and trivial because the column is empty in current state.

### Alternative 2: Re-purpose `metadata` JSONB for richer future fields, add a dedicated `name VARCHAR` alongside it

**Description:** Keep the JSONB as a forward-compatible container while also storing `name` as a typed column for fast access.

**Pros:**

- Has both fast read path and forward extensibility.

**Cons:**

- Double-writes the same value to two places (data drift risk over the lifetime of the table).
- Pays JSONB overhead permanently with no concrete consumer for the open shape.
- "Forward-compatibility" is speculative — YAGNI applies.

**Decision:** REJECTED — speculative dual-write for a hypothetical future field is exactly the wrong trade-off. The cost of adding one more typed column later (when an actual consumer appears) is one ALTER TABLE; the cost of carrying a redundant JSONB column is paid forever.

### Alternative 3: Move `name` into `assets.name` only and drop the column on `soroban_contracts` entirely

**Description:** `assets.name` already exists for Fungible tokens. NFT and generic-contract names could live elsewhere or be omitted.

**Pros:**

- Single source of truth for token names.

**Cons:**

- Generic Soroban contracts (non-token dApps, libraries, routers) do not have an `assets` row — they need a name slot somewhere on `soroban_contracts`.
- NFT contracts (per ADR 0037) do not have an `assets` row either.
- The search vector lives on `soroban_contracts`; putting the name on `assets` only would require a JOIN inside a GENERATED column, which Postgres does not allow.

**Decision:** REJECTED — `soroban_contracts` is the only table that covers every contract type and is the only correct home for a contract-level name field.

---

## Consequences

### Positive

- Search FTS reads a typed column directly. Read path is `sc.name` instead of `sc.metadata->>'name'`; one less parse step per query, simpler EXPLAIN plans.
- Storage saving ~12 B per row (eliminated JSONB header + key overhead). At ~10M contracts: ~120 MB of pure DB size reduction.
- ADR 0023 narrowing principle is now applied uniformly to both the asset registry (`assets.name`) and the contract registry (`soroban_contracts.name`). One mental model for both.
- Future contract-level fields (`decimals` if 0138 ever revisits, `symbol` if a real consumer surfaces) follow the same `ADD COLUMN` pattern, with each addition forced through a code review of every consumer.

### Negative

- Wire-shape change for `GET /v1/contracts/:contract_id` (E11): the `metadata` field is dropped. Frontend must handle the absent field gracefully. This is a documented breaking change in this ADR's delivery PR; current behavior was returning `{}` always (effectively meaningless), so user-visible impact is nil.
- Anyone holding a stale view of the schema (e.g. a manually-written ad-hoc query against `metadata->>'name'`) will need to update once the migration lands. Sweep done as part of task 0156: 8 references found, all updated.
- A future requirement to store a complex shape on `soroban_contracts` (e.g. arbitrary user-provided per-contract annotations) would now require either a new typed column for the new shape or a deliberate, ADR-tracked re-introduction of a JSONB. This is the desired friction.

---

## Delivery Checklist

Per [ADR 0032](./0032_docs-architecture-evergreen-maintenance.md):

- [ ] `docs/architecture/technical-design-general-overview.md` — updated (search_vector definition reference)
- [ ] `docs/architecture/database-schema/database-schema-overview.md` — updated (`soroban_contracts` table schema; `metadata` removed, `name` added; search_vector definition refreshed)
- [ ] `docs/architecture/backend/backend-overview.md` — updated (E11 contract detail response shape; `metadata` field removed)
- [x] `docs/architecture/frontend/frontend-overview.md` — N/A — frontend already treats absent `metadata` field as "no metadata"; no copy change needed
- [x] `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` — N/A — indexer write path is described at the level of "stages and tables", not the per-column shape
- [x] `docs/architecture/infrastructure/infrastructure-overview.md` — N/A — pure schema change, no infra impact
- [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` — updated (mention of new `Symbol("name")` extraction path during contract deployment parsing)
- [ ] This ADR linked from each updated doc at the relevant section

---

## References

- [ADR 0023: Tokens typed metadata columns](./0023_tokens-typed-metadata-columns.md) — original statement of the typed-columns-vs-JSONB principle for the `tokens` (now `assets`) table.
- [ADR 0037: Current schema snapshot](./0037_current-schema-snapshot.md) — codifies the narrowing in the live schema.
- [ADR 0032: Docs architecture evergreen maintenance](./0032_docs-architecture-evergreen-maintenance.md) — required docs updates per shape-of-system change.
- [Task 0156: Soroban token name extraction](../1-tasks/active/0156_FEATURE_soroban-contract-name-extraction.md) — implementation thread bundling extraction logic with this schema migration.
