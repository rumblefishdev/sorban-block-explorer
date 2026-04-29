# Manual endpoint audit — runbook

Endpoint-by-endpoint manual cross-check of the public REST API against
real-world Stellar explorers, Horizon, and the local DB. Companion to
the automated audit harness in [`crates/audit-harness/`](../../crates/audit-harness/) —
not a replacement.

> **Status:** evergreen runbook. Originated from Filip Mazur's manual
> audit methodology (driver of bugs 0167 → 0168/0169/0170/0172/0173).
> Promoted to wiki so the same approach is reproducible by any team
> member without re-deriving the steps.

## Why both manual and automated

| Approach                        | Catches                                                                                                              | Misses                                                                   |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| **Manual (this runbook)**       | Response-shape drift vs frontend-overview, subtle protocol/parsing bugs, multi-source agreement                      | Doesn't scale beyond a few rows per endpoint                             |
| **Automated (`audit-harness`)** | Wide systematic divergences (FK / partition routing / hash extraction class), N=50+ samples, internal SQL invariants | Single-source diff, doesn't audit response shape, no UX-level eyeballing |

Both are needed. Manual finds _new_ bug classes; automated locks them
once a regression test exists.

## Pre-reading (do once per session)

1. [`docs/architecture/database-schema/endpoint-queries/`](../../docs/architecture/database-schema/endpoint-queries/) — the canonical SQL per public endpoint
2. [`docs/architecture/database-schema/endpoint-queries/run_endpoint.sh`](../../docs/architecture/database-schema/endpoint-queries/run_endpoint.sh) — runner with per-endpoint sample inputs
3. [`docs/architecture/database-schema/endpoint-queries/README.md`](../../docs/architecture/database-schema/endpoint-queries/README.md) — per-endpoint response-shape map
4. [Task 0167](../1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md) — origin of the SQL set, audit pattern, and known caveats (E5 S3 bridge, E14 archive XDR overlay, etc.)
5. [`docs/architecture/frontend/frontend-overview.md`](../../docs/architecture/frontend/frontend-overview.md) §6 — what the frontend actually renders per page

## External sources

Cross-check in this priority order, picking whichever has the data:

| Source              | URL                                       | Coverage                                       | Use when                                                           |
| ------------------- | ----------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------ |
| **stellarchain.io** | `https://stellarchain.io/`                | classic + Soroban, deep history                | First choice for Soroban-heavy endpoints                           |
| **stellar.expert**  | `https://stellar.expert/explorer/public/` | classic + Soroban, deep history, rich metadata | When stellarchain doesn't render the field you need (e.g. raw_xdr) |
| **Horizon API**     | `https://horizon.stellar.org/`            | classic only (no Soroban indexing)             | Fallback for ledgers / accounts / classic tx; programmatic JSON    |
| **Soroban RPC**     | gateway.fm / SDF                          | Soroban only, ~24h retention                   | Avoid for retroactive checks; OK for live ingest                   |
| **Local DB**        | `psql ... soroban_block_explorer`         | full local ground truth                        | When you need to inspect the row that produced the response        |

## Per-endpoint workflow

For each endpoint `NN` (start at the first one not yet audited):

### Step 1 — Run the endpoint SQL

```bash
cd docs/architecture/database-schema/endpoint-queries
./run_endpoint.sh NN
```

Confirms the SQL parses + returns rows. Capture stdout for later
comparison. Note the row count, column shape, sample values.

### Step 2 — Compare response shape vs frontend spec

Open [`frontend-overview.md`](../../docs/architecture/frontend/frontend-overview.md)
to the section that consumes endpoint `NN` (typically §6.NN-ish). Verify:

- Every field the frontend section names is present in the SQL output
- Every field the SQL output emits is referenced by the frontend section
- Optional fields (`null`-tolerant) are flagged as such

Drift in either direction is a finding worth recording. Examples found
this way:

- `memo` / `memo_type` in `TransactionListItem` — present in DTO + SQL,
  absent from frontend spec for E2 list. See task 0046 + ADR review.

### Step 3 — Pick N random rows

`N = 2-3` is the manual-audit sweet spot. Pick by hand — bias toward
diversity (different ledger ranges, different op types, fee-bump vs
plain, mainnet-prominent contracts).

### Step 4 — Cross-check on real explorers

For each picked row, open the corresponding entity on **two** explorers
(picking from the priority table above). Compare every field returned
by the SQL to what the explorer renders.

Use the local DB (`psql`) when:

- An explorer doesn't surface the field (rare — usually means the
  field is internal-only, valid omission)
- The explorers disagree (rare — but flag the disagreement explicitly)
- A value looks wrong but you can't tell which side has the bug
  (compare to archive XDR via `audit-harness` Phase 2c)

### Step 5 — Stop, report findings, hand back

Don't move to endpoint `NN+1` automatically. Write up:

- ✓ fields that match across DB + 2 explorers + frontend spec
- ⚠ drift (differs but explainable — drift between snapshot and
  current, optional-field omissions)
- ✗ mismatches (one source disagrees with the other two — almost
  always a bug in that source; if it's our DB, spawn a bug task)

Then **stop**. The user runs the same audit locally to confirm,
typically on real wallet StrKeys / contract IDs they care about.
After they confirm, move to endpoint `NN+1`.

## Bug task spawning

When a mismatch is real and reproducible, spawn a backlog bug task
following the existing 017N pattern:

```yaml
---
id: 'NNNN'
type: BUG
status: backlog
related_tasks: ['0175'] # this audit harness, since manual-endpoint-audit complements it
tags: [audit-driven, layer-<parser|persist|api>]
---
```

Body must include:

- Reproduction steps (exact endpoint, exact row, exact explorer URL
  used as ground truth)
- Hypothesis on root cause (which extraction / persist site)
- Acceptance criteria (re-running the audit returns 0 mismatches on
  the affected rows)

Existing audit-driven bugs to read for shape: [0167](../1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md),
[0168](../1-tasks/archive/0168_BUG_envelope-tx-processing-misalignment.md),
[0169](../1-tasks/archive/0169_BUG_inner-tx-hash-and-amount-semantics-audit.md),
[0173](../1-tasks/active/0173_BUG_xdr-parser-missing-per-operation-events-v4-meta.md),
[0176](../1-tasks/backlog/0176_BUG_ledger-hash-extraction-wrong.md),
[0177](../1-tasks/backlog/0177_BUG_muxed-account-leak-into-persist.md),
[0178](../1-tasks/backlog/0178_BUG_contract-strkeys-leak-into-accounts-table.md),
[0179](../1-tasks/backlog/0179_BUG_lp-asset-canonical-order-violated.md).

## Tips and traps

- **Apply-order vs hash-order.** `tx_processing` is in apply order;
  `tx_set` is in hash order (CAP-0063). Don't compare by index across
  the two — always compare by hash. See task 0168 for the canonical
  fix in `extract_envelopes`.
- **MuxedAccount surface.** Stellar source/destination/from fields are
  `MuxedAccount`. Our DB stores the underlying ed25519 G-key per
  ADR 0026; explorers may render either form (stellar.expert often
  shows G even when on-wire is M). When you see a 69-char M-key in
  `accounts.account_id`, that's task 0177 and a real bug.
- **Snapshot vs current state.** `account_balances_current.balance` is
  our snapshot at `last_updated_ledger`; Horizon shows current state.
  They will disagree on active accounts — that's drift, not a bug.
  Re-fetch Horizon at the same ledger only via archive XDR if you
  need a time-anchored cross-check.
- **Soroban events for V4 meta.** Per task 0173, our parser drops
  per-op events for Protocol-23+ V4 meta. Until that fix lands,
  expect ~91% of `soroban_invocations_appearances` rows to lack a
  matching event row. Don't spawn duplicate bugs for this until
  0173 closes.
- **Test residue.** A handful of `GAAA…` and `CAAA…` synthetic
  StrKeys exist in the DB from integration-test fixtures. Skip
  them in samples — they're not real on-chain entities.

## Cross-link with the automated harness

After every manual run, if you flag a new bug, also run the relevant
automated harness phase against the same sample to capture a
machine-readable artifact:

```bash
# Phase 1 SQL invariants — table-wide check
DATABASE_URL=postgres://... \
  crates/audit-harness/run-invariants.sh \
  --out crates/audit-harness/reports/$(date +%Y-%m-%d)-phase1-<dataset>.md

# Phase 2a Horizon diff — bulk per-table cross-check
cargo run -q -p audit-harness --bin horizon-diff -- \
  --table <table> --sample 50 --concurrency 6

# Phase 2c archive XDR re-parse — ground truth
cargo run -q -p audit-harness --bin archive-diff -- \
  --table ledgers --sample 50 --concurrency 4
```

Save the output under [`crates/audit-harness/reports/`](../../crates/audit-harness/reports/)
following the naming convention in that directory's README. The two
together — manual finding + automated regression check — is what makes
a bug closure stick.

## See also

- [`backfill-execution-plan.md`](backfill-execution-plan.md) — when audit fits
  in the cutover sequence
- [`partition-pruning-runbook.md`](partition-pruning-runbook.md) — sister
  operational runbook
- [Task 0175](../1-tasks/active/0175_FEATURE_audit-harness-backfill-correctness.md) — the automated harness this complements
