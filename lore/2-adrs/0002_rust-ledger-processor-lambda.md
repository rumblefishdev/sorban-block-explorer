---
id: '0002'
title: 'Use Rust for the Ledger Processor Lambda'
status: proposed
deciders: [stkrolikiewicz, fmazur]
related_tasks: ['0002', '0060', '0061', '0062', '0063', '0064']
related_adrs: []
tags: [language-choice, ledger-processor, rust]
links:
  - https://github.com/stellar/rs-stellar-xdr
  - https://github.com/stellar/go-stellar-sdk
  - https://github.com/stellar/js-stellar-sdk
history:
  - date: 2026-03-26
    status: proposed
    who: stkrolikiewicz
    note: 'ADR proposed based on task 0002 research findings and stellar-indexer reference'
---

# ADR 0002: Use Rust for the Ledger Processor Lambda

**Related:**

- [Task 0002: LedgerCloseMeta XDR parsing research](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/README.md)
- [S-language-choice-ledger-processor.md](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/notes/S-language-choice-ledger-processor.md)
- [stellar-indexer reference](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/sources/stellar-indexer-ledger-mod-rs.md)

---

## Context

The Ledger Processor Lambda is the core ingestion engine of the block explorer. It receives zstd-compressed LedgerCloseMetaBatch XDR files from S3, parses them, extracts all explorer data fields, and writes structured records to PostgreSQL.

Task 0002 research revealed:

1. **Protocol 25 introduced TransactionMetaV4** which relocates Soroban events from `sorobanMeta.events()` to top-level. The parser must dispatch on meta version (V3 vs V4).
2. **TX set has two phases** — classic (V0) and parallel Soroban (V1). Both must be iterated.
3. **XDR parsing is CPU-bound** — 76ms in Node.js vs estimated 5-10ms in native code for a heavy ledger (343 txs, 2.4MB).
4. **The team already has a working Rust implementation** (`rumblefishdev/stellar-indexer`) that correctly handles V3/V4 events, ScVal-to-typed-JSON conversion, and envelope extraction.

The architecture docs originally assumed TypeScript for all Lambda functions. This ADR proposes Rust specifically for the Ledger Processor, keeping the API Lambda in TypeScript (NestJS).

---

## Decision

Use **Rust** with the `stellar-xdr` crate for the Ledger Processor Lambda.

---

## Rationale

### 1. Canonical XDR types

The [`rs-stellar-xdr`](https://github.com/stellar/rs-stellar-xdr) crate is auto-generated from stellar-core's XDR definitions. It is the most accurate representation of XDR types — no wrapper layer, no manual type mapping. When a new protocol version ships, the crate is updated first.

The TypeScript SDK wraps XDR types with method accessors (`.ledgerSeq()` vs `.ledger_seq`). Union types return raw numbers from `.switch()` instead of typed enums. This caused issues during research — `meta.switch()` returned `4` as a raw number with `undefined` name, requiring manual dispatch. In Rust, `match meta { TransactionMeta::V4(v4) => ... }` is compile-time safe.

### 2. Existing reference implementation

`rumblefishdev/stellar-indexer` already implements:

- LedgerCloseMetaBatch parsing from zstd-compressed XDR
- V0/V1 transaction phase envelope extraction
- V3/V4 event handling with correct dispatch
- ScVal-to-typed-JSON conversion (`{ "type": "u128", "value": "123456" }`)
- Transaction hash computation via `env.hash(network_id)`

This codebase can be directly adapted for the Lambda handler, reducing implementation risk.

### 3. Performance headroom

| Metric             | TypeScript               | Rust (estimated)      |
| ------------------ | ------------------------ | --------------------- |
| Heavy ledger parse | 76ms                     | ~5-10ms               |
| Cold start         | 500-1500ms               | 100-300ms             |
| Memory overhead    | ~150MB (Node.js runtime) | ~20MB (static binary) |

While TypeScript performance is adequate (76ms with 5000ms budget = 65x headroom), Rust provides:

- Lower Lambda memory allocation → lower cost
- Faster cold starts → no need for provisioned concurrency
- Headroom for future growth (more TXs per ledger as Soroban adoption grows)

### 4. Lambda deployment model

Rust compiles to a static binary targeting `provided.al2023` Lambda runtime. No runtime dependencies, no `node_modules`. Binary size ~10-15MB. Deployment is a single zip file.

### 5. Separation of concerns

The Ledger Processor has a single job: parse XDR → write to DB. It doesn't serve HTTP, doesn't need NestJS, doesn't share request-time code with the API. The language boundary is clean — the processor writes to PostgreSQL, the API reads from PostgreSQL. No shared runtime code needed.

---

## Alternatives Considered

### Alternative 1: TypeScript (Node.js)

**Description:** Use `@stellar/stellar-sdk` in a Node.js Lambda, sharing the Nx monorepo with the API.

**Pros:**

- Single language across all Lambdas
- Shared domain types with NestJS API
- Shared ORM (Drizzle) for both indexer and API
- Simpler CI — one toolchain

**Cons:**

- XDR type safety weaker (raw switch values, no compile-time union dispatch)
- 5-10x slower parse performance
- Higher cold start, requires provisioned concurrency
- SDK v14.6.1 had undocumented V4 meta behavior (discovered during research)

**Decision:** REJECTED — XDR type safety and existing Rust implementation outweigh monorepo convenience.

### Alternative 2: Go

**Description:** Use `go-stellar-sdk`. Galexie and Horizon are both Go.

**Pros:**

- Stellar ecosystem native (Go-first tooling)
- Good Lambda cold start (static binary)
- `go-stellar-sdk` is well-maintained

**Cons:**

- No existing team implementation (unlike Rust)
- Go type system less expressive for XDR union matching than Rust `match`
- Same cross-language boundary issue as Rust
- Would need to learn a new codebase from scratch

**Decision:** REJECTED — Rust has an existing working implementation and stronger type safety.

### Alternative 3: Hybrid (Rust binary called from TypeScript Lambda)

**Description:** Compile Rust XDR parser as a CLI binary, invoke from Node.js Lambda via child process.

**Pros:**

- Keep Lambda runtime as Node.js (familiar deployment)
- Rust handles only the XDR-intensive part

**Cons:**

- Process spawning overhead per invocation
- Complex error handling across process boundary
- Two build systems for one Lambda
- Over-engineering for the performance gain

**Decision:** REJECTED — unnecessary complexity.

---

## Consequences

### Positive

- Compile-time XDR type safety prevents protocol upgrade surprises
- `stellar-indexer` codebase accelerates implementation
- Lower Lambda cost (less memory, no provisioned concurrency)
- Faster cold starts improve reliability during traffic spikes

### Negative

- Two languages in the monorepo (Rust for processor, TypeScript for everything else)
- Rust toolchain required in CI (can be containerized)
- Domain types (DB schema, ScVal JSON shapes) defined separately in Rust and TypeScript
- Team must maintain Rust proficiency for one component

### Mitigations

- DB schema is the shared contract — both languages read/write the same PostgreSQL tables
- ScVal typed JSON format (`{ type, value }`) is language-agnostic
- Rust component is isolated (single Lambda, single purpose) — bounded maintenance scope
- CI can use Docker-based Rust builds, no global toolchain needed

---

## References

- [Task 0002 research](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/README.md)
- [S-language-choice comparison](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/notes/S-language-choice-ledger-processor.md)
- [stellar-xdr crate](https://github.com/stellar/rs-stellar-xdr)
- [Protocol 25 upgrade guide](https://stellar.org/blog/developers/stellar-x-ray-protocol-25-upgrade-guide)
- [stellar-indexer reference (Rust)](../1-tasks/active/0002_RESEARCH_ledgerclosemeta-xdr-parsing/sources/stellar-indexer-events-mod-rs.md)
