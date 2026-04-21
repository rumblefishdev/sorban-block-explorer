---
id: '0150'
title: 'API-side XDR fetch + parse for E3 and E14 heavy fields (ADR 0029 read path)'
type: FEATURE
status: backlog
related_adr: ['0027', '0029']
related_tasks: ['0149', '0145']
blocked_by: []
tags:
  [layer-backend, layer-api, priority-high, effort-medium, adr-0029, read-path]
milestone: 1
links:
  - lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
history:
  - date: '2026-04-21'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from ADR 0029. Replaces the read-path component of the
      abandoned ADR 0028 artifact architecture: when E3 (/transactions/:hash)
      or E14 (/contracts/:id/events) need heavy fields (memo, signatures,
      XDR blobs, full event topics + data), fetch raw .xdr.zst from the
      public Stellar archive at request time and parse in-process.
---

# API-side XDR fetch + parse for E3 and E14 heavy fields

## Summary

Add an API-side module that, for endpoints E3 (`/transactions/:hash`)
and E14 (`/contracts/:id/events`), fetches the raw `.xdr.zst` for the
relevant ledger from the public Stellar archive
(`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`) and invokes
`xdr-parser::extract_*` on the subset of data the endpoint needs. The
extracted heavy fields merge with light fields queried from the ADR
0027 DB to produce the final JSON response.

This replaces the read-path component of the abandoned ADR 0028
artifact architecture. No parsed-ledger bucket on our side — the public
archive is the authoritative source at request time.

## Status: Backlog

**Current state:** spec only. Implementation can start after task 0149
(write path) lands — needs DB rows to look up ledger_sequence from
tx hash or contract_id. Not a hard block; the XDR fetcher can be
built and unit-tested in isolation.

## Context

ADR 0029 records the team's architectural pivot: no parsed-ledger
bucket; heavy-field reads go to the public archive on demand. Tasks
0146 and 0147 are superseded; task 0149 fills `persist_ledger` against
ADR 0027; the existing indexer lambda (task 0033) writes live data.
The remaining gap is the read path: the API layer must assemble
detail-endpoint responses whose spec (per ADR 0027 Part III §E3 and
§E14) requires fields not persisted in the DB.

### Fields served from public-archive fetch

Per ADR 0027 Part III:

- **E3 `/transactions/:hash`**: memo (type + content), signatures
  array, fee-bump `feeSource`, operation raw parameters,
  `envelope_xdr` / `result_xdr` / `result_meta_xdr`, diagnostic
  events, full event topics + data (for nested events view).
- **E14 `/contracts/:id/events`** (detail expand): full
  `topics[1..N]` beyond the DB `topic0`, raw event `data`.

## Scope

### In scope

1. **New module** in `crates/api/` (exact layout to decide — likely
   `api::xdr_fetch` or similar).
2. **Public-archive S3 client** — `aws-sdk-s3` with unsigned request
   mode against `aws-public-blockchain`. Reuse `xdr-parser` primitives
   (`decompress_zstd`, `deserialize_batch`) rather than reimplementing.
3. **Ledger-sequence → S3 key mapping** — Galexie filenames are
   `{hex_prefix}--{start}[-{end}].xdr.zst` where the hex prefix is
   derived from the batch number. `xdr-parser::parse_s3_key` already
   parses this format in reverse; task needs the forward direction.
   Options:
   - Deterministic construction from `ledger_sequence` if the
     hex-prefix formula is documented or reverse-engineerable.
   - Narrow S3 `ListObjectsV2` query per request if direct
     construction is not feasible.
     Pick one during scoping.
4. **Per-endpoint extractors** — thin wrappers that call the right
   `xdr-parser::extract_*` functions for each endpoint's payload:
   - E3: `extract_transactions` (memo, result_code), envelope-level
     signatures + fee-bump fields, `extract_events` filtered to the
     target tx, `extract_invocations` tree.
   - E14: `extract_events` filtered to the target contract, full
     topics + data.
5. **Response DTO merging** — combine DB-sourced light fields with
   the XDR-sourced heavy fields into a single endpoint response.
6. **Error handling + timeouts** — public S3 404 (unexpected for a
   ledger already in our DB), network timeout, parse error, partial
   payload. Return graceful degradation ("heavy details temporarily
   unavailable, retry shortly") rather than 500 on transient
   upstream failures.
7. **Observability** — `tracing` spans for public S3 GET, decompress,
   deserialize, per-endpoint extraction. Emit latency metrics
   suitable for CloudWatch / Prometheus scraping.

### Out of scope

- **Cache layer** — deferred to task 0151 (to be spawned only if
  measured hot-path latency is unacceptable).
- **Any write-path work** — task 0149 owns `persist_ledger`.
- **Light-field endpoints** — endpoints E1/E2/E4-E13/E15-E22 remain
  DB-only and are unaffected by this task.
- **Our-side bucket** — no artifact bucket per ADR 0029.
- **Re-parsing DB-persisted fields** — anything already in the ADR
  0027 DB is served from DB, not XDR.

## Acceptance Criteria

- [ ] Public Stellar S3 key construction works for a known Soroban-era
      ledger range; unit-tested round-trip against `xdr-parser::parse_s3_key`.
- [ ] E3 response includes DB light fields **and** XDR-sourced heavy
      fields (memo, signatures, XDR blobs, diagnostic events, full
      event topics + data).
- [ ] E14 response includes DB light fields **and** full `topics[1..N]` + raw `data` per event.
- [ ] Integration test: staging API hit for a known tx hash and
      contract returns complete responses; verified against a golden
      response fixture.
- [ ] Observability metrics: public S3 GET latency (p50/p95/p99),
      total endpoint latency (p50/p95/p99), upstream error rate.
- [ ] Graceful degradation: on upstream timeout, API returns
      structured error indicating heavy fields unavailable; light
      fields still present.
- [ ] `nx run rust:build`, `nx run rust:test`, `nx run rust:lint` pass.

## Open questions

1. **Key construction**: can we forward-derive the Galexie hex prefix
   from `ledger_sequence` deterministically, or does the API need a
   per-request S3 list? Affects latency budget materially. Scope
   early.
2. **Timeout budget**: what p99 latency does the API promise for E3
   and E14? Determines aggressive-vs-conservative timeout configuration
   on the public S3 GET.
3. **Batch-level fetch optimization**: a Galexie batch contains 64
   ledgers. If the endpoint only needs one tx, do we parse the whole
   batch or stream-deserialize to locate the target? Affects CPU cost
   per request.
4. **Concurrency control**: many concurrent E3 requests for the same
   ledger would cause thundering herd against public S3. Do we need
   in-process request coalescing even before a dedicated cache task?
5. **Egress and rate limits**: does Stellar SDF publish usage guidance
   for the public archive? Scope budgeting once known.

## Risks / Notes

- **Public S3 availability** = E3 and E14 availability. Two of 22
  endpoints depend on upstream uptime. Other 20 endpoints unaffected.
- **Cache follow-up**: task 0151 spawned only after measurement shows
  need. Do not pre-engineer.
- **Design archaeology**: earlier artifact-based approach (ADR 0028 +
  task 0146) is documented in PR #100 + git history; the shape-level
  thinking informs per-endpoint response contracts here.
