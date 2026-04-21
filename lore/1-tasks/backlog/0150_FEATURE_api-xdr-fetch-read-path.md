---
id: '0150'
title: 'API-side XDR fetch + parse for E3 and E14 heavy fields (ADR 0029 read path)'
type: FEATURE
status: backlog
related_adr: ['0027', '0029']
related_tasks: ['0149', '0145']
blocked_by: []
tags:
  [
    layer-backend,
    layer-api,
    priority-high,
    effort-medium,
    adr-0029,
    read-path,
    draft,
  ]
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
  - date: '2026-04-21'
    status: backlog
    who: stkrolikiewicz
    note: >
      Marked DRAFT. Scope, acceptance criteria, and open questions here
      are a starting point only — expected to be revised after task 0149
      (write-path) completes. 0149 will finalise which fields actually
      live in the ADR 0027 DB vs which genuinely need XDR fetch at read
      time; it may also surface parser extensions (e.g. for 0126 /
      0138) that shift the read-path scope. Don't treat this task body
      as frozen; re-read and update when 0149 lands.
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

## Status: Backlog — DRAFT

**Current state:** spec is a **draft** starting point, not a frozen
contract. Expected to be revised after task 0149 (write-path)
completes. 0149 finalises which fields live in the ADR 0027 DB
vs which genuinely need XDR fetch at read time; until then, the
list of heavy fields below should be treated as an upper bound.
Additional parser extensions from tasks 0126 (LP participants) or
0138 (Soroban per-account balances) may further reduce what this
task needs to fetch from XDR.

Implementation can in principle start in parallel with 0149 (the
XDR fetcher + key construction are independent), but refining
scope — and therefore acceptance criteria — is gated on 0149
merge.

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
   `{hex_prefix}--{start}[-{end}].xdr.zst` where the hex prefix
   follows the repo's existing reversed/XOR convention:
   `u32::MAX - ledger` for file keys, `u32::MAX - partition_start`
   for partition (folder) keys. See
   `crates/xdr-parser/src/lib.rs::parse_s3_key` docs and
   `crates/backfill-bench/src/main.rs::Partition::from_ledger` for
   the existing implementations. `parse_s3_key` already parses this
   format in reverse; task needs the forward direction using the
   same formula.
   Options:
   - Deterministic construction from `ledger_sequence` /
     `partition_start` using the documented existing convention.
   - Narrow S3 `ListObjectsV2` query per request only as a fallback
     if a batch file contains a variable `-{end}` suffix that the
     forward map alone cannot resolve.
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

1. **Key construction**: implement the forward mapping using the
   existing repo convention (`u32::MAX - ledger` for file keys,
   `u32::MAX - partition_start` for partition directories; see
   `xdr-parser::parse_s3_key` and `backfill-bench::Partition`). Add
   unit tests that verify round-trip symmetry with
   `xdr-parser::parse_s3_key`. An S3 list fallback remains open if
   any observed batch file has a variable `-{end}` suffix the
   forward map cannot reconstruct deterministically. Affects latency
   budget materially; scope early.
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
