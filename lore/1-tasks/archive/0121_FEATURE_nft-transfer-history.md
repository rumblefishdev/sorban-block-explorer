---
id: '0121'
title: 'NFT transfer history: schema + API endpoint'
type: FEATURE
status: archive
related_adr: ['0027', '0033', '0037']
related_tasks: ['0051', '0118']
superseded_by: ['0051']
tags: [layer-backend, layer-db, nfts, audit-gap, superseded]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
  - https://github.com/rumblefishdev/soroban-block-explorer/pull/152
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design requires GET /nfts/:id/transfers but no schema exists.'
  - date: '2026-05-04'
    status: archive
    who: stkrolikiewicz
    note: >
      Closed as superseded by task 0051 + PR #152.

      Schema: `nft_ownership` partitioned table (mig 0005:79, ADR 0027
      §13) was added by task 0118 Phase 2 (PR #110, archived
      2026-04-22). Full mint/transfer/burn timeline persisted at write
      time with NFT-vs-Fungible classification filter.

      API endpoint: PR #152 (open, ostatni commit 2026-05-04 10:08)
      ships `GET /v1/nfts/:id/transfers` as part of task 0051's NFTs
      module — wires canonical SQL `17_get_nfts_transfers.sql`
      verbatim into `crates/api/src/nfts/queries.rs`. PR #152 also
      fixed a `LAG` → `LEAD` bug in the canonical query (DESC-ordered
      window puts older event at FOLLOWING position) and replaced the
      "API stitches across page boundaries" caveat with a `LIMIT+1`
      peek-for-has-more pattern that lets the peek row participate in
      the LEAD window so the last KEPT row's from_account is correct
      without client-side stitching. 24 new tests, 143 total passing.

      `blocked_by` cleared:
      - 0136 was archived (REFACTOR surrogate BIGSERIAL ids — done long
        ago); stale entry in 0121 frontmatter.
      - 0118 Phase 3 (post-backfill SQL cleanup of `nfts` registry) is
        operational defensive scope, NOT a schema/endpoint blocker.
        Current backfill (PID 47507, started 2026-05-04 ~10:03 UTC) ran
        post-Phase-2-merge so writes are clean — Phase 3 SQL is a no-op
        for it. Phase 3 close decision deferred until audit-harness
        validates `nfts` clean (option B in 2026-05-04 PM sync).

      Phase 3 of 0118 retains its own closure path; it does not gate
      0121.
---

# NFT transfer history: schema + API endpoint

## Summary

The technical design specifies `GET /nfts/:id/transfers` and a "Transfer history" section
on the NFT detail page, but no `nft_transfers` table exists in the schema. The `nfts` table
only stores current owner — transfer history is lost.

## Implementation

Option A: Create an `nft_transfers` table populated during indexing from mint/transfer/burn
events.

Option B: Query `soroban_events` filtered by NFT contract + transfer topic pattern at API
query time (no new table, but slower and requires careful index design).

Recommendation: Option A — dedicated table with proper indexes for fast history queries.

**Blocker:** Task 0118 (NFT false positive fix) must be completed first, otherwise the
transfer history table will also be flooded with spurious fungible transfer entries.

## Acceptance Criteria

- [x] NFT transfer history queryable by contract_id + token_id — delivered by `nft_ownership` table (mig 0005:79, ADR 0027 §13) populated by task 0118 Phase 2 write path
- [x] Each transfer records: from, to, ledger_sequence, timestamp, event_type (mint/transfer/burn) — `nft_ownership` schema covers all fields; `from_owner` derived via LEAD window function (PR #152 fix from earlier LAG bug)
- [x] API endpoint `GET /nfts/:id/transfers` returns paginated transfer history — delivered by PR #152 (task 0051) wiring canonical SQL `17_get_nfts_transfers.sql` into `crates/api/src/nfts/queries.rs`
- [x] Indexer populates transfer records during event processing — task 0118 Phase 2 (PR #110) wired the write path with NFT-vs-Fungible classification filter

## Implementation Notes

No code or schema work performed under this task ID. Investigation on 2026-05-04 (during a backlog triage pass) revealed the scope had been delivered by sibling tasks before this one progressed past the blocked stage:

- **Schema (`nft_ownership` partitioned table):** ADR 0037 / migration `0005_tokens_nfts.sql:71` defines the table with PK `(nft_id, created_at, ledger_sequence, event_order)`, partition by `created_at` monthly, composite FK back to `transactions(id, created_at)`. Stored fields: `nft_id`, `created_at`, `ledger_sequence`, `event_order`, `event_type` SMALLINT (NftEventType enum 0=mint / 1=transfer / 2=burn), `owner_id` (NULL on burn), `transaction_id`. The "dedicated table" of Option A — landed without ever being labelled as 0121's deliverable.

- **Indexer write path:** Task 0118 Phase 2 (PR #110, archived 2026-04-22) added the WASM-spec classifier and gated `nft_ownership` writes — only contracts classified as `Nft` produce rows, `Fungible`/`Token` events are skipped. Same write path used by historical backfill and live forward ingestion.

- **API endpoint:** PR #152 (task 0051, currently open) ports `17_get_nfts_transfers.sql` verbatim into `crates/api/src/nfts/queries.rs`. Includes:

  - `LAG` → `LEAD` correction for `from_account` derivation on a DESC-ordered window
  - `LIMIT+1` peek-for-has-more pattern instead of client-side cross-page stitching
  - Filter scope widened from "transfers only" to full ownership timeline (mint + transfer + burn) per task 0051's "loose endpoint name" decision.

- **Canonical query:** Documented in `docs/architecture/database-schema/endpoint-queries/17_get_nfts_transfers.sql` (added by task 0167, archived 2026-04-27). One index seek per page, partition pruning on cursor `created_at`.

The two implementation options listed in this task were ultimately resolved as **Option A**, but executed as part of the broader 0118 + 0167 + 0051 thread, not as a standalone 0121 PR.

## Design Decisions

### From Plan

1. **Closed as superseded rather than re-scoped to a residual task:** Every AC has a concrete delivered shipping vehicle (0118 PR #110 for write path, 0167 for canonical SQL, PR #152 for endpoint). Re-scoping 0121 to e.g. "endpoint test coverage" would inflate effort tracking with work that is naturally part of PR #152's review surface.

### Emerged

2. **Recognition lag for supersession:** `nft_ownership` was added by 0118 Phase 2 in late April and 0051 PR #152 was opened 2026-04-29, but 0121 remained `blocked` in the index until the 2026-05-04 backlog triage. Cause: `blocked_by: [0118, 0136]` masked the fact that the schema (Phase 2 of 0118) had landed independently and only Phase 3 (operational SQL) was waiting on backfill — a state that does not gate downstream consumers. Same pattern as task 0123 closure earlier in the same triage session.

3. **Stale `blocked_by` entry for 0136:** Task 0136 (REFACTOR surrogate BIGSERIAL ids) was archived. Frontmatter never refreshed. Future preventive: when archiving a task, sweep `grep -l "blocked_by.*<id>" lore/1-tasks/` and update referencing tasks in the same commit.

4. **Phase 3 of 0118 NOT counted as a 0121 blocker post-triage:** Original 0121 history called Phase 3 a blocker because false positives in `nft_ownership` would produce junk transfer history. Reality (post-2026-05-04 audit): write path filters BEFORE row creation, so post-Phase-2 backfills produce zero false positives. Phase 3 SQL only matters for legacy pre-Phase-2 data — none exists in any live DB at this point. Endpoint correctness is decoupled from Phase 3 execution.

## Issues Encountered

- **Phantom-duplicate detection lag (~2 weeks):** Same root cause as task 0123 closure — sibling tasks (0118 Phase 2 and 0051) shipped scope incrementally without explicitly closing this task. Frontmatter `blocked_by` did not auto-resolve when one of the blockers (0136) was archived.

- **Stale "Option A vs Option B" framing:** Original task spec offered "dedicated table OR query soroban_events at runtime." After ADR 0033/0029 landed, Option B became architecturally infeasible (`soroban_events_appearances` is an appearance index only — raw event payload lives in S3, fetched at read time). The "options" framing was already dead by the time 0118 Phase 2 / 0167 chose Option A definitively.

## Future Work

None spawned. The `GET /nfts/:id/transfers` endpoint will start returning data once:

- PR #152 merges (currently open as of 2026-05-04 10:08)
- `nft_ownership` is populated, either via the active audit-DB backfill (PID 47507, ETA ~24h) on the audit harness path, or via a future production-DB backfill + live indexer post-launch.

If PR #152 surfaces unexpected behavior on real data once backfill finishes, file follow-up bugs against task 0051 (the active owner of the NFTs module), not this archived task.
