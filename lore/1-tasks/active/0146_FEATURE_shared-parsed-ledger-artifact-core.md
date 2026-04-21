---
id: '0146'
title: 'Shared parsed-ledger artifact core (model + builder + JSON + zstd + key layout)'
type: FEATURE
status: active
related_adr: ['0012', '0027', '0028']
related_tasks: ['0145', '0147', '0117', '0126', '0135']
tags:
  [
    layer-backend,
    priority-high,
    effort-medium,
    adr-0012,
    adr-0027,
    adr-0028,
    foundation,
    parser,
  ]
milestone: 1
links:
  - lore/2-adrs/0012_lightweight-bridge-db-schema-revision.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
  - lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md
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
  - date: '2026-04-20'
    status: active
    who: stkrolikiewicz
    note: >
      Scope expansion after ADR review. ADR 0012 mentions parsed_ledger_{seq}.json
      but does not define its structure; ADR 0027 (accepted) defines DB schema but
      not the S3 artifact shape. This task therefore DEFINES the artifact shape
      and will spawn ADR 0028 formalizing ParsedLedgerArtifact v1. Not pure
      composition — a real design decision load-bearing for live lambda (0147),
      backfill (0145), and the future DB ingester.
---

# Shared parsed-ledger artifact core

## Summary

Introduce a single, reusable module that takes a decoded `LedgerCloseMeta`
and produces a canonical `parsed_ledger_{seq}.json.zst` artifact. Both the
live Galexie onPut lambda (task 0147) and the offline backfill runner
(task 0145) consume this module. No I/O, no AWS, no DB — build, serialize,
compress, and S3 key layout.

This task has two deliverables:

1. **Rust implementation** — `crates/xdr-parser::artifact` submodule with
   the public API frozen for parallel consumption.
2. **ADR 0028** formalizing the `ParsedLedgerArtifact v1` JSON shape.
   Neither ADR 0012 (proposed) nor ADR 0027 (accepted) defines the artifact
   structure — only that "one JSON file per ledger on S3" exists. This task
   picks the shape and records the decision.

Foundation task. Public API must be frozen quickly (target ~2 working days)
so 0147 and 0145 can run in parallel without contract churn.

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

### Artifact shape: no authoritative source yet

ADR 0012 (`proposed`) mentions only _"one JSON file per ledger on S3,
`parsed_ledger_{sequence}.json`, write-once, immutable"_ — no field-level
breakdown. ADR 0027 (`accepted`) defines DB schema and endpoint realizability
but refers to S3 payload fields only descriptively (Part III §E3 and §E14 list
what S3 must carry: memo, signatures, fee-bump feeSource, op raw params, XDR
blobs, diagnostic events, full event `topics[1..N]` and raw data). Neither
ADR captures a concrete serialized structure.

This task therefore defines the shape. Because the decision binds the live
lambda (0147), backfill (0145), and every future DB ingester for the full
artifact corpus, it is promoted to its own ADR (0028) rather than buried in
module docs.

### Shape principles (to be formalized in ADR 0028)

- **Public-readable identities**: StrKey (`G…`/`C…`) for accounts and
  contracts in JSON — not the DB surrogate `BIGINT` ids from ADR 0026.
  Consumers (including the DB ingester) resolve StrKey → surrogate at write
  time.
- **Hashes as hex strings (64 chars)** in JSON — ADR 0024's BYTEA(32) is a
  DB storage choice, not a wire format.
- **Empty arrays preserved** (not omitted) for stable field presence across
  ledgers and versions.
- **`ledger_metadata.schema_version` marker** so consumers can refuse
  unknown versions (placed inside `ledger_metadata`, not root, so future
  v2 can extend root with new sections without moving the version tag).

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
2. **`ParsedLedgerArtifact` struct + ADR 0028 shape spec** — defined by
   this task (no prior authoritative source). Root composition:
   `ledger_metadata`, `transactions[]` (hash, source_account, memo_type,
   memo, result_code, signatures, envelope_xdr, result_xdr,
   result_meta_xdr, operation_tree, operations[], events[], invocations[]),
   `wasm_uploads[]`, `contract_metadata[]`, `token_metadata[]`,
   `nft_metadata[]`. StrKey for accounts/contracts, hex for hashes. Derives
   `Serialize`, `Deserialize`, `Debug`. Shape recorded in ADR 0028 drafted
   as part of PR 1.
3. **Schema version tag** — `ledger_metadata.schema_version: "v1"`. Required
   so downstream consumers can refuse unknown versions and we can re-emit
   safely if the shape changes. Version semantics defined in ADR 0028.
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

### Step 2 — Model + ADR 0028 draft

Define `ParsedLedgerArtifact` + nested types. Derive the shape from what
extract\__ functions produce + what consumers need (ADR 0027 Part III lists
per-endpoint S3 dependencies as ground truth for required fields). Do NOT
reuse `Extracted_` directly — those are DB-schema-oriented (snake_case DB
column names, DB surrogate semantics). Artifact-local types use StrKey and
hex. Draft ADR 0028 alongside: shape, field conventions, versioning rule.

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

### Step 7 — Freeze API + publish + promote ADR 0028

Tag the public API as frozen in PR description. Promote ADR 0028 from
`proposed` to `accepted` (it was drafted in Step 2 and refined through
PRs 2/3). 0147 and 0145 unblock. Any shape change after freeze requires a
new ADR (0028 revision or supersede) and coordinated re-emit across all
three tasks — documented, not silent.

## Acceptance Criteria

- [ ] `xdr-parser::artifact` module exposes `ParsedLedgerArtifact`,
      `build_parsed_ledger_artifact`, `serialize_artifact_json`,
      `compress_artifact_zstd`, `parsed_ledger_s3_key`.
- [ ] Public API compiles cleanly with no `anyhow` leakage; local
      `ArtifactError` type wraps domain errors.
- [ ] Artifact JSON matches ADR 0028 spec byte-for-byte on 5 golden
      fixtures covering diverse ledger content.
- [ ] ADR 0028 drafted, accepted upon task completion. Covers: root
      structure, field naming, StrKey/hex conventions, versioning rule.
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
- **Shape decision is load-bearing** — ADR 0028 locks the JSON structure
  for the lifetime of the v1 corpus. Change requires a new ADR plus
  re-emit of the whole corpus (millions of ledgers). Draft ADR 0028
  carefully; get at least one independent review before the PR 1 freeze.
- **ADR 0012/0027 alignment** — ADR 0012 stays `proposed`, ADR 0027 is
  accepted. ADR 0028 sits downstream of both and describes the S3 side
  that ADR 0027 treats only as dependencies.
- **Golden fixtures drift** — mainnet XDR is append-only but new
  operation types can appear. Fixture set is representative, not
  exhaustive; refresh when new op types hit mainnet.
- **No workspace crate** — intentional. A separate crate would force a
  cross-crate dep graph change (`indexer` → new crate → `xdr-parser`) for
  a module that is 100% composition. Submodule keeps the blast radius
  minimal.
