---
id: '0015'
title: 'Hash uniqueness, typed topic0, memo migration honesty, CHECK policy'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0013', '0014']
tags:
  [
    database,
    schema,
    stellar,
    xdr,
    hash-integrity,
    soroban-events,
    memo,
    check-constraints,
  ]
links: []
history:
  - date: 2026-04-19
    status: proposed
    who: fmazur
    note: 'ADR created — corrective revision of ADR 0014 after second-pass review'
---

# ADR 0015: Hash uniqueness, typed `topic0`, memo migration honesty, CHECK policy

**Related:**

- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)

---

## Status

`proposed` — corrective delta on top of ADR 0014. Does not supersede 0014 in
full; this ADR overrides ADR 0014 only on the four points listed below. Every
other decision in ADR 0011–0014 stands.

---

## Context

ADR 0014 resolved seven correctness issues in ADR 0013. A second-pass review
identified four places where the resolution is either too soft, semantically
lossy, or relies on language that suggests a stronger guarantee than the schema
actually provides:

1. **Transaction hash uniqueness.** ADR 0014 describes
   `UNIQUE (hash, created_at)` + parser invariant + nightly monitoring as
   "enforced". In reality: it is an ingest invariant plus out-of-band detection,
   not DB-level enforcement. A buggy parser writing the same hash into two
   different ledger windows would insert both rows without any immediate
   constraint violation; the nightly monitor catches it hours later. For
   `/transactions/:hash` lookups, the situation is also unsatisfactory: without
   a global unique index on `hash`, the planner must scan `idx_tx_hash` on
   every partition (cross-partition search), which degrades linearly with
   partition count.

2. **`topic0` canonicalization is lossy across ScVal types.** ADR 0014
   specified `TEXT` with rules that produce the same string for
   `Symbol("transfer")` and `String("transfer")`, for `u64(1)` and `i64(1)`,
   for `u32(1)` and `u64(1)`. This collapses distinct protocol-level values
   into one, and makes filter-by-topic0 return false positives for
   applications that legitimately use both `Symbol` and `String` or different
   integer widths.

3. **Memo migration for existing rows described too optimistically.** ADR 0014
   specifies `ALTER COLUMN TYPE memo TYPE BYTEA USING convert_to(memo,
'UTF8')`. This works for greenfield, but any legacy row whose original
   memo was non-UTF-8 (legitimate for `memo_text`, which is 28 arbitrary
   bytes) was already corrupted by the prior `VARCHAR(128)` storage — the
   `convert_to` call cannot restore bytes that are already lost. The ADR text
   does not call this out.

4. **CHECK constraint rationale too shallow.** ADR 0014 excludes
   `operations.type`, `transfer_type`, `role` from CHECK with the phrase
   "application-level taxonomy." That reads as hand-wave. The real distinction
   is between enums fixed by Stellar protocol (low migration burden) versus
   enums that evolve with protocol upgrades or explorer features
   (constant migration burden). The rationale needs to say that explicitly,
   and one additional column — `token_transfers.source` — meets the
   "protocol-fixed/provenance" criterion and should receive a CHECK.

This ADR resolves all four. One new table (`transaction_hash_index`) is
introduced — the justification for it is spelled out in full, and the table is
the smallest possible artifact that satisfies two concrete requirements
simultaneously (hash global uniqueness and fast cross-partition lookup).
No other schema growth.

---

## Decision

### Summary of decisions

- **Transaction hash global uniqueness is DB-enforced via a new minimal
  lookup table `transaction_hash_index`.** Not a monitoring-only mitigation.
  The table is tiny (three columns, PK `hash`), serves double duty as the
  routing index for `GET /transactions/:hash`, and is written in the same
  ingest batch as `transactions`. Without it, we have no DB-level defense
  against parser bugs producing duplicate hashes across partitions, and
  `/transactions/:hash` has no single-lookup path.
- **`topic0` stores a typed canonical representation** of the form
  `"{type_code}:{value}"`, preserving ScVal type information. Remains `TEXT`
  with the same B-tree index. Parser produces the type-prefixed form
  deterministically; different ScVal types never collapse to the same string.
- **Memo migration for existing data is explicitly "best-effort, lossy for
  any pre-existing non-UTF-8 `memo_text` bytes."** Not a silent ALTER. For
  strict correctness of historical memos, the only path is re-ingestion of
  affected ledgers from S3 (where the raw XDR is preserved). We are pre-GA;
  the project accepts best-effort migration for testnet-era historical rows
  and relies on greenfield-from-BYTEA for all future data.
- **CHECK constraint policy is formalized.** `CHECK` is added only on
  columns whose value space is an enum (a) fixed by Stellar/Soroban protocol
  or (b) a closed provenance marker internal to ingest. One additional
  column — `token_transfers.source` — is promoted into the CHECK set under
  criterion (b). `operations.type`, `transfer_type`, `role` remain without
  CHECK for the reason spelled out below: they evolve with protocol upgrades
  and explorer features, and adding CHECK creates a mandatory migration
  coupling with no integrity gain.

### Transaction hash uniqueness (fix #1)

**Decision:**

Introduce `transaction_hash_index`, a minimal non-partitioned lookup table
whose primary purpose is **globally unique `hash`** and whose secondary
purpose is **O(log N) routing for `GET /transactions/:hash`**.

```sql
CREATE TABLE transaction_hash_index (
    hash            VARCHAR(64) PRIMARY KEY,
    ledger_sequence BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL
);
```

**Three columns. No FK. No auxiliary indexes.** The PK `hash` is the only
access path and the only integrity constraint we need.

**Ingest semantics (normative):**

For every `transactions` row inserted in a ledger-group commit, a matching
row is inserted into `transaction_hash_index` in the same SQL transaction.
Both inserts use `ON CONFLICT (hash) DO NOTHING` / `ON CONFLICT (hash,
created_at) DO NOTHING` respectively. If `transaction_hash_index` rejects an
insert (hash already present), the `transactions` insert is a no-op and the
ledger-group commit proceeds.

**Re-ingest semantics (normative):**

`DELETE FROM transactions WHERE ledger_sequence = N` cascades via FK to all
child tables (operations, events, invocations, participants, transfers,
nft_ownership). It does **not** cascade to `transaction_hash_index` because
that table has no FK to `transactions`. The re-ingest path therefore also
issues:

```sql
DELETE FROM transaction_hash_index WHERE ledger_sequence = $1;
```

explicitly before re-inserting. This is part of the re-ingest stored
procedure or script — a one-line addition to the existing protocol.

**Lookup path for `GET /transactions/:hash`:**

```sql
-- Step 1: O(log N) PK lookup — single partition-agnostic hit
SELECT ledger_sequence, created_at
FROM transaction_hash_index
WHERE hash = $1;

-- Step 2: Partition-pruned fetch of the full row
SELECT *
FROM transactions
WHERE hash = $1 AND created_at = $2;

-- Step 3: Fetch parsed_ledger_{ledger_sequence}.json from S3 for detail view
```

Step 2 is partition-pruned on `created_at`. Without
`transaction_hash_index`, step 1 does not exist and step 2's equivalent
(`WHERE hash = $1`) scans every partition's `idx_tx_hash`.

**Size envelope (mainnet projection):**

At ~300M transactions and ~90 B per row (including PK overhead), the table
lands at ~30 GB of heap + ~20 GB of PK index = ~50 GB total. This is ~3–5%
of the projected full-DB footprint. In exchange, it delivers:

- rigorous DB-level hash uniqueness (not invariant + detection);
- O(log N) lookup for the most-accessed detail endpoint.

Nightly monitoring query from ADR 0014 is removed — the constraint is now
enforced by the PK, not a query.

### `topic0` typed canonical representation (fix #2)

**Decision:**

`soroban_events.topic0` remains `TEXT`. The parser produces a typed
canonical string of the form:

```
{type_code}:{value}
```

Where `type_code` is a short fixed token identifying the ScVal variant and
`value` is the canonical string encoding for that variant. Different ScVal
types **never** produce the same string even if their value encodings would
otherwise collide.

**Normative encoding rules (exhaustive for ScVal variants that can appear
as event topics):**

| ScVal variant                                    | `type_code` | Value encoding                     | Example                                        |
| ------------------------------------------------ | ----------- | ---------------------------------- | ---------------------------------------------- |
| `Symbol`                                         | `sym`       | raw symbol text (UTF-8, ≤32 bytes) | `sym:transfer`                                 |
| `String`                                         | `str`       | raw string text                    | `str:transfer`                                 |
| `Bool`                                           | `bool`      | `true` or `false`                  | `bool:true`                                    |
| `Void`                                           | `void`      | empty                              | `void:`                                        |
| `U32`                                            | `u32`       | decimal                            | `u32:1`                                        |
| `I32`                                            | `i32`       | decimal                            | `i32:1`                                        |
| `U64`                                            | `u64`       | decimal                            | `u64:1`                                        |
| `I64`                                            | `i64`       | decimal                            | `i64:1`                                        |
| `U128`                                           | `u128`      | decimal                            | `u128:340282366920938463463374607431768211455` |
| `I128`                                           | `i128`      | decimal (signed)                   | `i128:-5`                                      |
| `U256`                                           | `u256`      | decimal                            | `u256:…`                                       |
| `I256`                                           | `i256`      | decimal (signed)                   | `i256:…`                                       |
| `Timepoint`                                      | `tp`        | decimal (seconds since epoch)      | `tp:1714000000`                                |
| `Duration`                                       | `dur`       | decimal (seconds)                  | `dur:3600`                                     |
| `Bytes`                                          | `bytes`     | lowercase hex, no `0x`             | `bytes:deadbeef`                               |
| `Address` (account G…)                           | `addr`      | StrKey G-form                      | `addr:GABC…`                                   |
| `Address` (contract C…)                          | `addr`      | StrKey C-form                      | `addr:CDEF…`                                   |
| `Error`                                          | `err`       | `{category}:{code}`                | `err:contract:5`                               |
| Any composite (Vec, Map, ContractInstance, etc.) | `xdr`       | base64-encoded canonical XDR       | `xdr:AAAA…`                                    |

**Index implication:** the existing B-tree
`(contract_id, topic0, created_at DESC)` handles the new encoding without
change. All realistic topic0 values remain well under Postgres's per-entry
limit. Prefix filters (`WHERE topic0 LIKE 'sym:transfer%'`) continue to use
the B-tree.

**API / filter implication:** an endpoint filter `filter[topic0]=sym:transfer`
matches only `Symbol("transfer")`, not `String("transfer")`. Clients that
want to match "any topic0 string with value 'transfer'" must issue two
filter values. This is the correct semantics — the protocol distinguishes
these types, and so should the explorer.

### Memo migration honesty (fix #3)

**Decision — three explicit cases:**

1. **Greenfield (new deployments):** `memo` is `BYTEA` from the initial DDL.
   Parser writes raw bytes extracted from the XDR memo field. No migration,
   no data-loss concern. This is the path all production deployments take.

2. **Pre-GA deployments with existing rows** (current project state,
   development/testnet environments with rows predating ADR 0014):

   - `ALTER TABLE transactions ALTER COLUMN memo TYPE BYTEA USING
convert_to(memo, 'UTF8')` is a **best-effort** migration. It succeeds
     for any memo whose original bytes were valid UTF-8. It produces
     corrupt bytes for any memo whose original bytes were not — and those
     bytes were already corrupted at write time by the prior `VARCHAR(128)`
     storage, so `convert_to` cannot recover them.
   - **The migration does not restore data that VARCHAR storage already
     lost.** It merely changes the column type. This is stated explicitly
     so that nobody later reads the migration as a guarantee of historical
     correctness.
   - **If historical memo correctness is required** for any pre-ADR-0014
     ledger range, the only correct recovery path is: re-ingest those
     ledgers from S3 `parsed_ledger_{N}.json` (where the raw XDR was
     preserved under ADR 0011 offload contract) via the re-ingest protocol,
     letting the parser write `memo` as `BYTEA` directly from the source.
     Re-ingest wall-clock cost is bounded by the number of affected ledgers;
     for testnet-era data this is hours, not days.
   - The project accepts best-effort migration without mandatory re-ingest.
     No user-facing feature depends on pre-ADR-0014 memo fidelity.

3. **Post-GA:** not applicable. The project is pre-GA; there is no
   production historical memo data whose integrity must be preserved across
   the VARCHAR→BYTEA transition.

**Documentation contract:** release notes and migration runbook both quote
the language "best-effort, lossy for pre-existing non-UTF-8 `memo_text`
bytes; for strict correctness, re-ingest affected ledgers." ADR 0014's
migration step 5 is superseded by this section.

### CHECK constraint policy (fix #4)

**Decision — formalized policy with four categories:**

A CHECK constraint is added to a column if and only if its value space
falls into one of these two categories:

- **(A) Protocol-fixed enum.** The set of legal values is defined by
  Stellar/Soroban protocol and changes rarely (once per many protocol
  upgrades, if ever). Adding CHECK here imposes near-zero migration burden
  because the enum does not drift.

- **(B) Ingest-internal provenance marker.** A small closed set controlled
  entirely by our parser code (never by protocol or by externally-visible
  taxonomy). Changes to this set are our own explicit decisions and come
  with a controlled migration.

A CHECK constraint is **not** added to a column if its value space falls
into one of these:

- **(C) Protocol-evolving enum.** Defined by protocol, but new values ship
  with protocol upgrades (e.g., `operations.type` received
  `BUMP_FOOTPRINT_TTL` in protocol 22 and `RESTORE_FOOTPRINT` in protocol
  20). CHECK here forces a DB migration coupled to every protocol upgrade;
  missing the migration turns a protocol upgrade into a parser outage. No
  integrity gain offsets this cost.

- **(D) Application-level taxonomy.** Explorer-specific groupings
  (`token_transfers.transfer_type`, `transaction_participants.role`) whose
  membership is expected to grow as we model new semantics. CHECK here
  forces a migration per feature addition; parser-level tests are the
  appropriate integrity gate.

**Applying the policy:**

| Column                                | Category |    CHECK?     | Source of truth                                                                |
| ------------------------------------- | -------- | :-----------: | ------------------------------------------------------------------------------ |
| `transactions.memo_type`              | A        |      yes      | XDR `MemoType` enum (5 values, stable)                                         |
| `soroban_events.event_type`           | A        |      yes      | CAP-67 event types (`contract`, `system`, `diagnostic`)                        |
| `soroban_contracts.contract_type`     | A+B      |      yes      | classification enum (`nft`, `fungible`, `token`, `other`) — bounded; we own it |
| `nft_ownership.event_type`            | A        |      yes      | SEP-0050 event types (`mint`, `transfer`, `burn`)                              |
| `account_balances_current.asset_type` | A        |      yes      | XDR `AssetType` enum                                                           |
| `account_balance_history.asset_type`  | A        |      yes      | XDR `AssetType` enum                                                           |
| `tokens.asset_type`                   | A+B      | yes (already) | bounded classifier (`native`, `classic`, `sac`, `soroban`)                     |
| **`token_transfers.source`**          | **B**    | **yes (NEW)** | provenance marker (`operation`, `event`) — closed set controlled by parser     |
| `operations.type`                     | C        |      no       | XDR `OperationType` — evolves with protocol                                    |
| `token_transfers.transfer_type`       | D        |      no       | explorer taxonomy                                                              |
| `transaction_participants.role`       | D        |      no       | explorer taxonomy                                                              |

**Change vs ADR 0014:** one CHECK added —
`token_transfers.source IN ('operation', 'event')`. No CHECKs removed.

---

## Detailed schema changes

### New table

```sql
CREATE TABLE transaction_hash_index (
    hash            VARCHAR(64) PRIMARY KEY,
    ledger_sequence BIGINT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL
);
-- No FK (by project policy: transactions is partitioned, and FK to
--   partitioned parent requires composite key; a lookup table whose sole
--   purpose is global hash uniqueness does not benefit from such complexity).
-- No FK to ledgers (by project policy: ledgers is not a relational hub).
-- No auxiliary indexes. The PK is the only access path.
```

**Why this is the minimal shape:**

- Fewer columns are not possible: `hash` identifies, `ledger_sequence` routes
  to S3, `created_at` routes to the correct `transactions` partition.
- `transaction_id` is intentionally **not** stored here — the caller can
  obtain it from `transactions` with the partition-pruned step 2 query above.
  Storing it would add 8 bytes per row with no access pattern that needs it.
- No FK. Adding a composite FK `(hash, created_at) → transactions` adds a
  per-insert check with zero additional integrity value (the PK on `hash`
  already prevents duplicates; if `transactions` is missing for a hash, the
  step-2 query returns empty and the API returns 404 — same as if the
  lookup table were inconsistent).

### Column changes

| Table             | Column   | Change                                                                         | Source             |
| ----------------- | -------- | ------------------------------------------------------------------------------ | ------------------ |
| `soroban_events`  | `topic0` | unchanged type (TEXT), parser encoding changes to `"{type_code}:{value}"` form | this ADR §Decision |
| `token_transfers` | `source` | add `CHECK (source IN ('operation', 'event'))`                                 | this ADR §Decision |

### Removed from ADR 0014

- Nightly monitoring query on `(hash, COUNT(DISTINCT created_at))`. No longer
  needed — the PK on `transaction_hash_index` enforces global uniqueness at
  write time. The query can remain as redundant audit if the operator
  desires, but it is no longer part of the integrity contract.

### Unchanged

Everything else from ADR 0014 stands, including:

- Canonical G-identity and `*_muxed` auxiliary columns.
- `memo` as `BYTEA` with `memo_type` discriminator.
- The six CHECK constraints from ADR 0014.
- The six prefix/trigram search indexes from ADR 0014.
- All FK graph decisions from ADR 0013.
- No FK to `ledgers`.
- Partitioning strategy from ADR 0012.

---

## Rationale

### Why `transaction_hash_index` is the minimum, not overreach

The alternative — "parser invariant + nightly monitoring" (ADR 0014) — has
two weaknesses that a 3-column lookup table resolves simultaneously:

1. **No DB-level uniqueness.** Parser bugs produce silent duplicates;
   detection latency is one night. The lookup table turns that into an
   immediate INSERT failure.
2. **No single-hit path for `/transactions/:hash`.** Without the lookup,
   every hash lookup scans `idx_tx_hash` across every partition. At mainnet
   partition counts (5-year projection: ~60 monthly partitions) this is
   ~60 index probes per request. With the lookup, it is one PK probe plus
   one partition-pruned probe.

One new table, three columns, no FK, no auxiliary indexes — this is as
small as "DB-enforced hash uniqueness + fast routing" can be made in
Postgres. It does not introduce a subsystem, an event store, or a
read model. It is a lookup.

The project principle "lightweight bridge DB" is preserved — ~50 GB at
mainnet scale is noise against the total DB weight, and the table replaces
a dual concern (integrity + routing) that would otherwise require two
separate mechanisms.

### Why typed `topic0` is `"{type_code}:{value}"`, not separate columns

Two columns (`topic0_type VARCHAR(8)` + `topic0_value TEXT`) were
considered and rejected. A single typed string:

- keeps the existing single-column B-tree index — no schema restructuring
  beyond parser encoding;
- avoids composite-filter complexity at the API layer
  (`WHERE type = ? AND value = ?` vs. `WHERE topic0 = ?`);
- is trivially searchable with prefix `LIKE 'addr:G%'` if needed;
- costs no more storage than the equivalent two-column layout (total bytes
  are the same);
- maintains ADR 0014's `topic0 TEXT` decision — parser does more work,
  schema does not.

Type collisions (`Symbol("x")` vs `String("x")`) are semantically distinct
in Soroban — the resolution must preserve that. Prefix-coding does; plain
canonicalization does not.

### Why memo migration is called "best-effort"

ADR 0014's `USING convert_to(memo, 'UTF8')` cannot restore bytes that
`VARCHAR(128)` already corrupted on the original write. Calling the
migration "successful" when some rows are silently wrong is the kind of
documentation lie that surfaces as a P2 incident six months later.

The honest statement — "best-effort, lossy for non-UTF-8; re-ingest from
S3 for strict correctness" — costs nothing to write and sets correct
operator expectations. The project is pre-GA, so there is no production
historical memo data at stake; this is pure documentation hygiene.

### Why one more CHECK, not a handful

The four-category policy matches how Stellar protocol and our ingest code
actually evolve. Protocol-fixed enums (`memo_type`, `event_type`,
`asset_type`) are stable across multiple protocol versions — CHECK there
pays for itself once and incurs migration cost approximately never.
Protocol-evolving enums (`operations.type`) are the exact case where CHECK
actively hurts: every protocol upgrade that adds an operation type (two in
the last year) would trigger a forced DB migration to avoid a parser
outage. Application taxonomies (`transfer_type`, `role`) sit entirely in
our code — the integrity gate is parser tests, not a DB constraint that
locks us into a migration per new category.

`token_transfers.source` crosses the line from D into B: it is a closed
two-value provenance marker (`operation` when the row came from classic
payment extraction, `event` when it came from SEP-41 event extraction).
New values appear only if we restructure the parser's provenance model,
which is already a controlled schema event. CHECK adds integrity at
effectively zero cost.

### What this ADR refuses to do

- No hash registry with additional columns "for traceability."
- No event store, audit log, or history table beyond what ADR 0012/0013
  already established.
- No secondary indexes on `transaction_hash_index` beyond the PK.
- No CHECK on `operations.type`, `transfer_type`, `role` — the policy
  explicitly rules these out.
- No reparse of S3 data as part of the default migration path — that is
  an operator choice reserved for strict-correctness scenarios.

---

## Consequences

### Stellar/XDR compliance

- **Positive.** Typed `topic0` aligns with ScVal's type-discriminated model;
  `Symbol` and `String` are first-class different types in the protocol and
  now are first-class different values in the schema. Hash uniqueness
  matches Stellar's global-uniqueness guarantee on `TransactionEnvelope.hash()`
  at the database level, not just at the parser level.
- **Neutral.** Memo handling is unchanged semantically (ADR 0014's BYTEA
  decision stands); only the migration narrative is corrected.
- **Neutral.** CHECK policy formalizes what ADR 0014 was doing, with one
  additional CHECK. No protocol semantics affected.

### DB weight

- **One new table** at ~50 GB projected mainnet. This is the first instance
  in ADR 0011–0015 of a lookup structure not pulling double duty as a
  filtering index; it is justified by the dual integrity+routing requirement
  and by the absence of a cheaper alternative. Project principle
  "lightweight bridge DB" is preserved — the new table is a lookup, not a
  storage layer, and carries no JSONB, no heavy payloads, no secondary
  indexes.
- **No other schema growth.** `topic0` type change (parser encoding) is
  free. CHECK addition is metadata-only.

### History correctness

- **Improved.** DB-enforced hash uniqueness means historical queries can
  rely on `(hash, created_at)` tuples being unambiguous, now as a
  constraint rather than an invariant. Typed `topic0` preserves
  protocol-level distinctions in historical event queries.
- **No regression.** All history paths from ADR 0012/0013/0014 continue
  working exactly as before.

### Endpoint performance

- **`GET /transactions/:hash`:** strictly faster. One PK probe (~sub-ms) +
  one partition-pruned secondary query (~sub-ms). Previously: N-partition
  scan (linear in partition count).
- **`GET /contracts/:id/events` with `filter[topic0]`:** strictly more
  accurate. No change in query cost.
- **Other endpoints:** no change.

### Ingest simplicity

- **Small increase.** Every transaction insert is now accompanied by a
  `transaction_hash_index` insert in the same SQL transaction. Two inserts
  per transaction instead of one. The second insert is a narrow 3-column
  row and shares the ingest batch. Parser code gains one line per
  transaction; parse-phase complexity unchanged.
- **Re-ingest protocol gains one line:** `DELETE FROM
transaction_hash_index WHERE ledger_sequence = $1` before the existing
  `DELETE FROM transactions WHERE ledger_sequence = $1`. Documented in the
  re-ingest runbook.

### Replay / re-ingest risk

- **Reduced.** DB-enforced hash uniqueness catches any replay that would
  have produced a duplicate row, at INSERT time rather than in a nightly
  monitor. The cost is the extra DELETE in the re-ingest protocol; a
  one-time operator cost per affected ledger.

### Operational cost

- ~50 GB additional storage at mainnet projection. Acceptable fraction of
  total footprint. No new extensions, no new services, no new runbooks
  beyond a one-line addition to the existing re-ingest runbook.
- Monitoring query from ADR 0014 can be retired. Net reduction in
  monitoring surface.

---

## Migration / rollout notes

Applies only to environments where ADR 0014 is already deployed. Greenfield
deployments incorporate these changes directly.

1. **Create the lookup table** (empty).
   ```sql
   CREATE TABLE transaction_hash_index (
       hash            VARCHAR(64) PRIMARY KEY,
       ledger_sequence BIGINT NOT NULL,
       created_at      TIMESTAMPTZ NOT NULL
   );
   ```
2. **Backfill from existing `transactions`.**
   ```sql
   INSERT INTO transaction_hash_index (hash, ledger_sequence, created_at)
   SELECT hash, ledger_sequence, created_at
   FROM transactions
   ON CONFLICT (hash) DO NOTHING;
   ```
   If this surfaces existing duplicates (parser bug already realized), the
   `ON CONFLICT` clause drops later rows. Run a pre-check query first to
   quantify:
   ```sql
   SELECT hash, COUNT(*) FROM transactions GROUP BY hash HAVING COUNT(*) > 1;
   ```
   If the pre-check returns rows, resolve them manually (identify correct
   `ledger_sequence` for each hash via S3, delete the incorrect
   `transactions` rows, then backfill). This is a one-time operator task;
   does not block the migration for environments with no pre-existing
   duplicates.
3. **Add the CHECK constraint on `token_transfers.source`.**
   ```sql
   ALTER TABLE token_transfers
       ADD CONSTRAINT ck_tt_source
       CHECK (source IN ('operation', 'event'))
       NOT VALID;
   ALTER TABLE token_transfers VALIDATE CONSTRAINT ck_tt_source;
   ```
4. **Parser update** — one change:
   - On every `transactions` insert, insert matching row into
     `transaction_hash_index` in the same SQL transaction.
   - Topic0 encoding rules (section Decision) replace ADR 0014's
     canonicalization rules. All new rows use typed form.
5. **Re-ingest affected rows for typed `topic0` (optional, operator
   choice).** Existing `soroban_events.topic0` rows written under ADR
   0014's rules remain in the old format; the index and the filter
   semantics continue to work (the old format is still valid B-tree
   content), but type collisions persist for those historical rows. If
   historical event-type correctness is required, re-ingest via the
   existing protocol. Recommended for pre-GA; skippable if no dashboard or
   API consumer filters on `topic0` for historical data.
6. **Update re-ingest runbook:** add `DELETE FROM transaction_hash_index
WHERE ledger_sequence = $1` as the first step.
7. **Retire nightly monitoring query** on `(hash, COUNT(DISTINCT
created_at))`. Optional — operators may keep it as redundant audit.

Rollback: drop the new table, drop the CHECK constraint, revert parser
code. All reversible. Any rows written with typed `topic0` continue to
function after parser rollback (the old rules are a subset of valid TEXT).

---

## Open questions

None that block this ADR. One deliberately not addressed:

- Whether `transaction_hash_index` should eventually carry additional
  columns (e.g., `transaction_id`) to shortcut step 2 of the
  `/transactions/:hash` lookup. Not included here because step 2 is
  partition-pruned and sub-millisecond already; adding columns to the
  lookup table trades storage for a marginal latency gain, and that
  trade-off is not currently motivated by any measurement. Revisit if
  and only if `/transactions/:hash` latency becomes a measured hotspot.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema revision](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0013: Sequential ingest schema with full FK integrity](0013_sequential-ingest-full-fk-schema.md)
- [ADR 0014: Schema fixes — Stellar/XDR compliance](0014_schema-fixes-stellar-xdr-compliance.md)
- [Stellar core: `Transaction::getContentsHash`](https://github.com/stellar/stellar-core) — transaction hash determinism
- [CAP-0067: Unified Events](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md) — ScVal in event topics
- [Stellar XDR: Operation types by protocol version](https://github.com/stellar/stellar-xdr)
- [PostgreSQL: Partitioning and unique constraints](https://www.postgresql.org/docs/current/ddl-partitioning.html#DDL-PARTITIONING-DECLARATIVE-LIMITATIONS)
