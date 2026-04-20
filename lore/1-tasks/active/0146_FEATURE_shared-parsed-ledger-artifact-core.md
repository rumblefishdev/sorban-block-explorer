---
id: '0146'
title: 'Shared parsed-ledger artifact core (model + builder + JSON + zstd + key layout)'
type: FEATURE
status: active
related_adr: ['0012']
related_tasks: ['0145', '0147', '0117']
tags:
  [layer-backend, priority-high, effort-medium, adr-0012, foundation, parser]
milestone: 1
blocks: ['0145', '0147']
links:
  - lore/2-adrs/0012_zero-upsert-schema-full-fk-graph.md
history:
  - date: '2026-04-20'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0145 to carve out the shared artifact layer consumed by both
      the live Galexie lambda (0147) and the backfill runner (0145). Introduced
      so live + backfill cannot diverge on parsed JSON shape. Must freeze public
      API early — 0147 and 0145 block on it, run in parallel once API is locked.
  - date: '2026-04-20'
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active. Starting foundation work — freeze public API quickly
      (target ~2 working days) so 0147 and 0145 can run in parallel. Base
      branch refactor/lore-0140-adr-0027-schema; cut only after fmazur's Rust
      persistence rewrite lands.
---

# Shared parsed-ledger artifact core

## Summary

Introduce a single, reusable module that takes a decoded `LedgerCloseMeta`
and produces the canonical ADR 0012 `parsed_ledger_{seq}.json.zst` artifact.
Both the live Galexie onPut lambda (task 0147) and the offline backfill
runner (task 0145) consume this module. No I/O, no AWS, no DB — pure build +
serialize + compress + S3 key layout.

This is the foundation task. Its public API must be frozen quickly (target
~2 working days) so 0147 and 0145 can run in parallel without contract churn.

## Status: Active

**Current state:** activated 2026-04-20, not yet started. Base branch
`refactor/lore-0140-adr-0027-schema` must be stable (fmazur's Rust
persistence rewrite against ADR 0027 landed) before branch cut.

## Context

Today `crates/indexer/src/handler/process.rs` mixes parsing and DB persist in
one pass. `crates/backfill-bench` reuses that path, also ending in DB. The
ADR 0027 schema refactor (fmazur's task 0140, commit `89f4335`) rebuilt
the DB schema from scratch — 18 tables, surrogate account ids, BYTEA
hashes, monthly partitioning. ADR 0027 inherits ADR 0012's S3-offload
principle: heavy payloads live on S3 as parsed JSON, the DB stays a thin
lookup index. The Rust persistence rewrite against the new schema is the
next in-flight piece (33 expected compile errors in
`crates/db/src/{persistence,soroban}.rs` mark the follow-up surface).

Under ADR 0012 a single `parsed_ledger_{seq}.json.zst` per ledger lives in
our S3 bucket. The indexer DB becomes a thin index pointing into those files.
Both data origins — live events from Galexie and historical ledgers from
the public Stellar archive — must emit byte-identical artifacts. If parser
logic forks between live path and backfill path, we will eventually diff
the corpus and re-backfill to converge. Cheap to prevent now, expensive to
fix later.

## Scope

### In scope

1. **New module location** — `crates/xdr-parser/src/artifact/` submodule.
   Co-located with extraction code; no new crate. Keeps the artifact shape
   next to the types it composes (`ExtractedLedger`, `ExtractedTransaction`,
   etc.) and avoids a workspace-level crate split for what is effectively
   one builder + serializer.
2. **`ParsedLedgerArtifact` struct** — mirrors ADR 0012 §"File structure"
   exactly: `ledger_metadata`, `transactions[]` (hash, source_account,
   memo_type, memo, result_code, signatures, envelope_xdr, result_xdr,
   result_meta_xdr, operation_tree, operations[], events[], invocations[]),
   `wasm_uploads[]`, `contract_metadata[]`, `token_metadata[]`,
   `nft_metadata[]`. Derives `Serialize`, `Deserialize`, `Debug`.
3. **Schema version tag** — `ledger_metadata.schema_version: "v1"`. Required
   so downstream consumers can refuse unknown versions and we can re-emit
   safely if the shape changes post-0141.
4. **Public builder** —
   `pub fn build_parsed_ledger_artifact(meta: &LedgerCloseMeta) -> Result<ParsedLedgerArtifact, ParseError>`.
   Reuses existing `extract_*` functions already exported from `xdr-parser`.
   No new extraction logic — this task is purely composition.
5. **JSON serialization** — `pub fn serialize_artifact_json(a: &ParsedLedgerArtifact) -> Result<Vec<u8>, ArtifactError>`.
   Deterministic ordering (serde_json default + `BTreeMap` where we have
   maps). Empty arrays preserved (not skipped) for shape stability;
   consumers key off presence, not absence, of fields.
6. **Zstd compression** — `pub fn compress_artifact_zstd(json: &[u8]) -> Result<Vec<u8>, ArtifactError>`.
   Level 3 default. Level exposed via function arg (default const), not a
   builder — callers rarely override.
7. **S3 key layout** —
   `pub fn parsed_ledger_s3_key(sequence: u32) -> String` returning
   `parsed-ledgers/v1/{partition_start}-{partition_end}/parsed_ledger_{sequence}.json.zst`
   where partitions are 64k ledgers (aligned with `backfill-bench` partition
   math). `v1` mirrors `schema_version` and buys cheap re-emit headroom.
   Drop the `stellar/pubnet/` prefix proposed earlier — our bucket is
   already network-scoped by CDK stack; adding it duplicates that boundary.
8. **Error type** — `ArtifactError` local to the module; wraps `ParseError`
   for parse-side failures, `serde_json::Error` for serialize, `io::Error`
   for zstd. No `anyhow` in the public API.
9. **Tests** — unit tests per function + 5 golden fixtures covering:
   pure payment, Soroban invocation, WASM upload, NFT mint, liquidity
   pool op. Golden files checked into `tests/fixtures/` as raw
   `LedgerCloseMetaBatch` bytes + expected JSON. Determinism verified by
   re-serializing and byte-comparing.

### Out of scope

- **Refactor of `indexer/handler/process.rs`** — shifting the live indexer
  to consume this artifact is a follow-up (can be merged after 0147 ships).
  This task only adds new code; it does not rewire existing DB-bound paths.
- **Any I/O** — no S3 client, no filesystem, no network. Callers own I/O.
- **Partition math as a public API** — internal to the key builder.
  Consumers never compute partitions themselves.
- **CLI, lambda, or worker code** — lives in 0145 / 0147.
- **DB persist changes** — untouched.
- **Pre-Soroban ledgers** — same boundary as 0145 / indexer scope.

## Implementation Plan

### Step 1 — Confirm branch base

Verify fmazur's Rust persistence rewrite (follow-up to commit `89f4335`)
has landed on `refactor/lore-0140-adr-0027-schema`. If still in flight,
align on a sync point before branch cut. Do not branch off a moving
target.

### Step 2 — Model

Define `ParsedLedgerArtifact` + nested types. Map 1:1 to ADR 0012 §"File
structure". Reuse existing `Extracted*` where shapes match; introduce
artifact-local wrappers only where the JSON shape diverges from DB-oriented
fields.

### Step 3 — Builder

Implement `build_parsed_ledger_artifact`. Pure function over `&LedgerCloseMeta`.
No side effects. Returns `Err(ParseError)` on any extraction failure;
partial artifacts are **not** emitted — either the ledger serializes fully
or the caller handles the error.

### Step 4 — Serialize + compress

`serialize_artifact_json` → `serde_json::to_vec` with explicit config (no
pretty, no trailing newline). `compress_artifact_zstd` → `zstd::encode_all`
at level 3. Exposed as separate functions so callers can log JSON size
pre-compression for observability.

### Step 5 — Key builder

`parsed_ledger_s3_key(sequence)` — single allocation, format string.
Partition math shared with `backfill-bench` via a small `pub(crate) fn
partition_bounds(seq: u32) -> (u32, u32)`.

### Step 6 — Tests + golden fixtures

Golden fixtures picked from real mainnet ledgers (not synthesized) so any
drift from real XDR shape is caught. Fixtures small enough to commit
(<100 KB each after zstd).

### Step 7 — Freeze API + publish

Tag the public API as frozen in PR description. 0147 and 0145 unblock.
Any shape change after freeze requires coordinated update across all three
tasks — documented, not silent.

## Acceptance Criteria

- [ ] `xdr-parser::artifact` module exposes `ParsedLedgerArtifact`,
      `build_parsed_ledger_artifact`, `serialize_artifact_json`,
      `compress_artifact_zstd`, `parsed_ledger_s3_key`.
- [ ] Public API compiles cleanly with no `anyhow` leakage; local
      `ArtifactError` type wraps domain errors.
- [ ] Artifact JSON matches ADR 0012 §"File structure" byte-for-byte on 5
      golden fixtures covering diverse ledger content.
- [ ] `ledger_metadata.schema_version == "v1"` present in every artifact.
- [ ] Deterministic serialization — re-running builder + serializer on the
      same `LedgerCloseMeta` produces byte-identical output.
- [ ] S3 key layout: `parsed-ledgers/v1/{partition_start}-{partition_end}/parsed_ledger_{seq}.json.zst`
      with 64k partition size.
- [ ] `nx run rust:build`, `nx run rust:test`, `nx run rust:lint` pass.
- [ ] Public API frozen — PR description states contract and marks the
      freeze so 0147 and 0145 can start.

## Risks / Notes

- **API freeze discipline** — the whole point of this task is a stable
  contract. If scope changes land after freeze, 0147 and 0145 both have to
  rebase. Keep the surface small.
- **Shape vs ADR 0012 proposed state** — ADR 0012 is proposed, 0141 may
  still move. `schema_version: "v1"` covers re-emit; no versioned structs
  yet (YAGNI).
- **Golden fixtures drift** — mainnet XDR is append-only but new
  operation types can appear. Fixture set is representative, not
  exhaustive; refresh when new op types hit mainnet.
- **No workspace crate** — intentional. A separate crate would force a
  cross-crate dep graph change (`indexer` → new crate → `xdr-parser`) for
  a module that is 100% composition. Submodule keeps the blast radius
  minimal.
