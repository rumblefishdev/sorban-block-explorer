---
id: '0004'
title: 'Rust-only XDR parsing — eliminate TS on-demand decode'
status: accepted
deciders: [stkrolikiewicz, fmazur]
related_tasks: ['0024', '0025', '0026', '0027']
related_adrs: ['0002']
tags: [language-choice, xdr-parsing, rust]
links: []
history:
  - date: 2026-03-30
    status: proposed
    who: fmazur
    note: 'ADR proposed. Extends ADR 0002 (Rust Ledger Processor). Eliminates TS on-demand decode path.'
  - date: 2026-03-30
    status: accepted
    who: fmazur
    note: 'Accepted and implemented. TS XDR code removed, @stellar/stellar-sdk removed, docs updated.'
---

# ADR 0004: Rust-only XDR parsing — eliminate TS on-demand decode

**Related:**

- [ADR 0002: Use Rust for the Ledger Processor Lambda](0002_rust-ledger-processor-lambda.md)
- [Task 0024: XDR parsing: LedgerCloseMeta](../1-tasks/backlog/0024_FEATURE_xdr-parsing-ledgerclosemeta.md)
- [Task 0025: XDR parsing: operations](../1-tasks/backlog/0025_FEATURE_xdr-parsing-operations.md)
- [Task 0026: XDR parsing: Soroban events/invocations](../1-tasks/backlog/0026_FEATURE_xdr-parsing-soroban-events-invocations.md)
- [Task 0027: XDR parsing: LedgerEntryChanges](../1-tasks/backlog/0027_FEATURE_xdr-parsing-ledger-entry-changes.md)

---

## Context

ADR 0002 established Rust as the language for the Ledger Processor Lambda. The original architecture included a secondary on-demand decode path in the NestJS API using `@stellar/stellar-sdk` (TypeScript) for "advanced transaction views" — decoding stored `envelope_xdr`, `result_xdr`, and `result_meta_xdr` at request time.

This created two XDR parsing implementations in two languages with different type systems, both needing to stay in sync with Stellar protocol upgrades. The shared TS XDR library (`libs/shared/src/xdr/`) was built as infrastructure for this secondary path.

---

## Decision

All XDR parsing happens exclusively in Rust at ingestion time. The Rust Ledger Processor materializes all fields into the PostgreSQL read model. The NestJS API is pure CRUD — it reads pre-materialized data from the database. Zero `@stellar/stellar-sdk` in API runtime.

Raw XDR payloads (`envelope_xdr`, `result_xdr`, `result_meta_xdr`) are stored verbatim in the database for the "advanced view" but are returned as opaque base64 strings — no server-side decode.

---

## Rationale

- **Single parser, single language:** Maintaining two XDR parsers (Rust + TS) in sync across protocol upgrades is not justified on the green path. The Rust parser is authoritative.
- **API simplicity:** The NestJS API becomes a thin CRUD layer over PostgreSQL with no XDR dependencies, no binary parsing, and no protocol-version awareness.
- **No data loss:** Raw XDR is stored verbatim. If a field is missing from the materialized read model, the Rust parser can be updated and data re-ingested from the stored XDR.
- **Performance:** Eliminates on-demand CPU-bound decode work in Lambda cold path.

---

## Alternatives Considered

### Alternative 1: TS on-demand decode as fallback

**Description:** Keep `@stellar/stellar-sdk` in the API for decoding stored raw XDR when the advanced view needs fields not in the read model.

**Pros:**

- Immediate access to any XDR field without re-ingestion
- Fallback if Rust parser misses a field

**Cons:**

- Two parsers in two languages — maintenance cost of keeping both in sync with protocol upgrades
- `@stellar/stellar-sdk` is a heavy dependency for Lambda cold starts
- Creates ambiguity about which parser is authoritative
- The "fallback" incentivizes skipping fields in the Rust parser

**Decision:** REJECTED — maintenance cost of dual parsers is not justified on green path

---

## Consequences

### Positive

- Single XDR parsing implementation (Rust) — one place to update on protocol changes
- NestJS API has zero binary parsing dependencies — faster Lambda cold starts
- Cleaner architecture: ingestion writes complete read model, API reads it

### Negative

- If Rust parser doesn't extract a field, API cannot serve it until parser is updated and data is re-ingested
- Advanced view shows raw XDR as opaque strings — client-side decode is the user's responsibility
- Removing shared TS XDR lib (`libs/shared/src/xdr/`) — code written during research phase becomes dead code
