---
id: '0022'
title: 'Schema snapshot correction (tokens / wasm_interface_metadata / soroban_contracts) + token metadata enrichment worker plan'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0011', '0012', '0014', '0018', '0019', '0020', '0021']
tags:
  [
    database,
    schema,
    correction,
    metadata,
    enrichment,
    stellar-toml,
    sep-1,
    sep-41,
    tokens,
    wasm,
    audit,
  ]
links: []
history:
  - date: 2026-04-20
    status: proposed
    who: fmazur
    note: 'ADR created — closes the two "blockers" surfaced in ADR 0021 coverage audit (E9 token metadata, E12 contract interface). Discovers that both were artifacts of an incomplete schema snapshot in ADR 0019, not real gaps. Corrects the snapshot and defines token metadata enrichment worker.'
---

# ADR 0022: Schema snapshot correction + token metadata enrichment worker plan

**Related:**

- [ADR 0019: Schema snapshot and sizing at 11M ledgers](0019_schema-snapshot-and-sizing-11m-ledgers.md) — document being corrected
- [ADR 0021: Schema ↔ endpoint ↔ frontend coverage matrix](0021_schema-endpoint-frontend-coverage-matrix.md) — ADR that surfaced the "blockers" this ADR resolves

---

## Status

`proposed` — corrective and clarifying delta.

This ADR does **not** change any production table. Every column it
references already exists in the live schema as documented in
[`docs/database-audit_pl.md` (2026-04-10 first-implementation audit)](../../docs/database-audit_pl.md).
What this ADR changes:

1. **Documents** the correct state of three tables whose ADR 0019
   snapshot was incomplete (`tokens`, `wasm_interface_metadata`,
   `soroban_contracts`).
2. **Defines** the strategy for populating `tokens.metadata` via a
   separate enrichment worker (new feature work, not a schema change).
3. **Closes** ADR 0021 blockers E9 and E12 by pointing to columns that
   already exist.

---

## Context

ADR 0021 Part III enumerated two endpoints as **❌ BLOCKERS** against
the final schema:

- **E9 `GET /tokens/:id`** — spec §6.9 requires `description`, `icon`,
  `home page` in token metadata. ADR 0019 snapshot for `tokens` did not
  expose any column capable of holding these fields.
- **E12 `GET /contracts/:contract_id/interface`** — spec §6.10 requires
  a list of public functions with parameter names and types. ADR 0019
  snapshot for `wasm_interface_metadata` showed only `name`,
  `uploaded_at_ledger`, `contract_type` — no function-list column.

Both descriptions were **wrong relative to the live schema**.

Cross-checking against the first-implementation DB audit
(`docs/database-audit_pl.md`, sections §`tokens`, §`soroban_contracts`,
§`wasm_interface_metadata`) surfaces the actual column lists for the
three tables. They differ from ADR 0019 snapshot in ways that are
directly relevant to E9 and E12.

Audit findings:

| Table                     | Missing from ADR 0019 snapshot                                               | Present in live schema                   |
| ------------------------- | ---------------------------------------------------------------------------- | ---------------------------------------- | --- | ------------------------------------ |
| `tokens`                  | `total_supply NUMERIC(28,7)`, `holder_count INTEGER`, `metadata JSONB`       | yes (all three, all currently NULL-only) |
| `wasm_interface_metadata` | `metadata JSONB NOT NULL` containing `"functions"` array + `"wasm_byte_len"` | yes (entire table body is this column)   |
| `soroban_contracts`       | `metadata JSONB`                                                             | yes (accumulated via `                   |     | ` merge at WASM upload + deployment) |

The ADR 0019 snapshot compacted these columns away during the iterative
schema revision across ADRs 0011–0018 and never restored them when
summarizing. ADR 0021's coverage audit surfaced the inconsistency.

Implication: both "blockers" are **documentation defects**, not schema
gaps. The data shapes required by E9 and E12 are already present in
the live schema.

However, for E9 there is a **separate real gap**: `tokens.metadata`
column exists but no ingest path writes to it. Per the audit
(line 365):

> `upsert_tokens_batch` ... `INSERT ... ON CONFLICT DO NOTHING` —
> inserts `asset_type, asset_code, issuer_address, contract_id, name,
total_supply, holder_count` (**BEZ `metadata`**)

So E9 is blocked not by schema, but by **lack of a population
pipeline** for metadata content. That is the genuine remediation work
this ADR defines.

---

## Decision

### Part 1: Correct ADR 0019 snapshot

The authoritative schema for the three affected tables is hereby
restated (matching the live state documented in
`database-audit_pl.md`). ADR 0019 §7, §8, §11 entries are superseded
by the following:

#### `tokens` (corrected)

```sql
CREATE TABLE tokens (
    id                SERIAL PRIMARY KEY,
    asset_type        VARCHAR(20) NOT NULL,
    asset_code        VARCHAR(12),
    issuer_address    VARCHAR(56) REFERENCES accounts(account_id),
    contract_id       VARCHAR(56) REFERENCES soroban_contracts(contract_id),
    name              VARCHAR(256),
    total_supply      NUMERIC(28, 7),              -- currently NULL (no UPDATE path); closed by task 0135
    holder_count      INTEGER,                     -- currently NULL (no UPDATE path); closed by task 0135
    metadata          JSONB,                       -- currently NULL (no write path); closed by this ADR Part 3
    CONSTRAINT ck_tokens_asset_type CHECK (
        asset_type IN ('native', 'classic', 'sac', 'soroban')
    )
);
CREATE UNIQUE INDEX idx_tokens_classic ON tokens (asset_code, issuer_address)
    WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX idx_tokens_soroban ON tokens (contract_id)
    WHERE asset_type = 'soroban';
CREATE UNIQUE INDEX idx_tokens_sac     ON tokens (contract_id)
    WHERE asset_type = 'sac';
CREATE INDEX idx_tokens_type ON tokens (asset_type);
```

**Note:** ADR 0019 additionally described `decimals SMALLINT`,
`metadata_ledger BIGINT`, and `search_vector TSVECTOR` with GIN /
trigram indexes. These were **speculative** — they did not exist in the
first-implementation audit. They are removed from the authoritative
reference. If any future feature needs decimals / full-text search, it
can be reintroduced as an additive change with a fresh ADR citing the
concrete endpoint that consumes it.

#### `wasm_interface_metadata` (corrected)

```sql
CREATE TABLE wasm_interface_metadata (
    wasm_hash VARCHAR(64) PRIMARY KEY,
    metadata  JSONB NOT NULL
    -- metadata shape (populated by WASM upload parser, crates/db/src/soroban.rs:149-166):
    --   {
    --     "functions": [
    --       { "name": "transfer",
    --         "params": [ { "name": "from", "type": "Address" }, ... ],
    --         "return": "Void" },
    --       ...
    --     ],
    --     "wasm_byte_len": <integer>
    --   }
);
```

No FKs. No secondary indexes. Keyed by natural `wasm_hash`. The
"2-ledger deployment pattern" (WASM uploaded in ledger A, contract
deployed in ledger B) is bridged by this table joining into
`soroban_contracts` via `wasm_hash` on deploy.

#### `soroban_contracts` (corrected — `metadata` JSONB added)

```sql
CREATE TABLE soroban_contracts (
    contract_id              VARCHAR(56) PRIMARY KEY,
    wasm_hash                VARCHAR(64),
    wasm_uploaded_at_ledger  BIGINT,
    deployer_account         VARCHAR(56) REFERENCES accounts(account_id),
    deployed_at_ledger       BIGINT,
    contract_type            VARCHAR(20) NOT NULL DEFAULT 'other',
    is_sac                   BOOLEAN NOT NULL DEFAULT FALSE,
    name                     VARCHAR(256),
    metadata                 JSONB,                  -- accumulated via `||` merge (see audit line 180)
    search_vector            TSVECTOR GENERATED ALWAYS AS (
                                 to_tsvector('simple',
                                     coalesce(metadata ->> 'name', '') || ' ' ||
                                     coalesce(name, ''))
                             ) STORED,
    CONSTRAINT ck_contracts_contract_type CHECK (
        contract_type IN ('nft', 'fungible', 'token', 'other')
    )
);
-- Indexes (after ADR 0020 drop of idx_contracts_deployer):
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);
```

`metadata` here is **upstream-sourced** from
`wasm_interface_metadata.metadata` at deploy time (see audit line 188,
`upsert_contract_deployments_batch` JOINs on `wasm_hash` and copies
`metadata`). It also accumulates additively via the `||` operator
(audit line 189, `update_contract_interfaces_by_wasm_hash`).

### Part 2: E12 — resolved with zero schema work

`GET /contracts/:contract_id/interface` becomes:

```sql
SELECT wim.metadata -> 'functions' AS interface,
       wim.metadata -> 'wasm_byte_len' AS wasm_byte_len
  FROM soroban_contracts sc
  JOIN wasm_interface_metadata wim ON wim.wasm_hash = sc.wasm_hash
 WHERE sc.contract_id = :contract_id;
```

The JSONB `functions` array already carries name / params / return
shape parsed from `contractspecv0` by the indexer at WASM upload time.
Frontend §6.10 "Contract interface — list of public functions with
parameter names and types" maps directly to this structure.

**Status:** E12 no longer blocked. Backend implementation task:
`GET /contracts/:contract_id/interface` handler that shapes the JSONB
into the REST response format. No schema change required.

### Part 3: E9 — token metadata enrichment worker

`tokens.metadata` column exists, but `upsert_tokens_batch` does not
populate it. Fields required by spec §6.9 (`description`, `icon`,
`home page`) must come from external sources, not from ledger
meta / XDR:

| Token type                              | Metadata source                                                                                                                            | Mechanism                                                                   |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `native` (XLM)                          | Protocol-defined constants                                                                                                                 | Hardcoded in backend or static seed row                                     |
| `classic`                               | `stellar.toml` at the issuer's `home_domain` (SEP-1)                                                                                       | HTTPS fetch; parse `[[CURRENCIES]]` entries matching `(asset_code, issuer)` |
| `sac`                                   | Same as `classic` — SAC wraps a classic asset                                                                                              | Same                                                                        |
| `soroban` (SEP-41 fungible, SEP-50 NFT) | Soroban RPC calls to contract: `name()`, `symbol()`, `decimals()`; icon URL by convention (contract storage or `name()` string convention) | Soroban RPC `simulateTransaction` / storage read                            |

**Architectural decision:** **separate async worker**, decoupled from
the main indexer pipeline.

Rejected alternatives:

- **Inline in indexer (`upsert_tokens_batch`):** pipes external HTTP /
  RPC into a hot path. Failures in stellar.toml serving cascade into
  ingest pipeline failures. Slows indexing. Requires retry logic
  coupling that does not belong in `crates/db`.
- **Live fetch in backend API at request time:** adds unpredictable
  latency to a hot read endpoint. Caches only hide the problem. Creates
  a hard runtime dependency on external web + chain infrastructure.
- **Frontend-side stellar.toml fetch:** CORS, N+1 fetches per list
  render, and violates the clean "backend is the only read interface"
  line from `backend-overview.md §2`.

Chosen design:

```
┌────────────┐      ┌──────────────┐       ┌─────────────┐
│ Indexer    │      │ Metadata     │       │ RDS         │
│ (ingest    │      │ Enrichment   │       │ PostgreSQL  │
│ pipeline)  │──────│ Worker       │──────>│ tokens.     │
│            │ new  │ (async,      │       │ metadata    │
│            │ rows │ periodic)    │       │ JSONB       │
└────────────┘      └──────┬───────┘       └─────────────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
      stellar.toml    Soroban RPC   Static seed
      (HTTPS per     (name/symbol  (native XLM)
       home_domain)   /decimals)
```

**Worker contract:**

- Trigger: scheduled (e.g. every 6h) or event-driven (on new token row).
- Input: rows in `tokens` where `metadata IS NULL` OR
  `last_enriched_at < now() - interval '24h'` (if we add that column).
- Output: `UPDATE tokens SET metadata = $1 WHERE id = $2`.
- Shape of `tokens.metadata` JSONB:

  ```json
  {
    "description": "USD Coin on Stellar",
    "icon_url": "https://circle.com/usdc.png",
    "home_page": "https://circle.com",
    "decimals": 7,
    "enriched_at": "2026-04-20T12:00:00Z",
    "enriched_source": "stellar.toml" | "sep-41" | "native-const"
  }
  ```

- Graceful degradation: if source unavailable, leave row untouched; UI
  renders what it has.

**E9 coverage after worker exists:**

```sql
SELECT t.id, t.asset_type, t.asset_code, t.issuer_address, t.contract_id,
       t.name, t.total_supply, t.holder_count,
       t.metadata ->> 'description' AS description,
       t.metadata ->> 'icon_url'    AS icon_url,
       t.metadata ->> 'home_page'   AS home_page,
       CASE WHEN t.asset_type = 'soroban' OR t.asset_type = 'sac'
            THEN sc.deployed_at_ledger END AS deployed_at_ledger
  FROM tokens t
  LEFT JOIN soroban_contracts sc ON sc.contract_id = t.contract_id
 WHERE t.id = :id;
```

All §6.9 fields realized. NULL-tolerant (per frontend spec: "metadata
should tolerate partial availability").

**Status before worker ships:** E9 technically executes (query runs,
`metadata`-derived fields return NULL). Frontend degrades gracefully by
spec. Not a hard blocker; **soft-blocked pending worker**.

**Status after worker ships:** E9 fully realized.

### Part 4: ADR 0021 coverage matrix update

Applied separately in a follow-up edit to ADR 0021. Summary:

| Endpoint                                | ADR 0021 status         | Post this ADR                                                              |
| --------------------------------------- | ----------------------- | -------------------------------------------------------------------------- |
| E9 `/tokens/:id`                        | ❌ BLOCKER (schema gap) | ⚠️ (schema OK, awaits enrichment worker)                                   |
| E12 `/contracts/:contract_id/interface` | ❌ BLOCKER (schema gap) | ✅ (resolvable from existing `wasm_interface_metadata.metadata.functions`) |

---

## Rationale

### Why document this as a fresh ADR rather than editing ADR 0019

ADRs are **post-factum** decision records (per `lore/2-adrs/CLAUDE.md`
header convention). Amending ADR 0019 in place would erase the
iteration trail and make ADR 0021's discovery (the coverage audit that
found the mistake) look like it operated on correct inputs, which it
did not. A separate corrective ADR preserves:

- the snapshot-shaped history of how the schema design played out,
- the audit discovery as a genuine finding (not a pre-baked input),
- future readers' ability to see where ADR 0019 was wrong and why.

### Why the enrichment worker is a separate worker, not an indexer step

- `crates/indexer` is deterministic over ledger inputs. Adding HTTPS
  calls (stellar.toml) or RPC calls (SEP-41 `name()`) to the indexer
  breaks that invariant and couples it to network availability.
- Metadata is not part of the ledger; it has no natural ordering
  relative to ingest. It lives on its own schedule.
- Decoupling lets the worker be idle / failed / replaced without
  affecting chain data correctness.

### Why not just widen `tokens` with `description TEXT`, `icon_url TEXT`, `home_page TEXT`

Three nullable text columns vs one JSONB:

- JSONB already exists — no migration.
- JSONB tolerates metadata shape evolution (SEP-1 and SEP-41
  conventions vary). Adding structured columns fixes the shape, making
  future metadata fields (colors, markets, social links) require
  further migrations.
- Query cost is comparable (`->>` operator is cheap).

### Why `decimals` / `metadata_ledger` / `search_vector` get dropped from the authoritative `tokens` snapshot

ADR 0019 speculatively added these three columns without any
corresponding endpoint call-site. The first-implementation schema
never materialized them. Per the schema-minimization principle
throughout ADRs 0011–0020 (no "just-in-case" columns), they should not
be in the authoritative snapshot. If a future feature needs
`decimals` in the row (e.g. formatted-amount rendering without a
JSONB lookup), it can be reintroduced with a dedicated ADR citing the
feature.

---

## Consequences

### Positive

- **Two ADR 0021 blockers closed** with zero schema change.
- ADR 0019 authoritative snapshot aligns with reality. Downstream ADRs
  (including ADR 0021 Part II queries) become correct.
- Token metadata enrichment worker is now a clearly-scoped follow-up
  task, decoupled from indexer correctness.
- E12 (contract interface) becomes a trivial backend handler: select
  one JSONB field and shape the response.

### Negative

- ADR 0019 Mermaid ERD (and ADR 0020 Mermaid ERD which inherited from
  it) now list `tokens` and `wasm_interface_metadata` with stale column
  sets. The ERDs are visual references, not machine-consumed. A future
  housekeeping pass can regenerate both.
- The enrichment worker adds an operational surface (scheduled Lambda
  or equivalent) that did not exist before. Complexity cost is
  accepted as the only way to close E9 without putting external
  fetches in the indexer or API hot paths.
- `tokens.metadata` JSONB carries an implicit schema (the field list)
  that lives in worker code rather than DB CHECK. Acceptable for
  evolving metadata shapes; CHECK can be added later if worker output
  stabilizes.

---

## Follow-up work (not blocking this ADR)

1. **Token metadata enrichment worker** — create task in backlog:
   - Source adapters: stellar.toml fetcher, Soroban RPC fetcher, native
     seed.
   - Scheduling: periodic (e.g. 6h) + on-demand re-enrich.
   - Error handling: transient vs permanent source failures; retry /
     backoff.
   - Observability: per-source success rate metrics.
2. **Task 0135 (token holder_count tracking)** — already in `active/`;
   populates `tokens.total_supply` + `tokens.holder_count`. Covers the
   same "currently always NULL" gap for those columns that this ADR
   covers for `metadata`.
3. **ADR 0021 update** — edit Part III coverage table: E9 → ⚠️,
   E12 → ✅. Edit Part IV follow-ups: drop E9 / E12 as blockers, keep
   as tracked work.
4. **ADR 0019 / ADR 0020 ERD regeneration** — regenerate Mermaid ERDs
   with the corrected `tokens` and `wasm_interface_metadata` shapes.
   Low-priority housekeeping.

---

## References

- [`docs/database-audit_pl.md`](../../docs/database-audit_pl.md) —
  ground-truth first-implementation audit (2026-04-10); authoritative
  for the corrected column lists. Cited sections: `§tokens` (line 332),
  `§soroban_contracts` (line 175), `§wasm_interface_metadata` (line 495).
- [ADR 0019: Schema snapshot and sizing at 11M ledgers](0019_schema-snapshot-and-sizing-11m-ledgers.md)
- [ADR 0021: Schema ↔ endpoint ↔ frontend coverage matrix](0021_schema-endpoint-frontend-coverage-matrix.md)
- [Stellar SEP-1: stellar.toml](https://stellar.org/protocol/sep-1) —
  asset metadata convention for classic issuers.
- [Stellar SEP-41: Token Interface](https://stellar.org/protocol/sep-41) —
  Soroban fungible token contract interface (`name`, `symbol`, `decimals`).
- [`backend-overview.md §2`](../../docs/architecture/backend/backend-overview.md) —
  mandates backend as sole read interface (rationale for worker vs live
  fetch).
- [`frontend-overview.md §6.9`](../../docs/architecture/frontend/frontend-overview.md) —
  token detail requirements.
- [`frontend-overview.md §6.10`](../../docs/architecture/frontend/frontend-overview.md) —
  contract interface requirements.
