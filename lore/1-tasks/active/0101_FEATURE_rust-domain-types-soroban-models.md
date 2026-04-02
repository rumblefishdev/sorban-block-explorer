---
id: '0101'
title: 'Rust domain types: Soroban models (contract, invocation, event, token, account, NFT)'
type: FEATURE
status: active
related_adr: ['0005']
related_tasks: ['0010', '0011', '0012', '0079', '0094', '0018']
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
      crates/domain/ already has Ledger and Transaction; Soroban models,
      Token, Account, NFT are missing.
  - date: 2026-04-02
    status: active
    who: stkrolikiewicz
    note: 'Activated — ready to implement Rust domain types'
---

# Rust domain types: Soroban models (contract, invocation, event, token, account, NFT)

## Summary

Define shared Rust domain structs in `crates/domain/` for Soroban contracts, invocations, events, event interpretations, tokens, accounts, and NFTs. These replace the TypeScript types from tasks 0010-0012 which became obsolete after ADR 0005 (Rust-only backend). The structs are consumed by both `crates/api/` and `crates/indexer/` and must align with the PostgreSQL DDL from task 0018.

## Status: Active

**Current state:** Not started. `crates/domain/` already has `Ledger` and `Transaction` structs as reference for patterns and conventions.

## Context

After ADR 0005, the entire backend (API + Indexer) is Rust. TypeScript domain types in `libs/domain/` (tasks 0010-0012, split in 0079) are no longer consumed by any backend code. The Rust `crates/domain/` crate needs equivalent structs.

### Existing patterns (from `crates/domain/`)

- `Ledger` and `Transaction` structs already exist with `#[derive(Debug, Clone, Serialize, Deserialize)]`
- `chrono::DateTime<Utc>` for timestamps
- `i64` for BIGINT/BIGSERIAL columns
- `Option<T>` for nullable columns
- `String` for VARCHAR columns

### Types to define

Sourced from the TypeScript types in tasks 0010-0012 and DDL in task 0018:

| Module       | Structs/Enums                                                                                                                                                            | Source task |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------- |
| `soroban.rs` | `ContractType`, `ContractFunction`, `ContractMetadata`, `SorobanContract`, `EventType`, `SorobanInvocation`, `SorobanEvent`, `InterpretationType`, `EventInterpretation` | 0010        |
| `token.rs`   | `AssetType`, `Token`                                                                                                                                                     | 0011        |
| `account.rs` | `Account`                                                                                                                                                                | 0011        |
| `nft.rs`     | `Nft`                                                                                                                                                                    | 0011        |

### Key type mapping decisions

| TypeScript                       | Rust                                              | Rationale                                  |
| -------------------------------- | ------------------------------------------------- | ------------------------------------------ |
| `BigIntString` (string alias)    | `i64`                                             | sqlx maps BIGINT to i64 natively           |
| `NumericString` (string alias)   | `rust_decimal::Decimal` or `String`               | NUMERIC(28,7) — evaluate sqlx support      |
| `JsonValue`                      | `serde_json::Value`                               | Direct equivalent                          |
| `ScVal` (JsonValue alias)        | `serde_json::Value`                               | Placeholder until xdr-parser crate matures |
| `readonly` arrays                | `Vec<T>`                                          | Rust ownership handles immutability        |
| Union types (`'token' \| 'dex'`) | `enum` with `#[serde(rename_all = "snake_case")]` | Idiomatic Rust                             |

## Implementation Plan

### Step 1: Add dependencies to `crates/domain/Cargo.toml`

Add `serde_json` (for JSONB fields) and potentially `rust_decimal` (for NUMERIC columns). Check if `sqlx::FromRow` derive should be in domain or kept at db layer.

### Step 2: Define Soroban enums and helper types

Create `soroban.rs` with `ContractType`, `EventType`, `InterpretationType` enums and `ContractFunction`, `ContractMetadata` structs.

### Step 3: Define Soroban entity structs

In `soroban.rs`: `SorobanContract`, `SorobanInvocation`, `SorobanEvent`, `EventInterpretation`. Align field names and nullability with DDL from task 0018.

### Step 4: Define Token, Account, NFT

Create `token.rs` (`AssetType` enum, `Token` struct), `account.rs` (`Account` struct), `nft.rs` (`Nft` struct). Align with DDL.

### Step 5: Register modules in `lib.rs`

Add `pub mod soroban;`, `pub mod token;`, `pub mod account;`, `pub mod nft;` to `crates/domain/src/lib.rs`.

### Step 6: Verify compilation

Run `cargo build -p domain` and ensure all structs compile and derive macros work.

## Acceptance Criteria

- [ ] `ContractType`, `EventType`, `InterpretationType`, `AssetType` enums defined with serde support
- [ ] `ContractFunction`, `ContractMetadata` helper types defined
- [ ] `SorobanContract` struct — all DDL fields except `search_vector`
- [ ] `SorobanInvocation` struct — all DDL fields, JSONB as `serde_json::Value`
- [ ] `SorobanEvent` struct — all DDL fields
- [ ] `EventInterpretation` struct — all DDL fields
- [ ] `Token` struct — all DDL fields, NUMERIC mapped appropriately
- [ ] `Account` struct — all DDL fields, balances as `Vec<serde_json::Value>`
- [ ] `Nft` struct — all DDL fields
- [ ] All structs derive `Debug, Clone, Serialize, Deserialize`
- [ ] Field nullability matches DDL (Option<T> for nullable)
- [ ] Modules registered in `crates/domain/src/lib.rs`
- [ ] `cargo build -p domain` passes

## Notes

- **Depends on 0094** (scaffold Cargo workspace) if `crates/domain/Cargo.toml` doesn't yet support additional dependencies.
- **Does NOT depend on 0018** — types can be defined before SQL migrations exist, but should align with the DDL spec in 0018.
- `search_vector` (TSVECTOR generated column) is DB-only — excluded from domain struct, same as in TypeScript.
- Consider whether `sqlx::FromRow` derive belongs in domain crate or a separate db-layer mapping. Existing `Ledger`/`Transaction` structs don't use it — follow the same pattern for now.
- TypeScript types in `libs/domain/` will be cleaned up when task 0095 (monorepo restructure) removes `apps/api` and `apps/indexer`. The `libs/domain/` package may still be needed by `web/` frontend for API response types — but that's covered by task 0096 (OpenAPI TypeScript codegen).
