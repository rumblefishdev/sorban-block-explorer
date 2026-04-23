---
id: '0154'
title: 'REFACTOR: rename `tokens` → `assets` (Stellar taxonomy alignment)'
type: REFACTOR
status: active
related_adr: ['0022', '0023', '0027', '0032', '0033']
related_tasks: ['0118', '0120', '0124', '0135', '0155']
tags:
  [
    layer-db,
    layer-indexer,
    layer-api,
    layer-docs,
    schema,
    refactor,
    naming,
    priority-medium,
    effort-medium,
  ]
links:
  - notes/R-assets-vs-tokens-taxonomy.md
  - notes/S-asset-type-label-remap.md
  - notes/G-docs-architecture-rename-scope.md
history:
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from research note `notes/R-assets-vs-tokens-taxonomy.md`.
      Rename eliminates the `tokens` table vs `contract_type = 'token'`
      role-label collision and aligns with the official Stellar taxonomy
      (Assets vs Contract Tokens are distinct categories, not synonyms).
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      README restructured per review feedback on PR #107 (convention is
      ~50-100 lines). Heavy per-file `docs/architecture/**` scope moved
      to `notes/G-docs-architecture-rename-scope.md`; label-remap
      reasoning moved to `notes/S-asset-type-label-remap.md`.
  - date: '2026-04-22'
    status: active
    who: stkrolikiewicz
    note: >
      Activated for implementation. Scope: rename tokens→assets per
      Stellar taxonomy, see README.
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Zwolniony z active — priorytet na 0139. Wraca do backlog.
  - date: '2026-04-23'
    status: active
    who: karolkow
    note: >
      Activated for implementation.
---

# REFACTOR: rename `tokens` → `assets` (Stellar taxonomy alignment)

## Summary

Rename the `tokens` table to `assets`, remap the `asset_type` label
`classic` → `classic_credit`, and propagate the rename through the
Rust domain, xdr-parser, indexer persist path, integration tests, API
surface, and every affected file under `docs/architecture/**`. Schema
shape (partial unique indexes, `ck_tokens_identity` → `ck_assets_identity`,
FK to `soroban_contracts`) is unchanged — only the name. The collision
between "token" as a table name and "token" as a SEP-41 contract role
goes away.

## Context

Full motivation: [notes/R-assets-vs-tokens-taxonomy.md](notes/R-assets-vs-tokens-taxonomy.md).
Label-remap decision: [notes/S-asset-type-label-remap.md](notes/S-asset-type-label-remap.md).

Our `tokens` table already carries `native` + `classic` rows that have
no SEP-41 surface — i.e. "assets" in Stellar-speak. The name is a
legacy artefact of the Soroban-first iteration, not a decided choice.
Simultaneously, `soroban_contracts.contract_type = 'token'` classifies
a contract's role. The overlap is a real ambiguity in team
conversations.

Coordinate with:

- ADR 0032 / task 0155 — evergreen `docs/architecture/**` policy; the
  doc updates here live in that same spirit.
- ADRs 0030 / 0031 — surrogate IDs and `SMALLINT` enums. The rename
  must thread through the Rust enum + `token_asset_type_name` helper,
  not a legacy `VARCHAR` CHECK.
- Tasks 0118 / 0120 / 0124 / 0135 — all touch the same surface;
  sequence so this rename does not collide mid-stream.

## Scope

### DB schema (reversible migration pair)

- `ALTER TABLE tokens RENAME TO assets`; FKs follow automatically.
- Rename `ck_tokens_*` → `ck_assets_*`, `uidx_tokens_*` →
  `uidx_assets_*`, `idx_tokens_*` → `idx_assets_*`.
- Remap `asset_type`: `classic` → `classic_credit`. `native`, `sac`,
  `soroban` unchanged.
- Post-ADR 0031: the remap happens in the Rust `TokenAssetType` enum
  (`Classic` → `ClassicCredit`) + `token_asset_type_name` SQL helper,
  not in a `VARCHAR` CHECK. Integration test iterating every variant
  needs matching updates.

### Rust code

`crates/domain/src/token.rs` → `asset.rs`; `struct Token` → `Asset`.
Propagate through `crates/xdr-parser/` (`ExtractedToken`,
`detect_tokens`, `classify_token`), `crates/indexer/src/handler/`
(`TokenRow`, `upsert_tokens*`, `token_rows`, `tokens_ms` metric field,
`persist_ledger` parameter), and `crates/indexer/tests/`. Expect
~15–20 touched files. `cargo check` as guard.

### API surface

`GET /tokens*` → `/assets*`. Keep `/tokens*` as aliases with a
`Deprecation` header for one release; drop in a follow-up. Update
`/search?type=token,...` similarly. Regenerate OpenAPI + client
types.

### docs/architecture/\*\* and metrics

Per-file edit list: [notes/G-docs-architecture-rename-scope.md](notes/G-docs-architecture-rename-scope.md).
`infrastructure-overview.md` verified-clean; the other six files all
need changes.

Metric/log field `tokens_ms` → `assets_ms`. Coordinate a Grafana /
CloudWatch dashboard PR so there is no gap.

### Out of scope

- ADR files 0022 / 0023 / 0027 — historical records, not renamed.
- `soroban_contracts.contract_type = 'token'` — role label stays;
  collision disappears once the table is renamed.
- `nfts` table — holds instances, no ambiguity.
- `asset_code VARCHAR(12)` deduplication, XLM ↔ XLM SAC link gap,
  T-REX compliance flavour column — separate follow-ups.

## Acceptance Criteria

- [x] ADR drafted and `accepted`, referenced from `related_adr`.
- [x] Reversible migration pair (up + down) lands and tested against
      a restored staging dump in both directions.
- [x] `cargo build --workspace` + `cargo clippy --all-targets
-- -D warnings` + `cargo test -p indexer persist_integration`
      green.
- [ ] Axum routes live at `/assets*`; `/tokens*` aliased or dropped
      per the ADR; OpenAPI regenerated; frontend types aliased or
      updated.
- [x] Every file in [notes/G-docs-architecture-rename-scope.md](notes/G-docs-architecture-rename-scope.md)
      updated; `infrastructure-overview.md` explicitly flagged as
      verified-unchanged in the PR description.
- [x] `tokens_ms` → `assets_ms` across dashboards; ops channel
      notified.
- [ ] 100-ledger backfill bench p95 within ±5 % of pre-rename
      baseline.

## Risks

- Public API breakage if already live to external consumers —
  mitigate with alias window documented in the ADR.
- Dashboard silent breakage from metric rename — mitigate by
  co-landing the dashboard PR and announcing in the ops channel.
- Tokens-table surface drift with tasks 0118 / 0120 / 0124 / 0135 —
  sequence deliberately; this rename is low-risk but disruptive if
  mid-stream against another task.
- Overlap with task 0155 on `docs/architecture/**` — coordinate
  ordering; do not open both PRs against the same files at once.

## Notes

- Mechanical rename only. No opportunistic schema changes, no
  endpoint reshaping. Clean diff reviews and reverts better.
- Effort ≈ 1–2 developer-days per research note §9.7, excluding the
  API versioning call.
