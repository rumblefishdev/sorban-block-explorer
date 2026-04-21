---
id: '0029'
title: 'Abandon parsed-ledger S3 artifacts; parse raw XDR on-demand at read time'
status: proposed
deciders: [Marek, Marcin, fmazur, stkrolikiewicz, karolkow]
related_tasks: ['0145', '0146', '0147', '0149', '0150']
related_adrs: ['0011', '0012', '0018', '0027', '0028']
tags: [architecture, s3, read-path, pivot]
links: []
history:
  - date: '2026-04-21'
    status: proposed
    who: stkrolikiewicz
    note: >
      Drafted following a team architectural decision to pivot from
      pre-parsed S3 artifacts to read-time XDR fetch against the public
      Stellar archive. Supersedes ADR 0028 wholesale; partially
      supersedes the S3-offload principle in ADR 0011/0012/0018.
      Aligns with the independent rescope of task 0145 (Karol, commit
      ebb307c, "pivot scope to postgres sink").
---

# ADR 0029: Abandon parsed-ledger S3 artifacts; parse raw XDR on-demand at read time

**Related:**

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md) — partially superseded
- [ADR 0012: Lightweight bridge DB schema (revision)](0012_lightweight-bridge-db-schema-revision.md) — partially superseded
- [ADR 0018: Minimal transactions detail to S3](0018_minimal-transactions-detail-to-s3.md) — partially superseded (read path reinterpreted)
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md) — unchanged; DB layout stays
- [ADR 0028: ParsedLedgerArtifact v1 shape](0028_parsed-ledger-artifact-v1-shape.md) — **superseded by this ADR**

---

## Context

ADRs 0011 / 0012 / 0018 established the "DB = lightweight index, our S3 =
heavy parsed JSON" pattern. ADR 0028 (proposed, now superseded) specified
the concrete shape of that artifact. Task 0146 began implementing it
(ADR 0028 + scaffold merged via PR #100).

In a team meeting on 2026-04-21 the architectural direction changed:

> No parsed-ledger S3 bucket on our side. If an endpoint needs data that
> is not in the DB (heavy fields like memo, signatures, raw XDR blobs,
> full event topics + data), fetch the raw `.xdr.zst` from the public
> Stellar archive (`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`)
> and parse it inline when assembling the response.

Task 0145 (owner: Karol) rescoped independently on the same day
(commit `ebb307c` — "pivot scope to postgres sink") along the same
direction: Postgres sink via `indexer::handler::process::process_ledger`,
no artifact emission, no dependency on task 0146. This ADR records the
decision, its consequences, and the remaining coordination (cancel
tasks 0146 and 0147, spawn task 0150, remove the `xdr-parser::artifact`
scaffold).

### What changes

- **No parsed-ledger artifact format** — the shape and storage described
  by ADR 0028 is abandoned.
- **No dedicated S3 bucket** for parsed artifacts on our side.
- **Read path** for E3 (`/transactions/:hash`) and E14
  (`/contracts/:id/events`) — any "heavy field" not persisted in the ADR
  0027 DB is fetched from the public Stellar archive at request time and
  parsed in-process before inclusion in the response.
- **Write path** — unchanged in principle: the indexer (`process_ledger`
  → `persist_ledger`) parses `LedgerCloseMeta` and writes the light
  column set into the ADR 0027 schema. The only thing that goes away is
  the subsequent artifact emission.

### What does not change

- **ADR 0027 DB schema** (18 tables). The schema and its endpoint
  realizability analysis remain intact.
- **`crates/xdr-parser`** — still the sole XDR parsing path (ADR 0004).
  Its role expands: used both at ingest and, for a small number of
  endpoints, at read time.
- **Galexie infrastructure** — raw ledger arrivals continue to land on
  the Galexie bucket and trigger the existing indexer lambda (from task
  0033).
- **Enrichment pipelines** — tokens, NFTs, LP TVL/volume/fee_revenue
  (tasks 0124 / 0125 / 0135 / 0138 / future) still fill their DB
  columns via their own schedules.

---

## Decision

1. **Cancel the parsed-ledger artifact storage track.** No bucket, no
   `.json.zst` files, no storage format spec. ADR 0028 is superseded.

2. **Cancel task 0146** (shared parsed-ledger artifact core) —
   superseded by the new write-path + read-path split (tasks 0149 +
   0150). Remove the `crates/xdr-parser/src/artifact/` module scaffold
   landed by PR #100.

3. **Cancel task 0147** (live Galexie onPut lambda emitting artifact) —
   the existing indexer lambda (task 0033) already calls
   `process_ledger` → `persist_ledger`; once task 0149 fills
   `persist_ledger`, the lambda writes to DB with no additional
   infrastructure. No separate task needed.

4. **Ingest goes directly to DB.** Task 0149 replaces the stubbed
   `persist_ledger` body with the full ADR 0027 write. The existing
   Galexie lambda and the new backfill runner (task 0145 per Karol's
   rescope — `crates/backfill-runner/`, Postgres sink, coexisting with
   `backfill-bench`) both call the same `process_ledger` entry point
   and rely on 0149 for the DB work.

5. **Heavy-field reads go to the public archive.** Spawn task 0150:
   API-side XDR fetcher for endpoints E3 and E14:

   ```
   API request → DB lookup for light fields + ledger_sequence
              → public Stellar S3 GetObject (unsigned)
              → xdr_parser::decompress_zstd + deserialize_batch
              → xdr_parser::extract_* (subset matching endpoint needs)
              → merge heavy fields into response JSON
   ```

6. **Caching is deferred.** Add cache only if measured hot-path latency
   proves insufficient. Spawn task 0151 at that point, not before.

7. **Production backfill gated on read-path validation.** The
   production run of task 0145 (not its implementation) waits for task
   0150 completion and E3/E14 endpoint integration tests against a
   small live sample. Rationale: committing weeks of compute before
   end-to-end validation is avoidable risk. Task 0145 implementation
   itself is not blocked.

---

## Rationale

### Simpler storage graph

One system of record (the DB) instead of two (DB + parsed S3). Fewer
failure modes, fewer consistency concerns (a DB row and a JSON blob
cannot disagree because the blob does not exist).

### Public archive is already canonical

Stellar Dev Foundation publishes `.xdr.zst` for every ledger on
`aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`. Mirroring that
corpus under our account adds cost without adding capability — the
upstream is the authoritative source and is globally accessible.

### Lower operational surface

No parsed-ledger bucket means: no bucket IAM policy, no lifecycle rules,
no cross-region replication decisions, no artifact-schema versioning /
re-emit drills. The ADR 0028 discipline (byte-for-byte determinism,
additive-only v1, etc.) evaporates along with the stored format.

### Parser reuse at read time

`crates/xdr-parser` already provides `decompress_zstd`,
`deserialize_batch`, and the full `extract_*` suite. Using it at read
time for E3/E14 is a straightforward invocation against bytes fetched
at request time. No new parsing code, just a new call site.

### Predictable M1 critical path

M1 requires all 22 endpoints to function. 20 of them read only from DB
and are unaffected by this pivot. The remaining 2 (E3, E14) consume the
public archive at request time. Task 0150 carries that specific work.

---

## Consequences

### Positive

- **One storage layer.** DB-only persistence eliminates a class of
  cross-store consistency bugs.
- **Zero mirror cost.** No TB-scale accumulation of `.json.zst` files
  in our S3.
- **Faster M1 integration path.** Existing indexer lambda + existing
  `backfill-bench` + new `backfill-runner` (task 0145) all reduce to
  "fill `persist_ledger` body" (task 0149). No new lambda to deploy,
  no new bucket to provision.
- **xdr-parser is load-bearing at read time.** Natural reuse; further
  optimisation of the parser benefits both paths.
- **Decoupled enrichment.** Tokens / NFTs / LP metrics continue on
  their own tracks; unaffected by the pivot.

### Negative

- **Public S3 dependency on the hot path for E3 and E14.** Two
  endpoints become dependent on the availability and latency of
  `aws-public-blockchain`. Mitigation: timeouts, graceful fallback to
  "details unavailable, retry shortly", observability on upstream
  health.
- **Latency budget tightens for E3 / E14.** Each detail request now
  incurs a public S3 GET (~20-100 ms) + zstd decompress (~5-20 ms) +
  XDR deserialize + targeted `extract_*` (~10-50 ms). Without caching,
  baseline p95 will be measurably higher than a DB-only endpoint.
- **Rate limits and egress accounting shift upstream.** High-traffic
  detail endpoints hit the public bucket repeatedly. If sustained
  traffic or cost proves material, task 0151 (cache layer) is the
  follow-up.
- **Lost work on ADR 0028 and task 0146.** The artifact shape spec
  and the scaffold are not reusable in the new architecture. Value
  salvaged: field-level shape thinking informs per-endpoint response
  contracts via xdr-parser; design archaeology preserved in git
  history.
- **M1 sequencing constraint.** Production backfill run (the execution
  of task 0145, not its implementation) waits for task 0150
  completion. Implementation can proceed in parallel with 0149 and 0150.

---

## Alternatives considered

### Alt 1: Continue with ADR 0028 artifact storage

**Description:** Complete task 0146 (PR 2/3), stand up a parsed-ledger
bucket, have Galexie lambda + backfill CLI emit `.json.zst`, run DB
ingester off those artifacts.

**Pros:** Predictable read-path latency (bucket is our infra, close to
API). Replay-friendly (re-parse with newer parser = rebuild corpus).

**Cons:** Multi-TB mirror storage; two write paths with synchronization
burden; artifact-schema versioning overhead; duplicate effort when the
public archive already hosts the raw data.

**Decision:** REJECTED.

### Alt 2: Serve everything from DB (no read-time XDR parse)

**Description:** Extend ADR 0027 schema to persist every heavy field
that E3 / E14 need.

**Pros:** All endpoints become pure DB reads. No public S3 dependency.

**Cons:** DB size explodes — the original motivation for S3 offload
(ADR 0011) re-applies here. Memo blobs, XDR envelopes, raw event
topics + data, signatures across millions of ledgers are the exact
payload the "light DB" principle was designed to keep out.

**Decision:** REJECTED.

### Alt 3: Hybrid — cache parsed blobs on our S3 lazily

**Description:** On first E3 / E14 hit, fetch from public archive,
parse, cache derived blob on our S3 for later requests.

**Pros:** Read latency improves after first hit; reduces public-archive
load.

**Cons:** Reintroduces an artifact bucket with its own lifecycle
concerns and consistency questions. Defers rather than removes the
complexity of ADR 0028.

**Decision:** DEFERRED — treat as a potential implementation of task
0151 (cache layer) once measurement justifies; not committed as the
default approach.

---

## Sequencing implications

| Phase           | Work                                                                                                             | Owner                       |
| --------------- | ---------------------------------------------------------------------------------------------------------------- | --------------------------- |
| 0               | Accept ADR 0029; cancel tasks 0146 + 0147; remove `xdr-parser::artifact` scaffold; spawn task 0150               | coordination                |
| 1               | Task 0149 — implement `persist_ledger` body against ADR 0027 schema                                              | Filip                       |
| 2               | Task 0145 implementation — `crates/backfill-runner/` Postgres sink CLI, coexisting with `backfill-bench`         | Karol                       |
| 2               | Task 0150 — API-side XDR fetcher + `extract_*` invocation for E3 / E14                                           | TBD (Stanisław recommended) |
| 3               | Integration test: live Galexie ingest of a sample range; API endpoint checks pass for light **and** heavy fields | team                        |
| 4               | Production backfill run (task 0145 execution, not implementation)                                                | operator                    |
| 5 (conditional) | Task 0151 cache layer if measured latency requires                                                               | —                           |

Phases 1 and 2 run in parallel. Phase 4 is gated on Phase 3
completion. Karol's rescope of 0145 (commit `ebb307c`) is consistent
with this sequencing and requires no further change beyond its own
merge.

---

## Open questions

1. **Public archive key-construction**: Galexie filenames encode
   `{hex_prefix}--{start}-{end}.xdr.zst` where the hex prefix is
   derived from the batch number. Task 0150 needs a deterministic
   forward map (`ledger_sequence → batch key`) or a narrow S3 list;
   `xdr-parser` already has the reverse parse (`parse_s3_key`) —
   verify symmetry during 0150 scoping.
2. **Timeout and fallback for public S3 GET**: what p99 latency budget
   does the API set before returning "heavy fields temporarily
   unavailable"? Task 0150 open.
3. **Observability**: metrics on public-archive GET latency, parse
   time, cache hit rate (once 0151 lands). Scope in 0150.
4. **Lost enrichment insurance**: without a parsed corpus, a parser
   bug discovered in production requires re-running ingest against the
   public archive. Acceptable given the archive is upstream-authoritative;
   documented here as an acknowledged operational property.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0018: Minimal transactions detail to S3](0018_minimal-transactions-detail-to-s3.md)
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md)
- [ADR 0028: ParsedLedgerArtifact v1 shape](0028_parsed-ledger-artifact-v1-shape.md) — superseded by this ADR
- [Task 0145 rescope commit `ebb307c`](https://github.com/rumblefishdev/soroban-block-explorer/commit/ebb307c656c092d5fccd82a2e0f58288384a0890) — Karol's pivot to Postgres sink
- [Public Stellar ledger archive](https://registry.opendata.aws/stellar-network/)
