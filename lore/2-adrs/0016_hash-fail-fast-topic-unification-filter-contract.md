---
id: '0016'
title: 'Hash fail-fast, topic0 pre-GA unification, topic0 filter contract'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0013', '0014', '0015']
tags:
  [database, schema, stellar, hash-integrity, soroban-events, ingest, migration]
links: []
history:
  - date: 2026-04-19
    status: proposed
    who: fmazur
    note: 'ADR created — corrective revision of ADR 0015 after internal-consistency review'
---

# ADR 0016: Hash fail-fast, `topic0` pre-GA unification, `topic0` filter contract

**Related:**

- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0015: Hash index, typed topic0, migration honesty, CHECK policy](0015_hash-index-topic-typing-migration-honesty.md)

---

## Status

`proposed` — corrective delta on top of ADR 0015. Does not supersede 0015 in
full; overrides ADR 0015 only on the three points listed below. Every other
decision in ADR 0011–0015 stands.

---

## Context

A second-pass review of ADR 0015 surfaced three places where the document is
internally inconsistent or states a guarantee stronger than the schema
actually provides:

1. **`transaction_hash_index` conflict semantics are self-contradictory.**
   ADR 0015 claims DB-level enforcement of global hash uniqueness and
   simultaneously specifies `ON CONFLICT (hash) DO NOTHING` on inserts. Those
   two statements cannot both be true: `DO NOTHING` silently suppresses the
   conflict that would otherwise enforce the constraint. Either the insert
   fails on conflict (hard enforcement) or it silently dedupes (mitigation).
   The current wording lets both meanings coexist and obscures which model is
   in effect.

2. **`topic0` historical rows are left in a dual format.** ADR 0015 changes
   parser output to the typed `"{type_code}:{value}"` form but marks
   re-ingest of pre-existing rows as optional. The practical result: a query
   `WHERE topic0 = 'sym:transfer'` returns rows ingested after the ADR but
   misses semantically identical rows ingested before it. Filter results
   depend on ingestion era — a silent correctness bug that is also
   observable as inconsistent dashboards across ledger ranges.

3. **`topic0` prefix/LIKE behavior is asserted without the supporting
   DDL.** ADR 0015 implies prefix `LIKE` queries on `topic0` are index-backed
   by the existing B-tree `idx_events_topic0`. With Postgres's default
   collation-aware B-tree, `LIKE 'prefix%'` is not optimized — prefix scan
   requires either `"C"` collation or the `text_pattern_ops` operator class.
   The ADR text implies a capability the DDL does not actually provide.

This ADR resolves all three. No new tables. No new indexes beyond what's
needed to resolve the explicit gap or removed. The `transaction_hash_index`
from ADR 0015 stays — its justification is re-evaluated and held.

---

## Decision

### Summary of decisions

- **`transaction_hash_index` inserts are fail-fast. No `ON CONFLICT` clause.**
  Any hash conflict aborts the whole ledger-group commit and surfaces as an
  ingest error. Ingest idempotency for retry scenarios is handled upstream
  by an explicit "has this ledger already been committed?" check before the
  transaction starts, not by swallowing the conflict inside it.
- **All existing `soroban_events.topic0` rows are re-ingested** as a
  one-time pre-GA migration. After the migration, the column contains
  exclusively typed `"{type_code}:{value}"` form. No dual-format period.
  Query semantics become era-independent.
- **`topic0` filter contract is equality-only.** `filter[topic0]=sym:transfer`
  is supported; `LIKE 'sym:%'` and other prefix patterns are explicitly
  outside the contract and not index-backed. No new operator-class index is
  added. This is a documented capability limit, not a hidden weakness.

### `transaction_hash_index` — fail-fast integrity (fix #1)

**Decision:**

- Remove `ON CONFLICT` from all `transaction_hash_index` inserts. Parser
  issues plain `INSERT INTO transaction_hash_index (...) VALUES (...)`. A
  conflict on `hash` raises `unique_violation`, which propagates up and
  aborts the ledger-group transaction.
- Ingest retries are handled by a **ledger-level idempotency check at the
  start of each ledger-group transaction**, before any INSERT is issued:

  ```sql
  -- Runs in the Rust ingest pipeline, read-committed, before BEGIN of
  -- the write transaction. Cheap single-row lookup on PK.
  SELECT EXISTS (SELECT 1 FROM ledgers WHERE sequence = $ledger_sequence);
  ```

  If the row exists, the ingest worker skips the ledger entirely. If not,
  it starts the write transaction and issues straight INSERTs with no
  `ON CONFLICT`.

- **The ledger-group commit is one SQL transaction per ledger.** Postgres
  atomicity guarantees all-or-nothing: if any child INSERT fails, `ledgers`
  row insertion is also rolled back. Therefore `ledgers.sequence = N`
  existence is a reliable "ledger N is fully committed" marker for the
  upstream idempotency check.
- **Hash conflict semantics:** if two distinct `(ledger_sequence, created_at)`
  pairs claim the same `hash` — which is a parser bug per Stellar's
  `Transaction::getContentsHash` determinism — the INSERT fails with
  `unique_violation`. The ingest worker logs, alerts, and does not proceed
  past the affected ledger. The operator investigates before resuming. This
  is the correct failure mode: a parser producing inconsistent hashes is a
  data-integrity emergency, not a retry condition.

**Other tables' `ON CONFLICT DO NOTHING` semantics are unchanged.** The
`transactions`, `operations`, `soroban_events`, etc. retain ADR 0013's
`ON CONFLICT` clauses. They are **defensive idempotency for the same
ledger-group retry path** — under normal operation with the upstream
`ledgers` existence check, they never fire. They exist for defense in depth
against a bug in the idempotency check itself. Only `transaction_hash_index`
is fail-fast because only `transaction_hash_index` is the integrity
boundary for hash uniqueness. The split is deliberate and spelled out.

### `topic0` historical unification (fix #2)

**Decision:**

- **One-time mandatory pre-GA migration:** all ledgers containing
  `soroban_events` rows written under ADR 0014 canonicalization rules are
  re-ingested via the existing re-ingest protocol. Parser outputs typed
  `"{type_code}:{value}"` form for every row.
- **No dual-format transitional period.** After migration, the column
  contains exclusively typed form. Query semantics are era-independent.
- **No API filter change is needed** to bridge the migration; there is no
  bridge — re-ingest eliminates the old format from storage entirely.
- Pre-GA scope makes this cheap: the number of ledgers that were ingested
  under ADR 0014's rules is bounded (tens to hundreds in a dev/testnet
  environment, not millions). Re-ingest cost is hours of wall-clock at
  worst, run once, acceptable under pre-GA migration window.
- **Greenfield deployments** (new installations adopting ADR 0015/0016
  together) have no historical data and skip the migration step.

### `topic0` filter contract — equality only (fix #3)

**Decision:**

- `soroban_events.topic0` supports **equality match only** in API filters.
  `filter[topic0]=sym:transfer` is index-backed by `idx_events_topic0`.
- **Prefix match (`LIKE 'sym:%'`, `LIKE 'addr:GABC%'`, etc.) is explicitly
  outside the API contract.** It will not be supported by the index and
  will not be exposed as a filter operator. Queries attempting it degrade
  to sequential scan and should not be issued from the API layer.
- **No new index is added.** No `text_pattern_ops` variant, no `"C"`
  collation change. The existing
  `idx_events_topic0 (contract_id, topic0, created_at DESC)` stays as-is
  and serves the equality-only contract optimally.
- If a future endpoint requires "all events with topic0 of type X" as a
  first-class feature, adding an index then is straightforward — but such
  an endpoint is not in the current API surface and is not anticipated.
  Deferring the capability matches the "no projecting for hypothetical
  future needs" principle.
- This is an API contract decision, not a schema limitation. The column
  can hold any typed-form string; the query planner just won't optimize
  prefix patterns without a matching index.

---

## Detailed schema changes

### DDL changes

**None.** All three fixes are resolved without changing table shapes,
column types, constraints, or indexes.

### Non-DDL contracts changed

| Contract                        | Before (ADR 0015)                         | After (this ADR)                                            | Artifact                     |
| ------------------------------- | ----------------------------------------- | ----------------------------------------------------------- | ---------------------------- |
| `transaction_hash_index` insert | `ON CONFLICT (hash) DO NOTHING`           | plain `INSERT` (fail-fast)                                  | parser code                  |
| Pre-transaction idempotency     | implicit (`ON CONFLICT` swallows retries) | explicit `SELECT EXISTS FROM ledgers` before `BEGIN`        | parser code                  |
| Hash conflict response          | silent no-op                              | `unique_violation` → ledger-group rollback + operator alert | parser code + runbook        |
| `topic0` historical rows        | dual format (old canonical + new typed)   | single format (typed only) after pre-GA re-ingest           | migration step               |
| `topic0` API filter contract    | ambiguous (equality + implied prefix)     | equality-only, explicit                                     | API docs + endpoint handlers |

### Inserts, exactly

Parser flow per ledger (fail-fast model):

```text
-- Outside transaction, pre-check:
SELECT EXISTS (SELECT 1 FROM ledgers WHERE sequence = $N);
  -- if true → skip ledger entirely; return

-- Otherwise:
BEGIN;
  INSERT INTO ledgers          (...) VALUES (...);                      -- no ON CONFLICT
  INSERT INTO accounts         (...) VALUES (...) ON CONFLICT DO NOTHING;  -- defensive
  INSERT INTO soroban_contracts(...) VALUES (...) ON CONFLICT DO NOTHING;  -- defensive
  ... (tokens, nfts, liquidity_pools)
  INSERT INTO transactions     (...) VALUES (...);                      -- no ON CONFLICT
  INSERT INTO transaction_hash_index (hash, ledger_sequence, created_at)
       VALUES (...);                                                    -- no ON CONFLICT — HARD
  INSERT INTO operations       (...) VALUES (...) ON CONFLICT DO NOTHING;  -- defensive
  ... (events, invocations, participants, transfers, nft_ownership,
       liquidity_pool_snapshots, lp_positions,
       account_balances_current, account_balance_history)
COMMIT;
```

**Why `transactions` itself is also no-`ON CONFLICT`:** the same reasoning
as `transaction_hash_index`. After the ledgers-existence check, a
conflict on `UNIQUE (hash, created_at)` during the transaction is a parser
bug. Let it surface. `ON CONFLICT` on `transactions` was defensive before;
it becomes a bug-hider now. Remove it.

**Why the defensive `ON CONFLICT DO NOTHING` stays on child tables:**
these are extra belt against the upstream check being wrong. Under correct
operation they never fire. If the ledgers-existence check has a bug and
misses a duplicate ingest, the child `ON CONFLICT` still dedupes safely.
The cost of keeping it is zero; removing it creates a larger blast radius
for an idempotency bug.

### Re-ingest protocol update

ADR 0015's re-ingest protocol adds the `DELETE FROM transaction_hash_index
WHERE ledger_sequence = $1`. That stays. No change in this ADR.

---

## Rationale

### Why fail-fast on `transaction_hash_index`

The whole point of introducing `transaction_hash_index` in ADR 0015 was
**to turn hash uniqueness from invariant-plus-monitoring into DB-enforced
integrity**. `ON CONFLICT DO NOTHING` undoes exactly that — it restores the
invariant-plus-monitoring model, now with an extra 50 GB table to maintain.
If we accept `ON CONFLICT DO NOTHING`, we should instead revert the
hash-index decision entirely and fall back to ADR 0014's monitoring query.

The hash index is justified only if it fails on conflict. The fix is to
actually let it fail.

Retry tolerance is not lost by removing `ON CONFLICT`. It is moved to a
different layer — the upstream `ledgers` existence check — where it
semantically belongs. "Has this ledger been committed?" is a ledger-level
question; the DB-level unique constraint on `hash` is a row-level integrity
question. Conflating them in `ON CONFLICT` hid that distinction.

### Why defensive `ON CONFLICT` stays elsewhere

Two layers of idempotency:

- **Semantic layer** (upstream): "skip if ledgers[N] exists." Handles retry.
- **Defense-in-depth layer** (`ON CONFLICT DO NOTHING` on child tables):
  catches any bug in the semantic layer without producing duplicates.

The cost of defense-in-depth is zero (INSERTs never conflict in correct
operation). The benefit is bounded blast radius if the semantic layer has
a subtle bug. Removing it for aesthetic consistency trades a real safety
property for no gain.

`transaction_hash_index` is different because it's the single source of
truth for hash uniqueness. `ON CONFLICT DO NOTHING` there would hide the
bug we explicitly want to surface — a parser producing inconsistent hashes.

The split is not cosmetic. It's the difference between integrity
enforcement (must fail loudly) and idempotency tolerance (should fail
quietly).

### Why mandatory topic0 re-ingest, not transitional dual-format

Dual-format storage makes query results depend on ingestion era. That is a
silent correctness bug with no upper bound on its surface: every
`topic0` filter potentially returns partial data. The only honest fix is
one of:

1. Query-time polyfill: API layer expands `filter[topic0]=sym:transfer` to
   match both old-canonicalized and new-typed forms. This is fragile,
   schema-coupled, and creates per-ScVal-type branching in the query path
   forever.
2. Mandatory re-unification of storage. One-time cost, then the problem is
   gone forever.

We are pre-GA. Re-ingest is a measurable operational cost (hours), not a
permanent tax (every filter, every future ScVal type, every future
developer confused by the inconsistency). Option 2 wins cleanly.

### Why equality-only filter on topic0

The endpoint surface — `GET /contracts/:contract_id/events` — does not
expose a `filter[topic0_prefix]` operator. Nothing in the documented
endpoints requires prefix matching on `topic0`. Committing to prefix
semantics in the ADR when the endpoint contract doesn't require it is
exactly the "projecting for hypothetical future needs" pattern this
project avoids.

Adding `text_pattern_ops` or `"C"` collation would require:

- a second index (added storage + write cost),
- a new API operator (new surface),
- new tests (new permutations),

for zero endpoint that requires it. Equality-only is the honest
commitment: it matches the endpoints, the index we have, and the
"minimalism" principle.

If a future API requires prefix search, this ADR can be overridden by a
follow-up that (a) names the endpoint, (b) adds the index, (c) documents
the new contract. Adding the capability prospectively without the
motivating endpoint would be speculation.

---

## Consequences

### Stellar/XDR compliance

- **Positive.** Hash fail-fast aligns the DB behavior with Stellar's
  deterministic `Transaction::getContentsHash` guarantee: the protocol
  says hashes are globally unique, and now the schema enforces it rather
  than hopes.
- **Positive.** Topic0 unification ensures ScVal type fidelity is
  uniform across historical data, matching what the protocol encodes in
  `ContractEvent.topics`.
- **Neutral.** Equality-only filter contract doesn't change what Stellar
  guarantees; it documents what our endpoint offers.

### Database weight

- **No new tables, no new indexes, no new columns.**
- `transaction_hash_index` unchanged (still ~50 GB at mainnet). Its
  justification (hash integrity + `/transactions/:hash` routing)
  **strengthens** under this ADR because it now actually enforces what it
  was introduced to enforce.
- Principle "lightweight bridge DB" preserved. No drift from ADR 0015's
  footprint.

### History correctness

- **Improved.** Topic0 filtering on historical events returns correct
  results independent of ingestion era. Hash uniqueness is a real
  property, not a monitored hope.
- **No regression.** All history reconstruction paths from ADR
  0012/0013/0014/0015 unchanged.

### Endpoint performance

- **`GET /transactions/:hash`:** unchanged vs ADR 0015 — still a single
  PK probe + partition-pruned secondary query.
- **`GET /contracts/:id/events` with `filter[topic0]`:** unchanged —
  equality match remains index-backed.
- **No endpoint degraded.** The equality-only contract matches actual
  endpoint needs.

### Ingest simplicity

- **Small refinement.** Parser adds one SELECT at the start of each ledger
  (cheap PK lookup) and removes `ON CONFLICT` from two insert sites
  (`transactions`, `transaction_hash_index`). Net code complexity
  approximately unchanged.
- **Error surface becomes visible.** A hash conflict now alerts an
  operator instead of silently landing in a monitoring query. This is
  better; silent data corruption is the worst failure mode.

### Replay / re-ingest risk

- **Reduced.** Re-ingest protocol is unchanged (DELETE cascade + explicit
  `DELETE FROM transaction_hash_index`). Hash fail-fast means any replay
  that would produce conflicting rows fails loudly instead of producing
  silent duplicates.

### Operational cost

- One-time pre-GA `topic0` re-ingest: bounded by the number of affected
  ledgers, measured in hours of wall-clock.
- Runbook update: "hash conflict = parser bug alert" added as an
  operational response. Zero cost outside the alert path.
- No new monitoring, no new infrastructure.

### Consistency of historical-data contract

- **Uniform after migration.** No "pre-ADR-0016" vs "post-ADR-0016"
  query differences after the one-time re-ingest completes.
- This is the explicit goal of fix #2 and it is achieved without new
  schema artifacts.

---

## Migration / rollout notes

Applies only to environments where ADR 0015 is deployed. Greenfield
deployments adopt ADR 0015 + ADR 0016 together and skip the topic0
re-ingest.

1. **Parser code changes:**
   - Add upstream `SELECT EXISTS (SELECT 1 FROM ledgers WHERE sequence =
$N)` check before starting the ledger-group transaction. Skip the
     ledger if the row exists.
   - Remove `ON CONFLICT (hash) DO NOTHING` from `transaction_hash_index`
     inserts.
   - Remove `ON CONFLICT (hash, created_at) DO NOTHING` from
     `transactions` inserts.
   - Retain `ON CONFLICT DO NOTHING` on all other tables (defensive).
2. **Deploy parser update** to staging, verify clean ingest through a
   test re-run of a recent ledger. Expected behavior: on replay, the
   `ledgers`-existence check fires and the ledger is skipped before any
   INSERT runs.
3. **Topic0 pre-GA re-ingest:**
   - Identify the ledger range that was ingested under ADR 0014's
     canonicalization rules (post-ADR-0014 deployment, pre-ADR-0015
     deployment — or whatever range contains old-format `topic0`).
   - For each ledger in that range: run the standard re-ingest protocol
     (`DELETE FROM transaction_hash_index WHERE ledger_sequence = $N;
DELETE FROM transactions WHERE ledger_sequence = $N;` — cascades
     clean child tables including `soroban_events` — then re-ingest from
     `parsed_ledger_{N}.json` on S3).
   - The new parser writes `topic0` in typed form.
   - After completion, verify:
     ```sql
     SELECT COUNT(*) FROM soroban_events
     WHERE topic0 IS NOT NULL
       AND topic0 NOT LIKE '%:%';   -- any row without type prefix = old format
     ```
     Expected: 0.
4. **API documentation update:** state explicitly that
   `filter[topic0]` supports equality match only; prefix or `LIKE`
   operators are not part of the contract.
5. **Runbook update:** add "hash conflict during ingest — investigate
   parser determinism; do not retry until root-caused" as an
   operational response.
6. **Retire** the nightly hash-uniqueness monitoring query if it was
   retained under ADR 0015 as redundant audit. It is now subsumed by
   INSERT-time enforcement.

Rollback: restore the `ON CONFLICT` clauses in the parser; no schema
rollback needed (nothing changed at the DDL level). If the topic0
re-ingest needs to be partially undone, the source of truth remains
`parsed_ledger_{N}.json` on S3 — re-running the prior parser reproduces
the old-format rows. The two formats are unambiguous to tell apart
(`topic0` without `:` = old format, `topic0` with `:` = new format), so
rollback recovery is bounded.

---

## Open questions

None. All three fixes produce unambiguous, documented contracts.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [ADR 0015: Hash index, typed topic0, migration honesty, CHECK policy](0015_hash-index-topic-typing-migration-honesty.md)
- [PostgreSQL: `ON CONFLICT` semantics](https://www.postgresql.org/docs/current/sql-insert.html#SQL-ON-CONFLICT)
- [PostgreSQL: Indexes and Collations — `text_pattern_ops`](https://www.postgresql.org/docs/current/indexes-opclass.html)
- [CAP-0067: Unified Events](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md)
