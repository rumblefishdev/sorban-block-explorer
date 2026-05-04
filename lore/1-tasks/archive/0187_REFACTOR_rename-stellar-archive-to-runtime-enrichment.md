---
id: '0187'
title: 'Refactor: rename stellar_archive module to runtime_enrichment with stellar_archive/ + sep1/ submodules'
type: REFACTOR
status: completed
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
  - date: '2026-05-04'
    status: completed
    who: karolkow
    note: >
      Module renamed and split. 5 files git-mv'd into stellar_archive/
      submodule, 2 new files created (runtime_enrichment/mod.rs +
      sep1/mod.rs skeleton). 8 consumer files + 3 docs/architecture files
      updated. cargo check, clippy -D warnings, and 119/119 lib+integration
      tests green (5 ignored network-dependent tests skipped). Follow-up
      noted in Future Work — sep1 fetcher impl to be bundled with first
      consumer (assets/{id}) per Karol's "bigger-batch" task-scope
      preference.
---

# Refactor: rename stellar_archive module to runtime_enrichment

## Summary

Rename `crates/api/src/stellar_archive/` to `crates/api/src/runtime_enrichment/`. Keep the original code under a `stellar_archive/` submodule (transport-specific naming preserved). Add a `sep1/` submodule skeleton (empty `mod.rs` + module-level docstring describing intent) so the follow-up SEP-1 stellar.toml fetcher can land cleanly. No behavior change. All existing tests pass unchanged.

## Context

`stellar_archive` (ADR 0029) currently implements one source of runtime details enrichment: S3 archive reread of `.xdr.zst` ledger files for heavy fields (memo, signatures, envelope/result XDR, contract events) consumed by `GET /v1/transactions/{hash}` and `GET /v1/contracts/{id}/events`.

The M2 enrichment plan (post-MVP) adds a second runtime source: HTTP fetches of stellar.toml (SEP-1) for fields that the indexer does not and cannot populate from XDR — e.g. asset description, home_page, conditions, anchor info, organization info — surfaced on `GET /v1/assets/{id}` and (once the accounts module ships) `GET /v1/accounts/{id}`.

Both sources share the same architectural shape:

- per-request, in-process, cached only in warm Lambda memory (LRU)
- fail-soft: response always returns the DB-light slice; missing enrichment is signalled by an `enrichment_status: "ok" | "unavailable"` field (mirrors the existing `heavy_fields_status` pattern from ADR 0029)
- timeout-bounded so the API stays under the API Gateway 29s ceiling

Keeping both under one module — `runtime_enrichment` — captures that they are the same architectural concept (read-time, fail-soft, in-process). The submodules `stellar_archive/` and `sep1/` separate the two transport-specific concerns. No DB column backfill happens here: that is the type-1 enrichment worker (separate crate, separate task).

This refactor is the minimum structural step that lets the SEP-1 fetcher land in a follow-up task without churning every consumer file again.

## Implementation Plan

### Step 1: Rename module directory

`git mv crates/api/src/stellar_archive crates/api/src/runtime_enrichment`

### Step 2: Move existing code into `stellar_archive/` submodule

Inside `runtime_enrichment/`:

- create `stellar_archive/` subdirectory
- move `mod.rs` → `stellar_archive/mod.rs` (existing `StellarArchiveFetcher`, `FetchError`, `default_timeout_config`, S3 constants)
- move `dto.rs`, `extractors.rs`, `key.rs`, `merge.rs` → `stellar_archive/`

### Step 3: Add `sep1/` skeleton

Create `runtime_enrichment/sep1/mod.rs` with module-level docstring stating intent (HTTP stellar.toml fetcher per SEP-1, fail-soft, LRU-cached) and a single `// TODO(0188): impl` line. No types yet. Follow-up task wires the implementation.

### Step 4: New top-level `runtime_enrichment/mod.rs`

```rust
//! Runtime details enrichment — per-request, fail-soft, in-process.
//!
//! Two transport-specific submodules share a common shape: best-effort fetch,
//! merge into the DB-light slice, signal status via `enrichment_status`.
//!
//! - [`stellar_archive`] — S3 reread of public Stellar archive ledgers (ADR 0029).
//! - [`sep1`] — HTTP fetch of issuer stellar.toml files (M2 follow-up).

pub mod sep1;
pub mod stellar_archive;
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

Either change every import to `runtime_enrichment::stellar_archive::*` or rely on a re-export from `runtime_enrichment::*` for the existing surface. Pick one — prefer explicit submodule path so the SEP-1 path is forced to choose its own surface in the follow-up.

### Step 6: Docs

Per ADR 0032 evergreen policy, update files under `docs/architecture/backend/**` that reference `stellar_archive` or describe the heavy-fields read path. The module is not renamed in ADR 0029 itself — that ADR remains the source of truth for the archive submodule's behavior; a follow-up task amends 0029 once the SEP-1 path lands and a unified description makes sense.

## Acceptance Criteria

- [x] `crates/api/src/runtime_enrichment/{mod.rs, stellar_archive/, sep1/mod.rs}` exists; `crates/api/src/stellar_archive/` no longer exists at the old top-level path.
- [x] All existing tests under `stellar_archive/mod.rs` pass unchanged (network-dependent tests gated by `#[ignore]`; lib+integration suite 119/119 green).
- [x] `cargo check -p api` and `cargo clippy -p api -- -D warnings` clean.
- [x] No behavior change in `GET /v1/transactions/{hash}` heavy-fields response or `GET /v1/contracts/{id}/events` event expansion (covered by existing integration tests; all green).
- [x] `sep1/mod.rs` is intentionally empty save for the docstring + TODO marker; no public types exported yet.
- [x] **Docs updated** — `docs/architecture/database-schema/database-schema-overview.md`, `docs/architecture/technical-design-general-overview.md`, and `docs/architecture/xdr-parsing/xdr-parsing-overview.md` references updated from `stellar_archive` to `runtime_enrichment::stellar_archive` per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md). N/A for ADR 0029 (amendment deferred until the SEP-1 path lands).

## Implementation Notes

**Files touched (18 total):**

- 5 files git-mv'd: `crates/api/src/stellar_archive/{mod,dto,extractors,key,merge}.rs` → `crates/api/src/runtime_enrichment/stellar_archive/{mod,dto,extractors,key,merge}.rs`.
- 2 new files: `crates/api/src/runtime_enrichment/mod.rs` (top-level shell, declares `pub mod sep1; pub mod stellar_archive;`) and `crates/api/src/runtime_enrichment/sep1/mod.rs` (docstring + single `TODO` marker, no types).
- 8 consumer files updated: `lib.rs`, `main.rs`, `state.rs`, `tests_integration.rs`, `contracts/handlers.rs`, `network/handlers.rs`, `transactions/handlers.rs`, `openapi/mod.rs` — all `crate::stellar_archive::*` paths rewritten to `crate::runtime_enrichment::stellar_archive::*`. No re-export shim added (explicit submodule path was preferred so the SEP-1 path is forced to declare its own surface).
- 3 docs/architecture files updated: paths normalised to `runtime_enrichment::stellar_archive` so `grep` lookups land in the right module.

**Verification:**

- `cargo check -p api` — clean.
- `cargo clippy -p api -- -D warnings` — clean.
- `cargo test -p api --lib --bins` — 119 passed, 0 failed, 5 ignored (network-dependent integration tests gated by `#[ignore]`).

## Design Decisions

### From Plan

1. **Module top-level rename to `runtime_enrichment`**: captures the shared architectural concept across both transports (read-time, fail-soft, in-process, fixed budget). Replaces the transport-specific `stellar_archive` top-level name that no longer covers the full surface once SEP-1 lands.

2. **Explicit submodule paths in consumers (no re-export shim)**: every call site updated from `crate::stellar_archive::*` to `crate::runtime_enrichment::stellar_archive::*` rather than re-exporting the old surface from the new top-level. Forces the future SEP-1 path to choose its own surface intentionally instead of inheriting the archive's surface by accident.

### Emerged

3. **Submodule named `stellar_archive`, not `archive`**: original plan called the submodule `archive/`. Karol pushed back during implementation: keep the transport-specific name (`stellar_archive`) so the submodule directly mirrors the long-form data-source name (the AWS public Stellar archive). Result: paths read as `runtime_enrichment::stellar_archive::*`, which is more self-describing at every call site than `runtime_enrichment::archive::*`.

4. **Test functions renamed `_from_stellar_archive` → `_from_archive`**: with the module path now `runtime_enrichment::stellar_archive::tests::*`, the original suffix `_from_stellar_archive` was redundant against its own module name. Shortened to `_from_archive`. No test reference outside the module was affected; cargo command in the docstring updated to match.

5. **Docs scope expanded beyond `docs/architecture/backend/**`**: original acceptance criterion said only `backend/**`. `grep` revealed three references in other architecture trees (`database-schema/`, `xdr-parsing/`, top-level `technical-design-general-overview.md`). Updated all three; criterion text broadened to `docs/architecture/**`.

## Issues Encountered

- **Untracked task file blocked branch checkout**: `git checkout -b refactor/0187_… origin/develop` failed because the just-created `lore/1-tasks/active/0187_*.md` was untracked in the worktree but tracked at the develop tip (the `chore(lore-0187): activate task` commit pushed it there). Fix: moved the untracked copy to `.trash/` (per CLAUDE.md `rm` is forbidden), then checkout succeeded and the develop-tracked copy materialised in place. Not a regression — first time this worktree picked up a develop commit referring to the same path it had just created locally.

## Future Work

Tracked as separate tasks (one consolidated batch per Karol's task-scope preference):

- **SEP-1 fetcher + first consumer (assets/{id} runtime enrichment)** — bundle: implement `runtime_enrichment::sep1::*` (reqwest + toml + LRU + size cap + fail-soft `enrichment_status`) AND wire the first consumer at `GET /v1/assets/{id}` (description, home*page, conditions, anchor*\*, org info from stellar.toml). One task, not two. Spawned as `0188_FEATURE_sep1-fetcher-and-assets-details-enrichment` (separate task file).
- **Type-1 enrichment worker crate (`crates/enrichment-worker`) + first job (assets.icon_url backfill)** — bundle: scheduled Lambda crate skeleton + EventBridge cron + first concrete job in one task. Separate from this work; future ADR / task.
- **Archive 0124 / 0125 as superseded** — once both runtime + worker module pairs land. Cleanup task.
- **ADR 0029 amendment** — once `sep1/` has a real body and a unified description across both submodules is worth writing.

## Out of Scope

- SEP-1 fetcher implementation (follow-up task — `crates/api/src/runtime_enrichment/sep1/` body).
- Any consumer changes that surface SEP-1 enrichment (asset/account details — separate consumer tasks).
- Type-1 enrichment worker crate (separate crate, separate task).
- Archiving 0124 / 0125 as superseded — done in a later cleanup task once the two-module pattern is in place.
- ADR 0029 amendment — done once `sep1/` has a real body and a unified description is meaningful.

## Notes

This is the C-path foundation step from the M2 enrichment planning session: spawn the rename only, let the structure inform what the SEP-1 fetcher task actually needs, then write the next task with that context.
