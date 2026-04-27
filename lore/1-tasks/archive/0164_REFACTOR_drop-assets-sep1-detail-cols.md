---
id: '0164'
title: 'Drop `assets.description` and `assets.home_page`; move asset detail metadata to S3'
type: REFACTOR
status: completed
related_adr: ['0023', '0037']
related_tasks: []
tags: ['schema', 'assets', 's3', 'metadata', 'effort-small', 'priority-low']
links: []
history:
  - date: 2026-04-24
    status: backlog
    who: fmazur
    note: >
      Created. Emerged from ADR 0037 (current-schema-snapshot) review —
      `description` and `home_page` are unused by any current endpoint
      and planned asset detail metadata will fetch from S3.
      `icon_url` stays (list thumbnail role).
  - date: 2026-04-24
    status: active
    who: fmazur
    note: >
      Activated. Owner direction: no new superseding ADR — update ADR 0037
      in place so the snapshot reflects the post-drop state with a note on
      the S3 detail-metadata layout. ADR 0023 history untouched.
  - date: 2026-04-24
    status: completed
    who: fmazur
    note: >
      Completed. Migration 20260424000000 applied to Docker DB; 2 columns
      dropped (both NULL). ADR 0037 DDL + Mermaid ERD + S3-layout note
      updated in place (assets section §11). Domain `crates/domain/src/asset.rs`
      updated; `cargo check --workspace` passes. Frontend §6.9 and
      database-schema-overview §4.10 updated. Enrichment worker (writes
      `icon_url` to DB + detail JSON to S3) remains unimplemented —
      captured below as future work, not auto-spawned per owner preference.
---

# Drop `assets.description` and `assets.home_page`; move asset detail metadata to S3

## Summary

Reverse ADR 0023 Part 3 for two of the three typed SEP-1 columns on `assets`:
drop `description` and `home_page` from the DB, route those fields through S3
(asset metadata fetched at detail-view time). Keep `icon_url` in DB — it serves
list-page thumbnails where per-row S3 fetching would be prohibitive.

## Context

ADR 0023 Part 3 added `description`, `icon_url`, `home_page` as typed columns
on `assets` (at the time `tokens`) to hold SEP-1 enrichment data. Its
"Objection 2" rejected an S3 layout on the grounds that it would break the
one-JSON-per-ledger invariant from ADR 0011. Two observations invalidate that
objection for the two detail-only fields:

1. **These fields are not ledger-indexed.** They originate from off-chain
   `stellar.toml` (SEP-1) or Soroban RPC calls — zero source data in XDR, no
   natural association with any specific ledger. Forcing them into
   `parsed_ledger_{N}.json` would misclassify them. The honest shape is
   per-entity (e.g. `assets/{id}.json`).
2. **Product decision (Filip, 2026-04-24): asset detail view will fetch
   metadata from S3.** This moots the DB storage requirement for detail-only
   SEP-1 fields.

`icon_url` is excluded from the drop because the list endpoint
(`/assets`) is expected to render asset thumbnails per row; 50× S3 fetches
per list response is not acceptable. `icon_url` stays in DB and gets
re-classified in the new ADR from "SEP-1 detail enrichment" to "list-level
thumbnail".

No data loss: per the 2026-04-10 pipeline audit
(`docs/audits/2026-04-10-pipeline-data-audit.md §"tokens / metadata"`),
ADR 0023 Part 3 was never implemented and no current code path writes
these columns; every row has `description IS NULL` and
`home_page IS NULL`.

## Implementation Plan

### Step 1: Update ADR 0037 in place

Per owner direction: no new superseding ADR is written. Instead the
post-drop state is absorbed into the existing schema-snapshot ADR.

- Drop `description` and `home_page` from the `assets` DDL block
- Drop them from the Mermaid ERD entity
- Add a short note under the table describing the S3 layout for
  detail metadata (`s3://<bucket>/assets/{id}.json`) and the new
  role of `icon_url` (list thumbnail, not SEP-1 detail)
- Add task 0164 to `related_tasks`
- ADR 0023 history remains untouched; the ADR 0037 snapshot is the
  authoritative post-change record

### Step 2: SQLx migration

```sql
ALTER TABLE assets DROP COLUMN description;
ALTER TABLE assets DROP COLUMN home_page;
```

Verify migration applies cleanly on both empty and populated databases.

### Step 3: Rust domain + indexer cleanup

- `crates/domain/src/asset.rs`: drop `description` and `home_page` fields;
  update doc-comment to reference `ADR 0037 / task 0164`; keep `icon_url`
- Audit any `INSERT INTO assets (..., description, home_page, ...)` —
  none expected (fields were never written), but confirm with
  `rg "description|home_page" crates/indexer/src/handler/persist/`
- `cargo check` / `cargo test` clean

### Step 4: Evergreen docs update (ADR 0032)

- `docs/architecture/frontend/frontend-overview.md` §6.9 (Asset detail) —
  reflect that `description` and `home/home_page` come from S3, not DB
- `docs/architecture/database-schema/database-schema-overview.md` —
  reflect column drops
- ADR 0037 footer note (or leave as-is; schema is a snapshot and the newer
  migration is itself documentation)

### Step 5: Follow-up task (out of scope here)

Spawn a separate backlog task for the actual enrichment worker
implementation: SEP-1 TOML fetcher → S3 write for asset metadata JSON, DB
write only for `icon_url`. This task does **not** implement the worker;
it only removes the unused columns and documents the new contract.

## Acceptance Criteria

- [x] ADR 0037 updated in place: assets DDL + Mermaid ERD reflect the drop,
      explanatory note added about S3 detail layout and icon_url role
- [x] SQLx migration `20260424000000_drop_assets_sep1_detail_cols.{up,down}.sql`
      created and applies cleanly on local Docker Postgres
- [x] `crates/domain/src/asset.rs` updated; `cargo check` passes
      (`cargo check --workspace` passes clean)
- [x] `rg "assets\.description|assets\.home_page" crates/ docs/` returns
      no live references (ADR files are allowed)
- [x] Frontend overview §6.9 updated to describe S3-sourced metadata
- [x] `database-schema-overview.md` §4.10 updated (columns removed, notes revised)
- [x] **Docs updated** — per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md);
      files touched: `frontend-overview.md §6.9`,
      `database-schema-overview.md §4.10`

## Implementation Notes

Actual work done, in order:

1. **ADR 0037 update** — removed `description` + `home_page` from assets
   DDL block (§11) and from Mermaid ERD entity. Added a paragraph after
   the DDL note describing the S3 layout (`s3://<bucket>/assets/{id}.json`)
   and re-classifying `icon_url` as a list-thumbnail column. Added task
   0164 to `related_tasks` frontmatter.
2. **Migration pair** — `20260424000000_drop_assets_sep1_detail_cols.up.sql`
   and `.down.sql`. Up: two `ALTER TABLE assets DROP COLUMN` statements.
   Down: re-add the columns as nullable (ADR 0023 Part 3 shape; no
   backfill needed because data was always NULL). Applied cleanly on
   `sorban-block-explorer-postgres-1` via `docker exec … psql`.
3. **Domain** — `crates/domain/src/asset.rs` dropped two fields; doc
   comment rewritten to reference ADR 0037 + task 0164 instead of ADR 0023. `cargo check --workspace` passes clean (0.80 s for domain
   alone; 2.18 s total).
4. **Evergreen docs** — §4.10 and §6.9 updated per ADR 0032. §6.9 now
   explains that `description` and `domain/home page` come from S3
   (`assets/{id}.json`); `name` and `icon` stay on the DB row.

No code outside `crates/domain/src/asset.rs` touched — the audit grep
confirmed ADR 0023 Part 3's enrichment worker was never wired, so no
INSERT/UPDATE referenced the dropped fields anywhere.

## Design Decisions

### From Plan

1. **`icon_url` stays on the DB row.** List endpoint renders
   thumbnails per row; 50× S3 GETs per page was not acceptable. Its
   role was re-classified from "SEP-1 detail enrichment" (ADR 0023
   Part 3) to "list-level thumbnail URL" (ADR 0037 note).

2. **Per-entity S3 namespace (`assets/{id}.json`) for detail
   metadata.** Off-chain SEP-1 data is not ledger-indexed, so the
   per-ledger `parsed_ledger_{N}.json` pattern from ADR 0011 does
   not fit. Per-entity is the honest shape.

### Emerged

3. **No new superseding ADR was written** (was planned as ADR 0038 in
   the initial backlog task). Owner explicitly directed: absorb the
   change into the existing ADR 0037 schema snapshot and treat it
   as "as if ADR 0037 had been written after this change". ADR 0023
   history was not touched. The ADR 0037 snapshot is the authoritative
   post-change record.

4. **ADR 0023 history left untouched.** Corollary of #3. The
   contradictory state between ADR 0023 Part 3 (keeps columns) and
   ADR 0037 (doesn't show them) is resolved by the snapshot being
   the current reality; the older ADR stays as-is as a decision
   record for its time.

5. **Enrichment worker not spawned as a new task.** Per owner
   preference (memory: "don't spawn follow-up tasks unless asked"),
   the follow-up SEP-1 TOML → S3 worker is captured below as future
   work only. No backlog entry created without explicit ask.

## Issues Encountered

- **Initial approach rejected mid-flight.** First attempt wrote ADR
  0038 (supersedes ADR 0023 Part 3) and added a history entry to ADR 0023. Owner rejected on the "no new ADR" preference before the
  migration applied. Rolled back: moved `0038_asset-detail-metadata-to-s3.md`
  to `.trash/`, removed the ADR 0023 history entry, collapsed the
  intended 0038 content into a short note in ADR 0037. No code was
  reverted because no code had yet been touched.

- **Migration comment referenced the killed ADR 0038.** Caught on
  the post-edit sanity grep. Rewrote the header to reference ADR 0037
  - task 0164.

## Future Work

These are **not spawned as tasks** per owner preference ("don't
spawn follow-up tasks unless asked"). Captured here as prose for
future sessions:

- **SEP-1 / SEP-41 enrichment worker.** Fetches `stellar.toml` from
  issuer `home_domain` for classic/SAC; Soroban RPC for soroban-native.
  Writes `icon_url` + `name` to DB, writes the detail JSON document
  (`{description, home_page, enriched_at, enriched_source}`) to S3 at
  `assets/{id}.json`. Unchanged from ADR 0023 Part 3 worker shape
  except the write target split.

- **Backend `GET /assets/:id` S3 fetch path.** When the backend
  transactions/assets module is implemented (not yet), the handler
  for `/assets/:id` should issue a single S3 GET for
  `assets/{id}.json` in parallel with the DB row read. Graceful
  fallback to `null` fields on S3 miss.

- **ADR 0022 Part 3 alignment note.** ADR 0022 still refers to
  "typed metadata columns" as the enrichment target. A future
  housekeeping pass may want to reconcile ADR 0022 Part 3 language
  with the post-ADR 0037 state, but owner did not request this in
  the current pass.
