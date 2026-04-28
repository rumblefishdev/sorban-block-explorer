# audit-harness

Continuous data-correctness audit for the Soroban block explorer DB.
Phase 1 of task 0175 — pure SQL invariants, no external dependency.

## What this is

A library of per-table SQL invariants that surface internal consistency
bugs in the explorer DB without calling Horizon, Soroban RPC, or
re-parsing archive XDR. Each invariant is a single SELECT that returns
`(violations, sample)`; zero violations = green.

This is the **automated complement** to Filip's PR-driven manual audits
(0167, 0172, 0173). Filip catches subtle deep bugs at N=6 hand-picked
transactions; this harness catches systematic divergences at N=all-rows
across all 17 tables. Findings funnel into the same bug-task pipeline.

Future phases (separate PRs):

- **2a** — DB vs Horizon API diff for classic tables
- **2b** — DB vs Soroban RPC diff for Soroban tables
- **2c** — DB vs raw archive XDR re-parse (ground truth)
- **3** — Aggregate sanity for continuous monitoring

## Usage

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
    crates/audit-harness/run-invariants.sh

# With a saved report
crates/audit-harness/run-invariants.sh --out /tmp/audit-2026-04-28.md
```

Exit code is `0` if every invariant returned `0 violations`, `1` if any
violation was found, `2` for runtime errors (DB unreachable, missing
psql, etc.).

## Layout

```
crates/audit-harness/
├── README.md                  # this file
├── run-invariants.sh          # bash runner — iterates ./sql/*.sql
└── sql/
    ├── 01_ledgers.sql
    ├── 02_transactions.sql
    ├── 03_transaction_hash_index.sql
    ├── 04_operations_appearances.sql
    ├── 05_transaction_participants.sql
    ├── 06_soroban_contracts.sql
    ├── 07_wasm_interface_metadata.sql
    ├── 08_soroban_events_appearances.sql
    ├── 09_soroban_invocations_appearances.sql
    ├── 10_assets.sql
    ├── 11_accounts.sql
    ├── 12_account_balances_current.sql
    ├── 13_nfts.sql
    ├── 14_nft_ownership.sql
    ├── 15_liquidity_pools.sql
    ├── 16_liquidity_pool_snapshots.sql
    ├── 17_lp_positions.sql
    └── 18_partition_routing.sql
```

Each file is self-contained — run any individually with `psql -f` to
debug a specific invariant.

## Output

Markdown-flavoured. Each file emits a `## <table>` header followed by
`### I<n> — <name>` sections. The runner prepends a timestamp banner and
appends a summary of row counts per table.

Exit-code aggregation is conservative: any non-zero `violations` count
across any invariant returns `1` from the runner.

## Coverage matrix (Phase 1)

| Table | Invariants | Highlights |
| --- | --- | --- |
| `ledgers` | 4 | sequence contiguity, hash UNIQUE, closed_at monotonic |
| `transactions` | 6 | hash UNIQUE, op_count vs appearances, FK to ledgers + accounts |
| `transaction_hash_index` | 4 | bidirectional FK with `transactions`, hash UNIQUE |
| `operations_appearances` | 6 | composite FK to transactions, FK to accounts/contracts/pools |
| `transaction_participants` | 3 | composite FK + uniqueness |
| `soroban_contracts` | 6 | StrKey shape, FK to deployer + wasm_hash → wasm_interface_metadata |
| `wasm_interface_metadata` | 3 | wasm_hash 32 bytes, JSONB shape |
| `soroban_events_appearances` | 4 | composite FK, ledger_seq matches parent tx |
| `soroban_invocations_appearances` | 6 | composite FK, ledger_seq, caller_id, event-coverage info |
| `assets` | 6 | ck_assets_identity per ADR 0038, native singleton, FK |
| `accounts` | 4 | StrKey shape (G/M tolerated pre-unwrap), monotonic seen ledgers |
| `account_balances_current` | 6 | partial uidx native vs credit, balance ≥ 0 |
| `nfts` | 5 | (contract_id, token_id) UNIQUE, current_owner ↔ last `nft_ownership` |
| `nft_ownership` | 6 | composite FK, mint-precedes-transfer, event_type enum |
| `liquidity_pools` | 5 | pool_id 32B, asset_a < asset_b ordering, fee_bps in [0,10000] |
| `liquidity_pool_snapshots` | 5 | FK + non-negative reserves + (pool_id, ledger_seq) UNIQUE |
| `lp_positions` | 6 | shares ≥ 0, sum of active ≈ snapshot.total_shares (within tol) |
| **partition_routing** | 3 | rows in `_default` should be 0; per-parent child count sanity |

## Notes for operators

- **Some violations are expected on dev DBs** with integration-test
  fixtures (synthetic StrKeys like `CAAA…FLTRNFT`, `GAAA…AAASRC`).
  These are not production data drift. Run against a freshly-restored
  staging or a clean docker DB after a real backfill for meaningful
  results.
- **Backfill-mid run skews counts** — the harness reads instantaneous
  state, so partial restores produce noisy reports. Pause backfill
  before running for a clean baseline.
- **Phase 1 is intentionally local-only.** External-source diffs ship
  in Phases 2a/2b/2c; the bash runner here handles Phase 1 only.
- **A non-zero exit does not always mean broken data.** Read the
  per-section violation counts and the sample to triage. Genuine
  bugs become bug tasks via the standard 0167-style spawn flow.

## Future improvements

- Output structured JSON (currently markdown-style for human reading;
  CI integration would benefit from machine-parseable output)
- Per-invariant skip flags (`--skip 02:I2`) for triaging false positives
- Re-implementation in Rust with a single binary if the bash + psql
  pipeline becomes a bottleneck
- Phase 2/3 tooling (separate binary likely)
