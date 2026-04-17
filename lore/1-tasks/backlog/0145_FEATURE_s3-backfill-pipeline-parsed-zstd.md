---
id: '0145'
title: 'Backfill pipeline: Stellar pubnet S3 → parsed JSON → zstd → local + our S3'
type: FEATURE
status: backlog
related_adr: ['0012']
related_tasks: ['0117', '0140', '0141', '0142']
tags:
  [layer-backend, layer-infra, priority-high, effort-large, backfill, adr-0012]
milestone: 1
links:
  - lore/2-adrs/0012_zero-upsert-schema-full-fk-graph.md
history:
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Created to pre-produce ADR 0012's `parsed_ledger_{seq}.json.zst` artifacts on
      our S3 bucket before schema migration (0142) lands. Reuses parser from 0117
      (archived backfill benchmark) but replaces the persist step with S3 upload —
      schema-free, so this work is not blocked by 0142 / 0141.
---

# Backfill pipeline: Stellar pubnet S3 → parsed JSON → zstd → local + our S3

## Summary

Build an end-to-end offline pipeline that streams the entire Soroban-era Stellar
pubnet archive from the public S3 bucket, parses each ledger through the existing
Rust XDR parser, emits the ADR 0012 `parsed_ledger_{seq}.json` shape, compresses
it with zstd, stores it in a local directory, and uploads the `.json.zst` to our
S3 bucket. The DB persist step is intentionally **out of scope** — schema is still
in flux (ADR 0012 proposed; finalization under 0141).

The goal: when the schema migration (0142) lands and the indexer goes live, the
S3 archive of parsed ledgers is already populated for the entire historical range.
Migration cutover becomes a matter of indexing S3 → DB, not re-parsing XDR from
scratch.

## Status: Backlog

**Current state:** Not started. No hard blockers — runs against current parser
output, no schema dependency. Can start immediately.

## Context

ADR 0012 §"S3 offload" specifies one `parsed_ledger_{sequence}.json` file per
ledger in our S3 bucket, containing all heavy parsed data (XDRs, operation_tree,
signatures, memo, event payloads, invocation args/returns, WASM uploads, contract
metadata, token metadata, NFT metadata, `ledger_metadata` header). The indexer
reads these files to populate lightweight DB rows + detail-endpoint S3 fetches.

Task 0117 (archived) built a local benchmark CLI that streamed ledgers from the
public Stellar S3 bucket and ran them through the existing parser + DB persist.
This task reuses the streaming + parsing + zstd handling from 0117 but:

- Replaces the DB persist step with "emit `parsed_ledger_{seq}.json` + upload to
  our S3" (no DB).
- Produces the full ADR 0012 file structure, not the pre-ADR Extracted\* structs.
- Targets the complete Soroban-era range (Protocol 20 launch → current network
  tip), not just a benchmark slice.

Starting the backfill before schema migration means:

- The compute cost (parsing tens of millions of ledgers) is paid once, not twice.
- When 0142 schema lands and 0141 closes, we already have weeks/months of parsed
  S3 artifacts ready — the indexer's DB-only persist step can walk them quickly.
- Decouples parsing infrastructure from schema decisions; 0141 open questions can
  shift without invalidating this work.

## Scope

### In scope

1. **Source pull** — stream `*.xdr.zst` files from
   `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` starting at the
   first Soroban-activation ledger (Protocol 20 go-live on mainnet, ~ledger
   53_795_300 / April 2024; confirm exact ledger in implementation). Stream-and-
   delete pattern inherited from 0117 so local disk does not fill up.
2. **Parse** — reuse `crates/xdr-parser` to produce the full in-memory parsed
   representation of each ledger.
3. **Assemble ADR 0012 file shape** — serialise to
   `parsed_ledger_{seq}.json` per ADR 0012 §"File structure":
   `ledger_metadata`, `transactions[]` (hash, source_account, memo_type, memo,
   result_code, signatures, envelope_xdr, result_xdr, result_meta_xdr,
   operation_tree, operations[], events[], invocations[]),
   `wasm_uploads[]`, `contract_metadata[]`, `token_metadata[]`, `nft_metadata[]`.
4. **Compress** — zstd-compress each JSON to `parsed_ledger_{seq}.json.zst`.
5. **Local sink** — write to a configurable local directory (default:
   `./parsed-ledgers/`). Files are immutable after write.
6. **S3 sink** — upload the `.json.zst` to our S3 bucket under a key layout
   aligned with ADR 0012 expectations (exact prefix TBD in implementation —
   verify against 0141 outcome). Upload is idempotent (skip if object exists
   with matching size/ETag).
7. **Parallelism** — multi-worker pipeline (Tokio `JoinSet` or similar) with
   configurable concurrency. Order-agnostic; per-ledger output is independent.
8. **Progress + resume** — track completion via a local watermark file or by
   listing S3. Crashed runs resume at the last persisted ledger.
9. **CLI** — new Rust binary crate (e.g. `crates/s3-backfill-pipeline/`) with
   subcommands for `run`, `status`, `rehash-s3`.

### Out of scope

- **Any DB persist** — no writes to PostgreSQL. Schema (ADR 0012) is still
  proposed, tables do not exist. Deferred to 0142.
- **Activity projections, `_current` projections, `search_index`** — all DB-only,
  deferred to 0142.
- **Rollup Lambdas** — `contract_stats_daily`, `volume_24h`. Deferred.
- **Live ingestion path** — this is strictly backfill. The production indexer
  Lambda (task 0044 / 0029) is a separate concern; it can read from our S3 once
  populated.
- **Pre-Soroban ledgers** — indexer's initial scope is Soroban-era only. Earlier
  Stellar history is out of M1.
- **File structure changes post-0141** — if ADR 0012 shifts the JSON shape before
  acceptance, this pipeline may need to re-emit. Mitigation: keep the parser
  output format-agnostic, serialize to JSON as the last step, version the output
  with a schema identifier in `ledger_metadata`.

## Implementation Plan

### Step 1 — Scaffold CLI crate

- New `crates/s3-backfill-pipeline/` Rust binary crate in the workspace.
- Dependencies: `xdr-parser` (reuse parse), `aws-sdk-s3`, `zstd`, `tokio`,
  `serde_json`, `anyhow`, `clap`.
- CLI subcommands: `run`, `status`, `rehash-s3`, `config`.

### Step 2 — S3 source reader

- `aws-sdk-s3` with `--no-sign-request` equivalent (public bucket).
- List partitions under `v1.1/stellar/ledgers/pubnet/` from the Soroban
  go-live partition onward.
- Stream `*.xdr.zst` objects, decompress in memory.

### Step 3 — Parse + assemble ADR 0012 JSON

- Feed the decompressed XDR bytes through `xdr-parser::process_ledger`.
- Map the parser output into the ADR 0012 file shape (new serialization module
  in this crate — do not pollute `xdr-parser` with JSON-shape logic).
- Omit empty arrays (e.g. no `nft_metadata[]` for pre-NFT ledgers) per JSON
  convention.

### Step 4 — Zstd + local sink

- Compress with `zstd` level 3 (default; tune if needed).
- Write atomically via tempfile + rename to the local directory.
- Filename: `parsed_ledger_{sequence}.json.zst`.

### Step 5 — S3 upload

- `PutObject` with `If-None-Match: *` (or a pre-flight `HeadObject`) for
  idempotent re-runs.
- Key layout: `parsed-ledgers/parsed_ledger_{sequence}.json.zst` (placeholder —
  final layout confirmed against 0141 / 0142 key conventions).
- Content-Type: `application/zstd`. Content-Encoding unset (object is literally
  zstd).

### Step 6 — Parallelism + resume

- `tokio::task::JoinSet` over configurable worker count.
- Source iterator produces ledger ranges; each worker owns a range and emits
  `.json.zst` artifacts.
- Resume: on startup, query S3 for highest `parsed_ledger_*.json.zst` key and
  skip already-uploaded ledgers. Optional local watermark for fast restart.

### Step 7 — Observability

- Structured logs (tracing) per ledger: parse duration, JSON size, zstd ratio,
  upload duration.
- Periodic progress summary (ledgers/sec, ledgers remaining, ETA).
- Exit code non-zero on unrecoverable errors.

### Step 8 — Integration verification

- Pick 10 random ledgers covering diverse content (pure payment, Soroban
  invocation, NFT mint, WASM upload, liquidity pool op). Verify JSON shape
  matches ADR 0012 §"File structure" exactly. Unit tests per section.

### Step 9 — Run against Soroban-era range

- Dry run on staging S3 with a 1000-ledger window.
- Full historical run targeting our production S3 bucket.

## Acceptance Criteria

- [ ] CLI crate `crates/s3-backfill-pipeline/` builds and passes lints.
- [ ] Reads from `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` without auth.
- [ ] Emits `parsed_ledger_{sequence}.json.zst` artifacts conforming to ADR 0012
      §"File structure" for at least 10 handpicked ledgers (integration tests).
- [ ] Local sink directory configurable and idempotent.
- [ ] S3 sink uploads and skips existing objects.
- [ ] Resumes from last uploaded ledger after crash / SIGTERM.
- [ ] Configurable worker count; measured throughput documented in README.
- [ ] Full Soroban-era range processed successfully to production S3.
- [ ] Output JSON shape re-verified after ADR 0012 → accepted (0141 ships);
      if shape changes, re-emit is feasible or migration path documented.

## Risks / Notes

- **JSON shape stability** — ADR 0012 is proposed. If 0141 changes the file
  structure, we may need a re-emit pass. Mitigation: keep the serialization
  module thin and versioned; include a schema version tag in `ledger_metadata`.
- **S3 key layout** — final prefix convention should match whatever the indexer
  (0142) reads from. Coordinate with 0141 if the ADR does not pin it.
- **Cost** — multi-TB ingress from public S3 is free (same region if we run in
  us-east-1). Egress from our S3 to Lambda at migration time is the larger
  cost; minimize by running the indexer in the same region as the bucket.
- **Compute cost** — parsing tens of millions of ledgers is CPU-heavy. A
  throughput benchmark (inherited from 0117) should inform whether to run this
  on a workstation, ECS Fargate burst, or a dedicated spot instance.
- **Independent of 0141 / 0142** — this task pre-produces artifacts; it does
  not commit us to the final schema. Worst case: JSON shape needs a minor
  re-serialisation pass later. The raw parsed data stays valid.
