---
id: '0101'
title: 'Rust domain types: DB entity models (operation, Soroban, token, account, NFT, pool)'
type: FEATURE
status: completed
related_adr: ['0005', '0007']
related_tasks: ['0010', '0011', '0012', '0079', '0094', '0018', '0019', '0098']
tags: [priority-high, effort-medium, layer-domain, rust]
milestone: 1
links: []
history:
  - date: 2026-04-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Created to replace TypeScript domain types (tasks 0010-0012, 0079)
      which are obsolete after ADR 0005 (Rust-only backend).
      crates/domain/ already has Ledger and Transaction; remaining models missing.
  - date: 2026-04-02
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Scope refined through multiple review rounds: pure DB row
      structs only, no enums, no xdr-parser re-exports.
  - date: 2026-04-02
    status: completed
    who: stkrolikiewicz
    note: >
      Implemented all 7 steps. 6 new modules, 9 structs added to crates/domain/.
      Domain stays lightweight (15 deps vs 80 with xdr-parser).
      Key decisions: no enums (pure DDL mirror), no xdr-parser dep,
      PoolAsset excluded (JSONB shape, not DB entity).
---

# Rust domain types: DB entity models

## Summary

Define shared Rust domain structs in `crates/domain/` for all remaining DB entity models. Pure DB row mirrors — every field maps 1:1 to a DDL column with matching type and nullability. No enums, no business logic, no helpers. VARCHAR columns stay `String`, JSONB stays `serde_json::Value`.

Complements the write-path `Extracted*` types in `crates/xdr-parser/` which serve a different purpose (pre-DB, no surrogate IDs, hash-based references, unix timestamps).

Replaces TypeScript domain types from tasks 0010-0012 which became obsolete after ADR 0005 (Rust-only backend).

## Status: Completed

## Context

### Two type layers

```
WRITE: XDR → Extracted* (xdr-parser) → SQL INSERT (db)
READ:  SQL SELECT → domain types (domain) → response DTOs (api)
```

| Concern      | xdr-parser `Extracted*`    | domain                            |
| ------------ | -------------------------- | --------------------------------- |
| IDs          | No surrogate IDs           | `id: i64` (DB-assigned)           |
| FKs          | `transaction_hash: String` | `transaction_id: i64`             |
| Timestamps   | `created_at: i64` (unix)   | `created_at: DateTime<Utc>`       |
| Type columns | `String`                   | `String` (same — pure DDL mirror) |
| Purpose      | Write path (indexer → DB)  | Read path (DB → API)              |

### Type mapping

| DDL type             | Rust type                | Rationale                                           |
| -------------------- | ------------------------ | --------------------------------------------------- |
| BIGINT / BIGSERIAL   | `i64`                    | sqlx maps natively                                  |
| SERIAL               | `i32`                    | 4-byte int, fits in i32                             |
| SMALLINT             | `i16`                    | 2-byte int                                          |
| INTEGER              | `i32`                    | 4-byte int                                          |
| NUMERIC              | `String`                 | Avoids `rust_decimal` dep; API serializes as string |
| VARCHAR / TEXT       | `String`                 | Direct mapping                                      |
| BOOLEAN              | `Option<bool>` or `bool` | `Option` if no NOT NULL constraint                  |
| JSONB                | `serde_json::Value`      | Direct equivalent                                   |
| TIMESTAMPTZ          | `DateTime<Utc>`          | Existing pattern                                    |
| TSVECTOR (generated) | excluded                 | DB-only column                                      |

## Acceptance Criteria

- [x] `Operation` struct — all DDL fields from migration 0002
- [x] `SorobanContract` struct — all DDL fields except `search_vector`
- [x] `SorobanInvocation` struct — all DDL fields
- [x] `SorobanEvent` struct — all DDL fields
- [x] `Token` struct — all DDL fields
- [x] `Account` struct — all DDL fields
- [x] `Nft` struct — all DDL fields
- [x] `LiquidityPool` struct — all DDL fields
- [x] `LiquidityPoolSnapshot` struct — all DDL fields
- [x] All structs derive `Debug, Clone, Serialize, Deserialize`
- [x] Field nullability matches DDL exactly (`Option<T>` ↔ no NOT NULL)
- [x] No enums — all VARCHAR columns as `String`
- [x] No xdr-parser dependency — domain stays lightweight
- [x] Modules registered in `crates/domain/src/lib.rs`
- [x] `cargo build -p domain` passes

## Implementation Notes

**Files created (6):**

- `crates/domain/src/operation.rs` — `Operation` (6 fields)
- `crates/domain/src/soroban.rs` — `SorobanContract` (7 fields), `SorobanInvocation` (10 fields), `SorobanEvent` (8 fields)
- `crates/domain/src/token.rs` — `Token` (9 fields)
- `crates/domain/src/account.rs` — `Account` (6 fields)
- `crates/domain/src/nft.rs` — `Nft` (9 fields)
- `crates/domain/src/pool.rs` — `LiquidityPool` (9 fields), `LiquidityPoolSnapshot` (9 fields)

**Files modified (2):**

- `crates/domain/Cargo.toml` — added `serde_json` dependency
- `crates/domain/src/lib.rs` — registered 6 new modules

**Total:** 9 structs, 73 fields, 332 lines added. Domain dep tree: 15 crates.

## Design Decisions

### From Plan

1. **Pure DDL mirror — no business logic in domain:** Every field maps 1:1 to a DDL column. Domain types are "DB row structs", not rich domain objects. Enums, helpers, and validation belong in the API layer.

2. **`String` for NUMERIC columns:** Avoids `rust_decimal` dependency. API serializes financial values as strings anyway. Consumers can parse to `Decimal` if needed.

3. **`serde_json::Value` for JSONB columns:** Direct equivalent. Typed deserialization (e.g. PoolAsset, ContractFunction) belongs in the consuming layer.

### Emerged

4. **No enums in domain types:** Initial plan included `OperationType`, `ContractType`, `EventType`, `AssetType` enums. Removed after review because: (a) `OperationType::Other` with `#[serde(other)]` loses the original string — unacceptable for a block explorer, (b) enums on struct fields create inconsistency (some fields use enum, others String), (c) DDL says `VARCHAR` → Rust should say `String`. Enums are a business logic concern for the API layer.

5. **No xdr-parser dependency:** Initial plan re-exported `ContractFunction`/`FunctionParam` from xdr-parser. Removed because it pulled 80 transitive dependencies (stellar-xdr, sha2, zstd, etc.) into a "lightweight domain types" crate. Consumers that need `ContractFunction` can depend on xdr-parser directly.

6. **`is_sac: Option<bool>` instead of `bool`:** DDL has `BOOLEAN DEFAULT FALSE` without NOT NULL constraint. Strict DDL alignment requires `Option<bool>`. Practically null won't occur (indexer always sets it), but the domain type must reflect what the DB allows.

7. **PoolAsset excluded from domain:** DDL defines `asset_a JSONB` and `asset_b JSONB` — raw JSONB without schema. `PoolAsset` would type the JSONB content, which is a deserialization concern for the API layer, not a DB row concern.

8. **Nft without surrogate `id`:** Migration 0006 uses `PRIMARY KEY (contract_id, token_id)` composite key — no `id BIGSERIAL`. Domain struct matches this (no `id` field), unlike task 0020 spec which had `id BIGSERIAL PRIMARY KEY`.

## Issues Encountered

- **Task 0020 DDL spec diverges from actual migration 0006:** Multiple differences (nfts PK structure, column sizes, nullability, fee_bps NOT NULL). Domain types are based on the real migration, not the stale task spec. Task 0020 needs separate cleanup.

## Future Work

- Rewrite migrations 0002-0006 to derive from domain types (types-first workflow). Separate task — DB is empty, no risk.
- Add `sqlx::FromRow` derive when `crates/db/` query functions are implemented
- Define enums (OperationType, ContractType, etc.) in `crates/api/` when API modules need pattern matching
- Define API view types (Pointer/Summary/Detail) and request/response types in `crates/api/`
- Close/update stale task 0020

## Out of Scope

- **Enums** (OperationType, ContractType, EventType, AssetType) — business logic, belong in API layer
- **ContractFunction / FunctionParam** — defined in `crates/xdr-parser/`, consumers depend directly
- **PoolAsset** — typed JSONB shape, not a DB entity; deserialize in API layer
- **API view types** (Pointer/Summary/Detail) — response DTOs in `crates/api/`
- **API request/response types** (pagination, search, network stats, chart) — not DB entities
- **EventInterpretation** — removed from architecture (task 0098, ADR 0007)
