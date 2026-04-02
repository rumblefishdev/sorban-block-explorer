---
id: '0101'
title: 'Rust domain types: DB entity models (operation, Soroban, token, account, NFT, pool)'
type: FEATURE
status: active
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
      Activated. Scope refined: pure DB row structs only, no enums, no re-exports.
      EventInterpretation excluded (removed from architecture per 0098/ADR 0007).
      API types (search, pagination, network stats, chart, view variants) excluded.
      ContractFunction/FunctionParam stay in xdr-parser (consumers depend directly).
---

# Rust domain types: DB entity models

## Summary

Define shared Rust domain structs in `crates/domain/` for all remaining DB entity models. Pure DB row mirrors — every field maps 1:1 to a DDL column with matching type and nullability. No enums, no business logic, no helpers. VARCHAR columns stay `String`, JSONB stays `serde_json::Value`.

Complements the write-path `Extracted*` types in `crates/xdr-parser/` which serve a different purpose (pre-DB, no surrogate IDs, hash-based references, unix timestamps).

Replaces TypeScript domain types from tasks 0010-0012 which became obsolete after ADR 0005 (Rust-only backend).

## Status: Active

**Current state:** Not started. `crates/domain/` already has `Ledger` and `Transaction` structs.

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

### Types to define

| Module         | Structs                                                | DDL source            |
| -------------- | ------------------------------------------------------ | --------------------- |
| `operation.rs` | `Operation`                                            | migration 0002        |
| `soroban.rs`   | `SorobanContract`, `SorobanInvocation`, `SorobanEvent` | migrations 0003, 0004 |
| `token.rs`     | `Token`                                                | migration 0005        |
| `account.rs`   | `Account`                                              | migration 0005        |
| `nft.rs`       | `Nft`                                                  | migration 0006        |
| `pool.rs`      | `LiquidityPool`, `LiquidityPoolSnapshot`               | migration 0006        |

### Type mapping

| DDL type             | Rust type                | Rationale                                           |
| -------------------- | ------------------------ | --------------------------------------------------- |
| BIGINT / BIGSERIAL   | `i64`                    | sqlx maps natively                                  |
| SERIAL               | `i32`                    | 4-byte int, fits in i32                             |
| SMALLINT             | `i16`                    | 2-byte int                                          |
| INTEGER              | `i32`                    | 4-byte int                                          |
| NUMERIC              | `String`                 | Avoids `rust_decimal` dep; API serializes as string |
| VARCHAR              | `String`                 | Direct mapping                                      |
| TEXT                 | `String`                 | Direct mapping                                      |
| BOOLEAN              | `Option<bool>` or `bool` | `Option` if no NOT NULL constraint                  |
| JSONB                | `serde_json::Value`      | Direct equivalent                                   |
| TIMESTAMPTZ          | `DateTime<Utc>`          | Existing pattern                                    |
| TSVECTOR (generated) | excluded                 | DB-only column                                      |

## Implementation Plan

### Step 1: Add `serde_json` dependency to `crates/domain/Cargo.toml`

Only new dependency. Domain stays lightweight (serde + serde_json + chrono).

### Step 2: Define `Operation` (`operation.rs`)

Pure struct, all fields from migration 0002. `op_type: String` (not enum).

### Step 3: Define Soroban structs (`soroban.rs`)

`SorobanContract`, `SorobanInvocation`, `SorobanEvent`. All VARCHAR type columns as `String`. `is_sac: Option<bool>` (DDL has no NOT NULL).

### Step 4: Define `Token`, `Account`, `Nft`

One struct per file. `asset_type: String` (not enum). All nullable fields as `Option<T>`.

### Step 5: Define pool structs (`pool.rs`)

`LiquidityPool`, `LiquidityPoolSnapshot`. NUMERIC fields as `String`. JSONB fields as `serde_json::Value`.

### Step 6: Register modules in `lib.rs`

### Step 7: `cargo build -p domain`

## Acceptance Criteria

- [ ] `Operation` struct — all DDL fields from migration 0002
- [ ] `SorobanContract` struct — all DDL fields except `search_vector`
- [ ] `SorobanInvocation` struct — all DDL fields
- [ ] `SorobanEvent` struct — all DDL fields
- [ ] `Token` struct — all DDL fields
- [ ] `Account` struct — all DDL fields
- [ ] `Nft` struct — all DDL fields
- [ ] `LiquidityPool` struct — all DDL fields
- [ ] `LiquidityPoolSnapshot` struct — all DDL fields
- [ ] All structs derive `Debug, Clone, Serialize, Deserialize`
- [ ] Field nullability matches DDL exactly (`Option<T>` ↔ no NOT NULL)
- [ ] No enums — all VARCHAR columns as `String`
- [ ] No xdr-parser dependency — domain stays lightweight
- [ ] Modules registered in `crates/domain/src/lib.rs`
- [ ] `cargo build -p domain` passes

## Out of Scope

- **Enums** (OperationType, ContractType, EventType, AssetType) — business logic, belong in API layer when pattern matching is needed
- **ContractFunction / FunctionParam** — defined in `crates/xdr-parser/`, consumers depend directly
- **PoolAsset** — typed JSONB shape, not a DB entity; deserialize in API layer
- **API view types** (Pointer/Summary/Detail) — response DTOs in `crates/api/`
- **API request/response types** (pagination, search, network stats, chart) — not DB entities
- **EventInterpretation** — removed from architecture (task 0098, ADR 0007)

## Notes

- Domain types should be created **alongside DDL** — they are the Rust expression of the same DB contract. The gap exists because the TS→Rust migration (ADR 0005) happened after TS types were already created.
- `search_vector` (TSVECTOR generated column) is DB-only — excluded from domain struct.
- `sqlx::FromRow` derive: not added now (existing `Ledger`/`Transaction` don't use it). Can be added when `crates/db/` query functions are implemented.
