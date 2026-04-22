---
id: '0032'
title: '`docs/architecture/**` becomes evergreen — maintained in sync with ADRs'
status: proposed
deciders: [stkrolikiewicz]
related_tasks: ['0154', '0155']
related_adrs:
  [
    '0022',
    '0023',
    '0024',
    '0025',
    '0026',
    '0027',
    '0028',
    '0029',
    '0030',
    '0031',
  ]
tags: [docs, process, governance]
links:
  - docs/architecture/technical-design-general-overview.md
  - docs/architecture/backend/backend-overview.md
  - docs/architecture/database-schema/database-schema-overview.md
  - docs/architecture/frontend/frontend-overview.md
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - docs/architecture/infrastructure/infrastructure-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
  - lore/1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md
history:
  - date: '2026-04-22'
    status: proposed
    who: stkrolikiewicz
    note: >
      Drafted after the tokens-vs-assets taxonomy research note surfaced
      concrete drift between
      `docs/architecture/technical-design-general-overview.md` and the real
      schema (4-value `asset_type` vs 3 in the doc, `VARCHAR(20)` vs
      `VARCHAR(10)`, partial unique indexes vs plain UNIQUE, missing
      `ck_tokens_identity`, plus analogous drift in other sections). The
      analogous pattern holds for other files under `docs/architecture/**`
      (backend, database-schema, frontend, indexing-pipeline,
      infrastructure, xdr-parsing). None of these have been updated since
      the Soroban-first iteration, yet ADRs 0022–0031 have reshaped the
      schema, persist path, and endpoint surface substantially. We can
      either keep treating these docs as snapshots of an old design
      iteration, or we commit to keeping them up to date. This ADR
      captures the latter decision and the process that makes it
      sustainable.
---

# ADR 0032: `docs/architecture/**` becomes evergreen — maintained in sync with ADRs

**Related:**

- [Task 0154: rename tokens → assets](../1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/README.md)
- [Task 0155: ADR-history audit of docs/architecture](../1-tasks/backlog/0155_DOCS_docs-architecture-adr-history-audit.md)
- ADRs 0022, 0023, 0024, 0025, 0026, 0027, 0028, 0029, 0030, 0031

---

## Context

Until now `docs/architecture/**` has been treated as a **design-time
snapshot** from the Soroban-first iteration of the project. It contains:

- `technical-design-general-overview.md` — the original system design.
- Per-area overviews: `backend/`, `database-schema/`, `frontend/`,
  `indexing-pipeline/`, `infrastructure/`, `xdr-parsing/`.

Since then, ADRs 0022 through 0031 have reshaped substantive parts of the
system:

- Schema surface: 0022 (schema correction), 0023 (typed token metadata),
  0024 (BYTEA hashes), 0025 (final schema v1), 0026 (accounts surrogate),
  0027 (post-surrogate schema), 0030 (contracts surrogate), 0031 (enum
  SMALLINTs).
- Ingest / persist path: 0027 (14-step `persist_ledger`), 0028 / 0029
  (parsed artifact, then abandoned in favour of read-time XDR fetch).

The research note on `tokens` vs `assets` (now at
[`lore/1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md`](../1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md),
§5.2) documents concrete drift
examples for the schema surface alone:

- `asset_type` has 4 values in the migration (`native, classic, sac,
soroban`), the design doc describes 3.
- `VARCHAR(20)` in reality, `VARCHAR(10)` in the doc.
- Partial unique indexes per `asset_type` in reality, plain `UNIQUE` in
  the doc.
- `ck_tokens_identity` CHECK constraint in reality, not mentioned in
  the doc.
- Analogous drift for `transaction_hash_index`, `transaction_participants`,
  `wasm_interface_metadata`, `lp_positions`, `nft_ownership`,
  `account_balances_current` / `account_balances_history`.

No reason to believe the other architecture files (backend, indexing
pipeline, infrastructure, etc.) are in better shape — they were all
written at the same iteration.

The question is not "are the docs wrong" (we know they are), but "do we
want them to be right, and if so, how do we sustain that".

## Decision

**Adopt `docs/architecture/**` as evergreen, living documentation of the
current system state.\*\* From this ADR onward:

1. **Any ADR that changes schema, API contract, ingest pipeline,
   infrastructure, or a major subsystem owns keeping the corresponding
   `docs/architecture/**` pages in sync.\*\* The PR that lands the ADR (or
   the task that implements it) updates the relevant overview files.
2. **One-shot catch-up.** Task 0155 performs a single backward-looking
   sweep: walk ADRs 0022–0031 in order, compare each to the current
   state of `docs/architecture/**`, update the docs to match. After this
   sweep, the docs represent the system _as of today_.
3. **Steady state.** Every future ADR PR checklist gains a line: _"If
   this ADR changes something visible in `docs/architecture/**`, the
   affected file(s) are updated in this PR."_ When that applies but is
   impractical (large sweep), the ADR spawns a dedicated DOCS task, and
   the ADR's `related_tasks` references it.
4. **Scope of "architecture docs".** Everything under
   `docs/architecture/**`. The research notes in `docs/` root (e.g.
   the tokens-vs-assets taxonomy note now under task 0154's `notes/`)
   stay as research snapshots;
   ADRs stay as immutable historical decisions; CLAUDE.md files stay
   under their own rules. This ADR is about the `architecture/`
   subtree.
5. **Level of detail.** These docs describe the _current_ system with
   enough fidelity for a new team member to onboard. They do not need
   to reproduce every migration or every ADR verbatim — they describe
   state and link out to ADRs for the _why_.

## Rationale

1. **The drift is already material.** The tokens-vs-assets research note
   found four concrete mismatches in one section of one file. The other
   sections and other files almost certainly have similar density. New
   developers onboarding against these docs will encode stale mental
   models; every future conversation will pay a correction tax.
2. **ADRs are the source of truth for _why_, not _what_.** ADRs
   accumulate chronologically and are intentionally immutable — reading
   ten of them to reconstruct the current schema is not a realistic
   onboarding path. Living docs that say "the schema is X today, see
   ADRs 0024, 0026, 0030, 0031 for how we got here" cost less per
   reader and age better.
3. **The cost of keeping them current is bounded.** Most ADRs touch a
   small, well-scoped area. Updating the relevant overview section in
   the same PR that lands the ADR is 15–60 minutes of work against a
   compounding benefit. The up-front catch-up (task 0155) is the
   expensive step; steady-state maintenance is cheap.
4. **It aligns with how we already use other docs.** `CLAUDE.md` files
   are evergreen, task and ADR templates are evergreen, the lore
   session files are evergreen. Having `docs/architecture/**` alone be
   frozen-in-time is inconsistent.
5. **It unblocks downstream work.** Task 0154 (tokens → assets rename)
   needs accurate schema docs to land cleanly. Without this decision,
   0154 would either touch docs opportunistically (ad-hoc, uneven), or
   skip them (widening the gap). Making the policy explicit means every
   future refactor lands in a consistent environment.

## Alternatives Considered

### Alternative 1: Keep `docs/architecture/**` frozen

**Description:** Treat the existing docs as a design-time artefact — "this
is what we set out to build in the Soroban-first iteration". Accept that
they will diverge from reality over time. Onboarding happens primarily
via code + ADRs + CLAUDE.md files.

**Pros:**

- Zero process overhead per ADR.
- Historical snapshot remains clean; comparing current state to
  original intent is easier when the docs don't move.

**Cons:**

- Drift keeps compounding silently. Every new ADR makes the gap larger.
- Onboarding against stale docs teaches wrong mental models that then
  have to be un-taught.
- Research notes like the tokens-vs-assets one keep having to do the
  job of "here's the real state" because the canonical place doesn't.
- Refactors that touch schema or pipeline (0154 being the near-term
  example) have to choose between updating the docs opportunistically
  or ignoring them — neither produces good outcomes.

**Decision:** REJECTED — the drift cost has already materialised and
compounds.

### Alternative 2: Replace `docs/architecture/**` with a generated

snapshot from ADRs

**Description:** Treat the ADR trail as the canonical source and
generate architecture docs from it (either mechanically or via AI
summarisation on demand).

**Pros:**

- No manual docs-upkeep step per ADR.
- Always consistent with ADRs by construction.

**Cons:**

- ADRs are reasoning-heavy ("why did we choose SMALLINT over PG enum")
  and do not form a readable _description_ of the current system.
  Synthesising a coherent overview from 10+ ADRs on every request is
  slow and expensive.
- Generated docs are harder to review in PRs (diff noise dominates).
- Loses the editorial curation that an overview page benefits from —
  diagrams, ordering of topics, glossaries.

**Decision:** REJECTED — ADRs and architecture overviews serve different
audiences (decision history vs. current-state onboarding); collapsing
them loses both.

### Alternative 3: Move architecture docs into a wiki / separate site

**Description:** Take them out of the repo, host as a Confluence / Notion /
MkDocs site, update there.

**Pros:**

- Richer editing surface (diagrams, cross-linking, search).
- Lower-friction edits for non-engineers.

**Cons:**

- Drift from code gets worse, not better, when docs move away from the
  PR review path.
- Loses PR-review gate for doc changes — no reviewer will catch "ADR
  changed schema, docs didn't".
- Adds an external tool dependency for onboarding.

**Decision:** REJECTED — the very property we want is that docs live
next to the code and move through the same PR review as the ADRs that
drive them.

## Consequences

### Positive

- New team members onboard against docs that match reality.
- Research notes stop having to duplicate "current state" descriptions —
  they can link to the architecture overview and focus on the question
  at hand.
- Task 0154 (tokens → assets) lands against a clean docs baseline.
- ADR review becomes richer — reviewers can check that the
  corresponding architecture update is coherent, catching half-baked
  changes early.

### Negative

- Every ADR PR (when applicable) now owns a docs update. Estimated
  15–60 minutes of additional work per ADR.
- One-shot audit (task 0155) is a 1–3 day sweep by one person, depending
  on how much drift the audit uncovers.
- Risk of the docs update becoming a rubber-stamp ("yes, updated") — to
  mitigate, code review checks the doc diff like any other diff.

### Follow-ups

- Task 0155 — ADR-history audit of `docs/architecture/**`. Sweeps ADRs
  0022–0031 against current doc state and brings the docs up to date.
- Update the ADR / task templates: add a "Docs updated?" checkbox to
  the acceptance criteria / PR checklist (captured in task 0155 as
  part of the process formalisation).
- Next time a doc section grows unwieldy, consider splitting by ADR
  range or by subsystem — but not until we see how the refreshed docs
  settle.

## References

- [ADR 0027](0027_post-surrogate-schema-and-endpoint-realizability.md) — most recent schema snapshot that the docs should already reflect
- [ADR 0030](0030_contracts-surrogate-bigint-id.md) — contracts surrogate
- [ADR 0031](0031_enum-columns-smallint-with-rust-enum.md) — SMALLINT enum flip
- [Task 0154 research note: Assets vs Tokens](../1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md) — the note that surfaced concrete drift and motivated this ADR
