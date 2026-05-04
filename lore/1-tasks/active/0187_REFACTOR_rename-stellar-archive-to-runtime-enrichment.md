---
id: '0187'
title: 'Refactor: rename stellar_archive module to runtime_enrichment with archive/ + sep1/ submodules'
type: REFACTOR
status: active
related_adr: ['0029', '0032']
related_tasks: ['0124', '0125']
tags:
  [
    priority-medium,
    effort-small,
    layer-backend,
    refactor,
    milestone-2,
    enrichment,
  ]
links: []
history:
  - date: '2026-05-04'
    status: backlog
    who: karolkow
    note: >
      Foundation refactor for the runtime details enrichment work. The current
      stellar_archive module (ADR 0029) implements only S3 archive reread for
      heavy fields. The post-MVP enrichment plan (M2) extends it with HTTP
      fetches of stellar.toml (SEP-1) for asset/account details. This task
      renames the module to runtime_enrichment and splits it into archive/
      (existing S3 path) and sep1/ (skeleton for the HTTP path) so the
      follow-up sep1 fetcher work can land without further structural churn.
      No behavior change in this task. Supersedes the JSONB-blob enrichment
      direction in 0124; 0124/0125 will be archived as superseded once the
      runtime + worker module pair is in place.
  - date: '2026-05-04'
    status: active
    who: karolkow
    note: >
      Promoted from backlog. Picked up as foundation step of M2 enrichment
      sequence (path C: spawn rename only, let structure inform sep1 fetcher
      task scope).
---

# Refactor: rename stellar_archive module to runtime_enrichment

## Summary

Rename `crates/api/src/stellar_archive/` to `crates/api/src/runtime_enrichment/`. Move existing code into a new `archive/` submodule. Add a `sep1/` submodule skeleton (empty `mod.rs` + module-level docstring describing intent) so the follow-up SEP-1 stellar.toml fetcher can land cleanly. No behavior change. All existing tests pass unchanged.

## Context

`stellar_archive` (ADR 0029) currently implements one source of runtime details enrichment: S3 archive reread of `.xdr.zst` ledger files for heavy fields (memo, signatures, envelope/result XDR, contract events) consumed by `GET /v1/transactions/{hash}` and `GET /v1/contracts/{id}/events`.

The M2 enrichment plan (post-MVP) adds a second runtime source: HTTP fetches of stellar.toml (SEP-1) for fields that the indexer does not and cannot populate from XDR — e.g. asset description, home_page, conditions, anchor info, organization info — surfaced on `GET /v1/assets/{id}` and (once the accounts module ships) `GET /v1/accounts/{id}`.

Both sources share the same architectural shape:

- per-request, in-process, cached only in warm Lambda memory (LRU)
- fail-soft: response always returns the DB-light slice; missing enrichment is signalled by an `enrichment_status: "ok" | "unavailable"` field (mirrors the existing `heavy_fields_status` pattern from ADR 0029)
- timeout-bounded so the API stays under the API Gateway 29s ceiling

Keeping both under one module — `runtime_enrichment` — captures that they are the same architectural concept (read-time, fail-soft, in-process). The submodules `archive/` and `sep1/` separate the two transport-specific concerns. No DB column backfill happens here: that is the type-1 enrichment worker (separate crate, separate task).

This refactor is the minimum structural step that lets the SEP-1 fetcher land in a follow-up task without churning every consumer file again.

## Implementation Plan

### Step 1: Rename module directory

`git mv crates/api/src/stellar_archive crates/api/src/runtime_enrichment`

### Step 2: Move existing code into `archive/` submodule

Inside `runtime_enrichment/`:

- create `archive/` subdirectory
- move `mod.rs` → `archive/mod.rs` (existing `StellarArchiveFetcher`, `FetchError`, `default_timeout_config`, S3 constants)
- move `dto.rs`, `extractors.rs`, `key.rs`, `merge.rs` → `archive/`

### Step 3: Add `sep1/` skeleton

Create `runtime_enrichment/sep1/mod.rs` with module-level docstring stating intent (HTTP stellar.toml fetcher per SEP-1, fail-soft, LRU-cached) and a single `// TODO(0188): impl` line. No types yet. Follow-up task wires the implementation.

### Step 4: New top-level `runtime_enrichment/mod.rs`

```rust
//! Runtime details enrichment — per-request, fail-soft, in-process.
//!
//! Two transport-specific submodules share a common shape: best-effort fetch,
//! merge into the DB-light slice, signal status via `enrichment_status`.
//!
//! - [`archive`] — S3 reread of public Stellar archive ledgers (ADR 0029).
//! - [`sep1`] — HTTP fetch of issuer stellar.toml files (M2 follow-up).

pub mod archive;
pub mod sep1;
```

Re-export the existing public API from `archive::*` if needed to keep call sites stable, or update consumers (Step 5).

### Step 5: Update consumers

Files referencing `stellar_archive`:

- `crates/api/src/main.rs`
- `crates/api/src/lib.rs`
- `crates/api/src/state.rs`
- `crates/api/src/transactions/handlers.rs`
- `crates/api/src/contracts/handlers.rs`
- `crates/api/src/network/handlers.rs`
- `crates/api/src/openapi/mod.rs`
- `crates/api/src/tests_integration.rs`

Either change every import to `runtime_enrichment::archive::*` or rely on a re-export from `runtime_enrichment::*` for the existing surface. Pick one — prefer explicit submodule path so the SEP-1 path is forced to choose its own surface in the follow-up.

### Step 6: Docs

Per ADR 0032 evergreen policy, update files under `docs/architecture/backend/**` that reference `stellar_archive` or describe the heavy-fields read path. The module is not renamed in ADR 0029 itself — that ADR remains the source of truth for the archive submodule's behavior; a follow-up task amends 0029 once the SEP-1 path lands and a unified description makes sense.

## Acceptance Criteria

- [ ] `crates/api/src/runtime_enrichment/{mod.rs, archive/, sep1/mod.rs}` exists; `crates/api/src/stellar_archive/` no longer exists.
- [ ] All existing tests under `archive/mod.rs` pass unchanged (including the `#[ignore]` integration tests when run with `--ignored`).
- [ ] `cargo check -p api` and `cargo clippy -p api -- -D warnings` clean.
- [ ] No behavior change in `GET /v1/transactions/{hash}` heavy-fields response or `GET /v1/contracts/{id}/events` event expansion (manual smoke or integration test confirms).
- [ ] `sep1/mod.rs` is intentionally empty save for the docstring + TODO marker; no public types exported yet.
- [ ] **Docs updated** — `docs/architecture/backend/**` references to `stellar_archive` updated to `runtime_enrichment::archive` per [ADR 0032](../2-adrs/0032_docs-architecture-evergreen-maintenance.md). N/A for ADR 0029 (amendment deferred until the SEP-1 path lands).

## Out of Scope

- SEP-1 fetcher implementation (follow-up task — `crates/api/src/runtime_enrichment/sep1/` body).
- Any consumer changes that surface SEP-1 enrichment (asset/account details — separate consumer tasks).
- Type-1 enrichment worker crate (separate crate, separate task).
- Archiving 0124 / 0125 as superseded — done in a later cleanup task once the two-module pattern is in place.
- ADR 0029 amendment — done once `sep1/` has a real body and a unified description is meaningful.

## Notes

This is the C-path foundation step from the M2 enrichment planning session: spawn the rename only, let the structure inform what the SEP-1 fetcher task actually needs, then write the next task with that context.
