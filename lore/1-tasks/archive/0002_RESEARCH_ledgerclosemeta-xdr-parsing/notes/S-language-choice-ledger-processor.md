---
title: 'Decision needed: Rust vs Go vs TypeScript for Ledger Processor'
type: synthesis
status: mature
spawned_from: null
spawns: []
tags: [language-choice, ledger-processor, architecture]
links:
  - https://github.com/stellar/stellar-galexie
  - https://github.com/stellar/rs-stellar-xdr
  - https://github.com/stellar/go-stellar-sdk
history:
  - date: 2026-03-26
    status: developing
    who: stkrolikiewicz
    note: 'Comparison based on research findings and stellar-indexer reference'
  - date: 2026-03-26
    status: mature
    who: stkrolikiewicz
    note: 'ADR-0002 proposed based on this analysis. Rust recommended.'
  - date: 2026-03-30
    status: mature
    who: stkrolikiewicz
    note: 'Annotated with ADR-0002/0004 outcomes. Preliminary assessment was pre-decision; Rust was chosen.'
---

# Decision needed: Rust vs Go vs TypeScript for Ledger Processor

## Context

The Ledger Processor is the core ingestion engine — it parses LedgerCloseMeta XDR from S3 and writes structured records to PostgreSQL. The language choice impacts parse performance, XDR type fidelity, team velocity, and deployment model.

Three options are viable. All have working XDR parsing implementations.

## Comparison

| Factor                  | Rust                                                                                                           | Go                                                                                                | TypeScript                                                                |
| ----------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| **XDR library**         | [`stellar-xdr`](https://github.com/stellar/rs-stellar-xdr) crate (canonical, auto-generated from stellar-core) | [`go-stellar-sdk`](https://github.com/stellar/go-stellar-sdk) (official, used by Horizon/Galexie) | `@stellar/stellar-sdk` (wraps `stellar-base` JS XDR)                      |
| **Reference impl**      | `stellar-indexer` (our team, working)                                                                          | Galexie itself, `stellar-ledger-data-indexer`                                                     | This research task (explore-xdr.mjs)                                      |
| **Parse performance**   | ~5-10ms per heavy ledger (native)                                                                              | ~15-30ms per heavy ledger                                                                         | ~76ms per heavy ledger (Node.js, measured)                                |
| **Protocol 25 support** | `stellar-xdr` tracks stellar-core XDR, first to support                                                        | `go-stellar-sdk` is official, quick support                                                       | SDK v14.6.1 supports P25, but V4 event handling required manual discovery |
| **V3/V4 meta handling** | Pattern-matched, explicit (see stellar-indexer events/mod.rs)                                                  | Similar pattern matching on Go structs                                                            | `meta.switch()` returns raw number, not typed — requires manual dispatch  |
| **Lambda deployment**   | Custom runtime or container, larger binary (~10MB)                                                             | Custom runtime or container (~5MB)                                                                | Native Lambda runtime, smallest cold start                                |
| **Cold start**          | ~100-300ms (static binary)                                                                                     | ~50-200ms (static binary)                                                                         | ~500-1500ms (Node.js + SDK module load)                                   |
| **Team expertise**      | stellar-indexer exists                                                                                         | —                                                                                                 | NestJS API is in TS, shared domain types                                  |
| **Shared code w/ API**  | None — separate language boundary                                                                              | None                                                                                              | Domain types, ScVal decode, error handling shared with NestJS             |
| **DB driver**           | sqlx, diesel (mature)                                                                                          | pgx, lib/pq (mature)                                                                              | drizzle-orm (task 0007, chosen for this project)                          |
| **Ecosystem fit**       | Stellar core is C++, XDR is generated for Rust                                                                 | Stellar ecosystem is Go-first (Horizon, Galexie, SDKs)                                            | Nx monorepo is TS, all other apps are TS                                  |

## Analysis

### Rust

**Pros:** Fastest parse, `stellar-indexer` already works, strongest type safety on XDR unions, no GC pauses.

**Cons:** Separate language in an otherwise TypeScript monorepo. No shared domain types with NestJS API. Requires Rust toolchain in CI. Team must maintain two languages.

### Go

**Pros:** Stellar ecosystem native (Galexie, Horizon are Go). Fastest cold start with static binary. `go-stellar-sdk` is the first-party SDK.

**Cons:** Same cross-language boundary issue as Rust. Go type system less expressive for XDR union handling. No existing team implementation.

### TypeScript

**Pros:** Shared monorepo, shared domain types with NestJS API. Drizzle ORM for both indexer and API. Simpler CI, single language. SDK works (proven in this research).

**Cons:** 5-10x slower parse than Rust (76ms vs ~10ms — but still 50x under Lambda budget). Weaker XDR type safety (`meta.switch()` returns raw numbers). Cold start penalty.

## Preliminary Assessment

The parse performance gap (76ms TS vs ~10ms Rust) is **irrelevant for our use case** — we have 5000ms budget per ledger and parse takes <100ms in the worst case. The bottleneck is DB writes, not XDR parsing.

The **monorepo cohesion** argument strongly favors TypeScript: shared types, shared ORM, single CI, single language for the team. Introducing Rust or Go means maintaining a separate build pipeline, separate dependency management, and a language boundary for domain types.

However, the `stellar-indexer` Rust codebase is valuable as a **reference implementation** regardless of language choice — its V3/V4 event handling patterns and ScVal typed JSON format should be adopted in the TypeScript implementation.

> **[Historical — superseded by ADR-0002 and ADR-0004]** This preliminary assessment was written before ADR decisions. The team chose Rust for XDR type safety (compile-time exhaustive `match`) and the existing `stellar-indexer` reference implementation, accepting the cross-language boundary cost. ADR-0004 further eliminated the TS on-demand decode path entirely — the NestJS API is pure CRUD with no XDR dependencies.

## Open Questions

1. Is there a Go or Rust Lambda runtime constraint that affects the decision?
2. Does the team have Go experience?
3. Should the Ledger Processor be a Lambda at all, or an ECS task (which favors Rust/Go for long-running processes)?
4. Could the processor be written in Rust as a standalone binary invoked by Lambda (hybrid approach)?

## Status

**Mature** — analysis complete. ADR-0002 **accepted** (2026-03-26). ADR-0004 further established Rust-only XDR parsing, eliminating the TS on-demand decode path.
