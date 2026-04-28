---
id: '0028'
title: 'ParsedLedgerArtifact v1 — canonical shape of parsed_ledger_{seq}.json.zst'
status: superseded
deciders: [stkrolikiewicz]
related_tasks: ['0146', '0145', '0147', '0126', '0135']
related_adrs: ['0011', '0012', '0018', '0023', '0024', '0026', '0027', '0029']
tags: [s3, artifact, schema, parser, foundation]
links: []
history:
  - date: '2026-04-20'
    status: proposed
    who: stkrolikiewicz
    note: >
      ADR drafted as part of task 0146. Consolidates ADR 0011's S3 offload
      sketch and ADR 0018's tx-detail field spec into a concrete JSON shape
      for live (0147) and backfill (0145) pipelines. Every field verified
      against `xdr-parser` source to confirm parser actually produces it from
      a single `LedgerCloseMeta`; fields the parser always emits as None are
      excluded, not nulled.
  - date: '2026-04-21'
    status: superseded
    who: stkrolikiewicz
    by: ['0029']
    note: >
      Superseded wholesale by ADR 0029 after the 2026-04-21 team meeting
      pivoted architecture away from parsed-ledger S3 artifacts. No bucket,
      no artifact format — write path goes directly to ADR 0027 DB (task
      0149); read path fetches raw XDR from public Stellar archive on
      demand (task 0150). The shape spec here has no consumer in the new
      architecture; kept in the repo as design archaeology.
---

# ADR 0028: ParsedLedgerArtifact v1 — canonical shape of `parsed_ledger_{seq}.json.zst`

**Related:**

- [Task 0146: Shared parsed-ledger artifact core](../1-tasks/active/0146_FEATURE_shared-parsed-ledger-artifact-core.md)
- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0012: Lightweight bridge DB schema (revision)](0012_lightweight-bridge-db-schema-revision.md)
- [ADR 0018: Minimal transaction detail to S3](0018_minimal-transactions-detail-to-s3.md)
- [ADR 0023: Tokens typed metadata columns](0023_tokens-typed-metadata-columns.md)
- [ADR 0024: Hashes as BYTEA binary storage](0024_hashes-bytea-binary-storage.md)
- [ADR 0026: Accounts surrogate BIGINT id](0026_accounts-surrogate-bigint-id.md)
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md)

---

## Context

ADR 0011 introduced the S3-offload principle and sketched the artifact
structure. ADR 0018 enumerated transaction/operation/event fields that
must live in S3 after DB-slimming columns are dropped. ADR 0027
(accepted) finalised the DB schema (18 tables) and lists per-endpoint
S3 dependencies descriptively. Neither ADR pins a concrete, field-level
JSON shape.

Task 0146 builds the Rust composition that emits the artifact. Tasks
0147 (live Galexie lambda) and 0145 (backfill runner) both consume it.
The future DB ingester reads the emitted corpus to populate the ADR 0027
tables.

Because the shape binds three pipelines and changing it later forces a
re-emit of millions of ledgers, the decision is promoted to its own ADR.

### Scope

Defines `ParsedLedgerArtifact v1`: root structure, every section's
fields, encoding rules for binaries and identifiers, versioning
semantics, and determinism requirements.

Out of scope: S3 key layout (task 0146), S3 bucket policy (CDK), DB
ingest implementation (future task), out-of-band enrichment pipelines
(price oracles, SEP-1 TOML scrape, holder-count aggregation — each
tracked by its own task).

---

## Decision

### Ground rule: XDR-derivable only

**The artifact carries only data the XDR parser can produce
deterministically from a single `LedgerCloseMeta`.**

Fields that require external sources (price oracles, SEP-1
`stellar.toml` scrape) or multi-ledger aggregation (holder counts,
rolling volume, TVL) are **not** in the artifact. Out-of-band pipelines
`UPDATE` those DB columns separately. Artifact → DB table mapping is
intentionally partial where enrichment is deferred. This keeps the
artifact deterministic, testable, and re-emittable.

Consequence: 13 DB columns (`tokens.{name, total_supply, holder_count,
description, icon_url, home_page}` — 6; `nfts.{collection_name, name,
media_url, metadata}` — 4; `liquidity_pool_snapshots.{tvl, volume,
fee_revenue}` — 3) are written by separate pipelines, not by the DB
ingester reading this artifact. Each is tracked by a dedicated task
(0124, 0125, 0135, future NFT-metadata-enrichment).

### Encoding conventions

| Domain type                                | JSON type                   | Notes                                                                                                                                                                               |
| ------------------------------------------ | --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Account ID (G-address)                     | string (56 chars)           | StrKey `G…`                                                                                                                                                                         |
| Muxed account (M-address)                  | string (69 chars) or `null` | StrKey `M…`; `null` when not muxed                                                                                                                                                  |
| Contract ID                                | string (56 chars)           | StrKey `C…`                                                                                                                                                                         |
| SHA-256 hash                               | string (64 chars)           | lowercase hex                                                                                                                                                                       |
| Pool ID (32-byte)                          | string (64 chars)           | lowercase hex                                                                                                                                                                       |
| XDR blob (envelope/result/meta)            | string                      | base64 (Stellar convention)                                                                                                                                                         |
| Classic fixed-point amount `NUMERIC(28,7)` | string                      | decimal with 7 fractional digits preserved                                                                                                                                          |
| Soroban raw i128 amount `NUMERIC(39,0)`    | string                      | decimal integer, no fraction                                                                                                                                                        |
| Ledger sequence                            | number (u32)                | JSON integer                                                                                                                                                                        |
| Unix timestamp (seconds)                   | number (i64)                | seconds since epoch, UTC                                                                                                                                                            |
| Memo value                                 | string or `null`            | encoding varies by `memo_type`; see §transactions                                                                                                                                   |
| ScVal-decoded                              | JSON value                  | produced by `xdr_parser::scval_to_typed_json`                                                                                                                                       |
| Enum/discriminator value                   | string                      | casing is field-specific; Stellar/XDR/parser-derived enums keep emitted casing (e.g. `PAYMENT`, `INVOKE_HOST_FUNCTION`, `txSUCCESS`); ADR-introduced enums use lowercase snake_case |

### Identity resolution boundary

The artifact carries **public, human-readable identifiers only**:

- Accounts/issuers/payers: StrKey `G…` (or `M…` for muxed).
- Contracts: StrKey `C…`.
- Hashes and pool IDs: hex strings.

ADR 0026's surrogate `accounts.id BIGINT` and ADR 0024's `BYTEA(32)`
storage types are **DB-local optimisations**. The DB ingester resolves
StrKey → surrogate, hex → BYTEA at write time. The artifact is never
coupled to those DB choices.

### Root structure

```json
{
  "ledger_metadata":           { ... },
  "transactions":              [ ... ],
  "account_states":            [ ... ],
  "liquidity_pools":           [ ... ],
  "liquidity_pool_snapshots":  [ ... ],
  "nft_events":                [ ... ],
  "wasm_uploads":              [ ... ],
  "contract_metadata":         [ ... ],
  "token_metadata":            [ ... ],
  "nft_metadata":              [ ... ]
}
```

All ten root keys are **always present**. Empty arrays are preserved
(not omitted) for stable shape. `ledger_metadata` is required; arrays
may be empty but must exist.

### Derivation map — DB tables with no direct artifact section

ADR 0027 has 18 tables. Ten map directly to the 10 root sections above.
Five are **derived at ingest time** from artifact data. This table is
the contract for the DB ingester:

| ADR 0027 table             | Derivation source                                                                                                                                                                                                         | Rule                                                           |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `transaction_hash_index`   | `transactions[].hash` + `ledger_metadata.{sequence, closed_at}`                                                                                                                                                           | one row per tx; 1:1 mapping                                    |
| `transaction_participants` | UNION over `transactions[]`: `source_account`, `fee_account`, `operations[].{source_account, destination}`, `invocations[].caller_account`, `events[].{transfer_from, transfer_to}`                                       | `INSERT … ON CONFLICT DO NOTHING` per `(tx, account)`          |
| `account_balances_current` | `account_states[].balances[]`                                                                                                                                                                                             | upsert with watermark on `last_updated_ledger` per PK          |
| `account_balance_history`  | `account_states[].balances[]`                                                                                                                                                                                             | `INSERT … ON CONFLICT DO NOTHING`; one row per observed change |
| `lp_positions`             | **NOT populated by artifact v1** — parser `extract_account_states` explicitly skips `pool_share` trustlines (`crates/xdr-parser/src/state.rs:225-226`); out of scope for M1 absent a parser extension (task 0126 blocked) | —                                                              |

Column-level derivations (within a single artifact):

| ADR 0027 column / consumer field                                                                                                                                                                                                                                  | Derivation source                                                                                                                        |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `transactions.ledger_sequence`, `operations.ledger_sequence`, `soroban_events.ledger_sequence`, `soroban_invocations.ledger_sequence`, `liquidity_pool_snapshots.ledger_sequence`                                                                                 | `ledger_metadata.sequence` (root)                                                                                                        |
| `transactions.created_at`, `operations.created_at`, `soroban_events.created_at`, `soroban_invocations.created_at`, `liquidity_pool_snapshots.created_at`, `transaction_participants.created_at`, `nft_ownership.created_at`, `account_balance_history.created_at` | `ledger_metadata.closed_at` (root) converted to TIMESTAMPTZ                                                                              |
| `transactions.operation_count`                                                                                                                                                                                                                                    | `transactions[i].operations.length`                                                                                                      |
| `transactions.has_soroban`                                                                                                                                                                                                                                        | `transactions[i].operations.any(op_type == "INVOKE_HOST_FUNCTION")` — ingester populates so partial index `idx_tx_has_soroban` is filled |
| `soroban_events.topic0`                                                                                                                                                                                                                                           | first element of `events[].topics[]`, lifted as text (one JSON-path access per row)                                                      |
| `nft_ownership.event_order`                                                                                                                                                                                                                                       | 0-based index of entry within the `nft_events[]` array                                                                                   |
| `is_fee_bump` (consumer display, not a DB column)                                                                                                                                                                                                                 | `transactions[i].inner_tx_hash IS NOT NULL`                                                                                              |
| `invocation.depth` (consumer display, not a DB column)                                                                                                                                                                                                            | walk `invocations[].parent_index` chain from each node                                                                                   |

Cross-ledger derivation:

| ADR 0027 column                             | Derivation                                                                                                                                                                                         |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `soroban_contracts.wasm_uploaded_at_ledger` | ingester records `ledger_metadata.sequence` on first observation of a given `wasm_hash` in `wasm_uploads[]`; a later contract deploy referencing that hash reads the recorded value at INSERT time |

### `ledger_metadata`

```json
{
  "schema_version": "v1",
  "sequence": 53795300,
  "hash": "64hex",
  "closed_at": 1708448400,
  "protocol_version": 20,
  "transaction_count": 142,
  "base_fee": 100
}
```

| Field               | Type         | Notes                          |
| ------------------- | ------------ | ------------------------------ |
| `schema_version`    | string       | **always `"v1"`** for this ADR |
| `sequence`          | u32          | ledger sequence                |
| `hash`              | hex          | ledger header hash             |
| `closed_at`         | unix seconds | ledger close time, UTC         |
| `protocol_version`  | u32          | Stellar protocol version       |
| `transaction_count` | u32          | transactions in this ledger    |
| `base_fee`          | u32          | base fee in stroops            |

`schema_version` lives here (not at root) so a future v2 can extend
the root with new sections without moving the version tag.

### `transactions[]`

```json
{
  "hash":                 "64hex",
  "application_order":    0,
  "source_account":       "G…",
  "source_account_muxed": "M…" | null,
  "fee_account":          "G…" | null,
  "fee_account_muxed":    "M…" | null,
  "inner_tx_hash":        "64hex" | null,
  "fee_charged":          100,
  "successful":           true,
  "result_code":          "txSUCCESS",
  "memo_type":            "text" | "id" | "hash" | "return" | "none",
  "memo":                 string | null,
  "envelope_xdr":         "base64",
  "result_xdr":           "base64",
  "result_meta_xdr":      "base64" | null,
  "signatures":           [ ... ],
  "operations":           [ ... ],
  "events":               [ ... ],
  "invocations":          [ ... ],
  "ledger_entry_changes": [ ... ],
  "parse_error":          false
}
```

**`memo` encoding by `memo_type`:**

| `memo_type` | `memo` value               |
| ----------- | -------------------------- |
| `"none"`    | `null`                     |
| `"text"`    | UTF-8 string               |
| `"id"`      | decimal string (u64 range) |
| `"hash"`    | hex (64 chars)             |
| `"return"`  | hex (64 chars)             |

**`signatures[]`:**

```json
{ "hint": "8hex", "signature": "base64" }
```

`hint` = ed25519 public-key hint (4 bytes → 8 hex chars).
`signature` = raw signature (64 bytes → base64).

**`parse_error`**: `true` if any sub-extraction for this tx failed.
When true, sub-arrays (`operations`, `events`, `invocations`,
`ledger_entry_changes`) may be incomplete but MUST be present
(possibly empty).

**Builder-computed fields** (not in `ExtractedTransaction` struct;
derived by the task 0146 builder from parser-accessible data):

- `application_order` — 1-based tx index within the ledger (matches the
  Stellar ecosystem convention used by Horizon `paging_token`,
  stellar-core, stellar.expert; bumped from 0-based per task **0172**).
- `source_account_muxed`, `fee_account`, `fee_account_muxed` — derived
  from the `TransactionEnvelope` returned by `extract_envelopes`; the
  builder matches on `TxV0` / `TxV1` / `TxFeeBump` variants and
  extracts MuxedAccount fields.
- `inner_tx_hash` — for fee-bump transactions (`TxFeeBump` variant),
  the builder computes SHA-256 over the XDR encoding of the inner
  transaction envelope (`FeeBumpTransactionInnerTx::Tx` branch). The
  parser crate already depends on `sha2` and `stellar-xdr` so no new
  deps. Null for non-fee-bump transactions.

**Fields derived at ingest (not in artifact)**: `ledger_sequence`,
`is_fee_bump`, `operation_count`, `has_soroban` — see Derivation map.

### `transactions[].operations[]`

```json
{
  "application_order":    0,
  "op_type":              "PAYMENT" | "INVOKE_HOST_FUNCTION" | ...,
  "source_account":       "G…" | null,
  "source_account_muxed": "M…" | null,
  "destination":          "G…" | null,
  "destination_muxed":    "M…" | null,
  "contract_id":          "C…" | null,
  "asset_code":           string | null,
  "asset_issuer":         "G…" | null,
  "pool_id":              "64hex" | null,
  "function_name":        string | null,
  "transfer_amount":      "100.0000000" | null,
  "details":              { type-specific JSON }
}
```

`source_account` is `null` when the op inherits the transaction source
(Stellar semantics). `transfer_amount` is a `NUMERIC(28,7)` decimal
string extracted per ADR 0018 (PAYMENT amount, PATH_PAYMENT destination
amount, LP_DEPOSIT/WITHDRAW asset A amount, CREATE_ACCOUNT
starting_balance, else `null`).

`asset_code` / `asset_issuer` / `pool_id` are ADR 0027 §5 first-class
filter columns — carried per-op so DB ingester populates without
decoding `details`.

`details` is type-specific ScVal-decoded JSON (not normalised).

### `transactions[].events[]`

```json
{
  "event_index":     0,
  "event_type":      "contract" | "system" | "diagnostic",
  "contract_id":     "C…" | null,
  "topics":          [ ScVal-decoded JSON, ... ],
  "data":            ScVal-decoded JSON,
  "transfer_from":   "G…" | null,
  "transfer_to":     "G…" | null,
  "transfer_amount": "100" | null
}
```

`transfer_from` / `transfer_to` / `transfer_amount` are populated only
when the first topic is `"transfer"`, `"mint"`, or `"burn"` per ADR 0018. `transfer_amount` here is `NUMERIC(39,0)` — Soroban raw i128
decimal, no fraction (distinct from `operations[].transfer_amount`
which is classic fixed-point).

ADR 0027 §9 `soroban_events.topic0` is populated by the DB ingester
from `topics[0]` at insert time (see Derivation map).

### `transactions[].invocations[]`

Flat list with `parent_index` — avoids nested objects, mirrors ADR 0027
§10 table shape.

```json
{
  "invocation_index": 0,
  "parent_index":     null | u32,
  "contract_id":      "C…" | null,
  "caller_account":   "G…" | null,
  "function_name":    string,
  "function_args":    ScVal-decoded JSON,
  "return_value":     ScVal-decoded JSON | null,
  "successful":       true
}
```

Root invocations have `parent_index: null`. Consumers needing a
depth value reconstruct it with a single O(N) walk over
`parent_index` chain from each node — not in ADR 0027 §10 schema,
not carried in the artifact.

**`function_name` is always present.** Parser emits sentinel strings
`"createContract"` / `"createContractV2"` for contract-creation
invocations (`crates/xdr-parser/src/invocation.rs:251,267`). No
nullability mismatch with ADR 0027 §10 `NOT NULL`. Despite the
`Option<String>` typing on `ExtractedInvocation.function_name`, the
`None` branch is never taken in practice; the artifact builder may
assert this at serialization time.

### `transactions[].ledger_entry_changes[]`

One entry per `LedgerEntryChange` from `TransactionMeta` V3/V4.

```json
{
  "change_index":    0,
  "change_type":     "created" | "updated" | "removed" | "state" | "restored",
  "entry_type":      "account" | "trustline" | "offer" | "data" | "claimable_balance" | "liquidity_pool" | "contract_data" | "contract_code" | "config_setting" | "ttl",
  "key":             JSON,
  "data":            JSON | null,
  "operation_index": u32 | null
}
```

`operation_index: null` for tx-level changes (`tx_changes_before` /
`tx_changes_after`). `data: null` for `"removed"` changes (only key
available).

### `account_states[]`

One entry per account whose state changed in this ledger.

```json
{
  "account_id":          "G…",
  "first_seen_ledger":   u32 | null,
  "last_seen_ledger":    u32,
  "sequence_number":     "decimal string",
  "home_domain":         string | null,
  "balances":            [ ... ],
  "removed_trustlines":  [ ... ]
}
```

`first_seen_ledger` is `null` for existing accounts (state change, not
creation). `sequence_number` is stringified because DB stores `BIGINT`;
though i64 fits in JSON number safe range today, stringification
eliminates future parser ambiguity.

#### `account_states[].balances[]` shape

Identity rules match ADR 0027 §17 `account_balances_current` CHECK
constraint:

| `asset_type` | `asset_code` | `issuer_address`  | routes to                                                      |
| ------------ | ------------ | ----------------- | -------------------------------------------------------------- |
| `native`     | `null`       | `null`            | `account_balances_current`                                     |
| `classic`    | required     | required (G-addr) | `account_balances_current` (covers SAC classic-side trustline) |

```json
{
  "asset_type":          "native" | "classic",
  "asset_code":          string | null,
  "issuer_address":      "G…" | null,
  "balance":             "0.0000000",
  "last_updated_ledger": u32
}
```

`balance` is `NUMERIC(28,7)` (classic fixed-point stroops).

**Pool-share trustlines are NOT in `balances[]`** despite being
structurally trustline ledger entries in Stellar. Parser
`extract_account_states` explicitly skips them
(`crates/xdr-parser/src/state.rs:225-226`) because they do not map to
`account_balances_current` (which has no `pool_id` column); `lp_positions`
tracks them instead. See the Derivation map — `lp_positions` is out of
scope for ADR 0028 v1 pending task 0126 (`pool-participants-tracking`,
blocked) which extends the parser with a dedicated extraction path.

**Pure Soroban token balances (per-account contract storage)** are
**out of scope for v1**. Task 0138 (`contract-token-balance-extraction`,
backlog) handles adding a dedicated section when the parser gains
contract_data balance extraction. Until then, `account_states[]` does
not track Soroban token balances.

#### `account_states[].removed_trustlines[]` shape

```json
{
  "asset_type":     "classic",
  "asset_code":     string,
  "issuer_address": "G…"
}
```

Tracked separately from `balances[]` so DB ingester can produce
`account_balance_history` removal rows without polluting the
current-balance upsert path.

### `liquidity_pools[]`

```json
{
  "pool_id":           "64hex",
  "asset_a_type":      "native" | "classic" | "sac" | "soroban",
  "asset_a_code":      string | null,
  "asset_a_issuer":    "G…" | null,
  "asset_b_type":      "native" | "classic" | "sac" | "soroban",
  "asset_b_code":      string | null,
  "asset_b_issuer":    "G…" | null,
  "fee_bps":           30,
  "created_at_ledger": u32 | null
}
```

`created_at_ledger: null` for pre-existing pools observed (state
change, not creation). DB ingester uses `INSERT … ON CONFLICT DO
NOTHING`; first sighting sets the column.

### `liquidity_pool_snapshots[]`

```json
{
  "pool_id": "64hex",
  "reserve_a": "0.0000000",
  "reserve_b": "0.0000000",
  "total_shares": "0.0000000"
}
```

All numeric fields are `NUMERIC(28,7)` decimal strings. Only
XDR-derivable fields carried.

**Fields populated out-of-band**: `tvl`, `volume`, `fee_revenue` (all
three `liquidity_pool_snapshots` NUMERIC(28,7) columns in ADR 0027
§15). Parser `extract_liquidity_pools` always emits these as `None`
(confirmed: `crates/xdr-parser/src/state.rs:447-449`). Task 0125
(`lp-price-oracle-tvl-volume`, backlog) handles the enrichment
pipeline that `UPDATE`s those DB columns.

### `nft_events[]`

```json
{
  "transaction_hash": "64hex",
  "contract_id":      "C…",
  "event_kind":       "mint" | "transfer" | "burn",
  "token_id":         ScVal-decoded JSON,
  "from":             "G…" | null,
  "to":               "G…" | null
}
```

`from` is `null` for `mint`. `to` is `null` for `burn`.

**`event_order`** (required by ADR 0027 §13 `nft_ownership.event_order
SMALLINT NOT NULL`): derived by DB ingester as the 0-based array index
within `nft_events[]`. Deterministic because the parser emits events in
ledger order; task 0146's determinism test asserts stable ordering.

### `wasm_uploads[]`

```json
{
  "wasm_hash":     "64hex",
  "wasm_byte_len": 12345,
  "functions": [
    {
      "name":    "transfer",
      "doc":     "string",
      "inputs":  [ { "name": "from", "type_name": "Address" }, ... ],
      "outputs": [ "i128" ]
    }
  ]
}
```

`soroban_contracts.wasm_uploaded_at_ledger` is populated at ingest
from `ledger_metadata.sequence` of the artifact in which the
`wasm_hash` first appears — see Derivation map. No per-entry
`uploaded_at_ledger` field in the artifact; the enclosing ledger IS
the answer.

### `contract_metadata[]`

```json
{
  "contract_id":        "C…",
  "wasm_hash":          "64hex" | null,
  "deployer_account":   "G…" | null,
  "deployed_at_ledger": u32,
  "contract_type":      "token" | "dex" | "lending" | "nft" | "other",
  "is_sac":             false,
  "metadata":           JSON
}
```

`metadata` is an opaque JSON object (matches ADR 0027 §7
`soroban_contracts.metadata JSONB`). Consumer convention: display name
lives at `metadata.name` (the generated `search_vector` column reads
`metadata->>'name'`).

### `token_metadata[]`

One entry per token identity first-seen this ledger. Identity rules
mirror ADR 0027 §11 `ck_tokens_identity` CHECK:

| `asset_type` | `asset_code` | `issuer_address`  | `contract_id`     |
| ------------ | ------------ | ----------------- | ----------------- |
| `native`     | `null`       | `null`            | `null`            |
| `classic`    | required     | required (G-addr) | `null`            |
| `sac`        | required     | required (G-addr) | required (C-addr) |
| `soroban`    | `null`       | `null`            | required (C-addr) |

```json
{
  "asset_type":     "native" | "classic" | "sac" | "soroban",
  "asset_code":     string | null,
  "issuer_address": "G…" | null,
  "contract_id":    "C…" | null
}
```

**Fields populated out-of-band**: `name`, `total_supply`,
`holder_count`, `description`, `icon_url`, `home_page` (all present on
ADR 0027 §11 `tokens` table). Parser `detect_tokens` always emits
`name`, `total_supply`, `holder_count` as `None` (confirmed:
`crates/xdr-parser/src/state.rs:477-479`); the SEP-1 TOML fields
(`description`, `icon_url`, `home_page`) are never produced by the
parser. Separate pipelines handle them:

- Task 0124 (`token-metadata-enrichment`, backlog) for name /
  description / icon_url / home_page.
- Task 0135 (`token-holder-count-tracking`, active) for holder_count.
- Total supply: pending a dedicated task (future scope; not in v1).

### `nft_metadata[]`

```json
{
  "contract_id":          "C…",
  "token_id":             string,
  "owner_account":        "G…" | null,
  "minted_at_ledger":     u32 | null,
  "current_owner_ledger": u32
}
```

`owner_account` + `current_owner_ledger` match ADR 0027 §12
`nfts.current_owner_id` + `current_owner_ledger` columns (source:
parser `ExtractedNft.last_seen_ledger` → `current_owner_ledger`).

**Fields populated out-of-band**: `collection_name`, `name`,
`media_url`, `metadata` (all present on ADR 0027 §12 `nfts` table).
Parser `detect_nfts` always emits these as `None` (confirmed:
`crates/xdr-parser/src/state.rs:513-517`). A future
NFT-metadata-enrichment task handles these via SEP-0050 contract
metadata calls.

**`token_id` format note**: `nft_metadata[].token_id` is a string
(stringified form for the ADR 0027 §12 `token_id VARCHAR(256)`
column), while `nft_events[].token_id` is the full ScVal-decoded JSON.
Two forms coexist in the artifact because parser `ExtractedNft` and
`NftEvent` emit different representations of the same concept.

### Determinism requirements

1. **Field order within objects**: struct declaration order (serde
   default). Matches the order in this ADR.
2. **Array order**:
   - `transactions[]`: by `application_order` ascending.
   - `operations[]`: by `application_order` ascending.
   - `events[]`: by `event_index` ascending.
   - `invocations[]`: by `invocation_index` ascending (depth-first).
   - `ledger_entry_changes[]`: by `change_index` ascending.
   - `account_states[]`, `account_states[].balances[]`,
     `liquidity_pools[]`, `liquidity_pool_snapshots[]`, `nft_events[]`,
     `wasm_uploads[]`, `contract_metadata[]`, `token_metadata[]`,
     `nft_metadata[]`: stable order as produced by the corresponding
     `extract_*` function.
3. **Serialization**: `serde_json::to_vec` default, no pretty printing,
   no trailing newline.
4. **Re-run equivalence**: building the artifact twice from the same
   `LedgerCloseMeta` MUST produce byte-identical output. Enforced by
   task 0146's golden-fixture test.

### Versioning

- `ledger_metadata.schema_version` identifies the artifact version.
- v1 = this ADR.
- **Breaking change** (rename, type change, semantic change, field
  removal) requires:
  1. A new ADR superseding this one.
  2. A new `schema_version` value (`"v2"`, …).
  3. Re-emit of the corpus (or dual-version tolerance in consumers —
     typically not worth it).
- **Additive change** (new field, new enum value, new op_type) does NOT
  bump the version. Consumers MUST ignore unknown fields and unknown
  enum values gracefully. Examples that would be additive: adding a
  `soroban_token_balances[]` section when task 0138 lands; adding
  `tvl` to snapshots if task 0125 lands within the v1 window.

### S3 key layout (reference — defined by task 0146)

```
parsed-ledgers/v1/{partition_start}-{partition_end}/parsed_ledger_{sequence}.json.zst
```

64k-ledger partitions. The `v1` path segment mirrors `schema_version`
so a re-emit at v2 can coexist under `parsed-ledgers/v2/…`.

---

## Rationale

### Why XDR-derivable only

Enrichment data (token names from TOML, TVL from price oracles,
holder_count aggregations) has different cadence, different failure
modes, and different trust boundaries than XDR parsing. Mixing them in
one pipeline couples failure domains: a broken TOML scraper would
require re-emitting artifacts. Keeping the artifact a pure view of XDR
state lets each enrichment pipeline fail and retry independently; they
write to DB columns that the artifact never touches.

Always-null placeholder fields would add noise (tens of bytes per
entity × millions of entities) and mislead reviewers into thinking the
artifact is the source of those values.

### Why StrKey (not surrogate BIGINT) for accounts/contracts

The artifact is the public payload consumed by the DB ingester and
potentially by external readers. StrKey is Stellar's canonical
human-readable form; surrogates are a DB-local optimisation (ADR 0026)
resolved at ingest. Surrogates in JSON would leak a storage-layer
concern into the wire format and force external consumers to maintain
a parallel account table.

### Why hex for hashes and pool IDs

64-character hex is grep-friendly, matches conventional SHA-256
display, aligns with Stellar SDK/Horizon conventions. Base64 would save
~25% size but loses inspection usability. DB storage as BYTEA (ADR 0024) is a storage optimisation, not a wire format.

### Why base64 for XDR blobs

Matches Stellar ecosystem convention (SDKs, Horizon, CLI tools use
base64 for XDR). Hex would inflate 2× unnecessarily. Consumers
deserialize XDR via `stellar-xdr` which expects base64.

### Why split NUMERIC(28,7) and NUMERIC(39,0) encoding rules

ADR 0027 uses two distinct precisions:

- Classic amounts (stroops): `NUMERIC(28,7)` — 7 decimal places.
- Soroban i128 raw balances: `NUMERIC(39,0)` — integer.

Conflating them would let consumers mis-route values into the wrong DB
column width. Separate encoding rows surface the distinction.

### Why Unix seconds for timestamps

Deterministic, compact, no timezone ambiguity. Consumers needing ISO
8601 use one line of conversion. Going the other way (ISO 8601 →
integer) is less safe.

### Why empty arrays preserved

Stable field presence simplifies consumer code: `artifact.events.len()`
always works. Serialized cost is trivial (empty array = 2 bytes
pre-compression).

### Why flat `invocations[]` with `parent_index`

Mirrors ADR 0027 §10 table exactly — DB ingester INSERTs rows without
restructuring. Tree reconstruction consumer-side is a single linear
scan on `parent_index`. Parser-produced `operation_tree` JSON is not
carried in the artifact: consumers with the flat list + `parent_index`
can reconstruct it trivially, and the DB has no `operation_tree`
column.

### Why trivially-derivable fields are excluded from the artifact

The artifact omits fields whose value is either a **constant copy from
`ledger_metadata`** or an **O(1)/O(N) simple derivation from sibling
artifact data**, unless pre-extraction enables a genuine hot-path query
optimisation (such as populating an indexed DB column that would
otherwise require JSON-path parsing at insert time).

Applying this rule:

- **Removed**: `transactions[].{ledger_sequence, is_fee_bump,
operation_count}`, `transactions[].invocations[].depth`,
  `events[].topic0`, `liquidity_pool_snapshots[].ledger_sequence`,
  `wasm_uploads[].uploaded_at_ledger`. Each is either a root copy, a
  null check, an array length, or a single JSON-path access.
- **Removed**: `has_soroban` (O(N) scan over operations) — same rule
  class; ingester populates the indexed DB column by scanning
  `operations[]` at INSERT time. The partial index `idx_tx_has_soroban`
  depends on the DB column being populated, not on the artifact
  carrying the boolean.
- **Kept**: `operations[].{asset_code, asset_issuer, pool_id}`,
  `events[].{transfer_from, transfer_to, transfer_amount}`, and
  `operations[].transfer_amount`. These require domain knowledge
  (per-op-type field dispatch inside `details` JSON) or avoid
  per-insert JSON-path scans for indexed filter columns — genuine
  pre-extraction value.

Aggregate saving across the corpus (10M+ ledgers, avg ~200 tx/ledger):
~5 KB/ledger pre-compression → ~50 GB total, ~15-20 GB post-zstd. More
importantly: cleaner contract for external consumers, fewer "looks
redundant, is it really the same?" questions during review, and
principled rule that scales as new fields are proposed.

### Why artifact carries full XDR blobs AND extracted fields

Belt-and-suspenders:

- Extracted fields serve ADR 0027 fast endpoints.
- XDR blobs enable forensic replay (reparse with a future parser
  version) and satisfy consumers outside our pipeline who prefer
  canonical XDR.

zstd compresses XDR blobs well — acceptable size cost.

### Why derivation map (5 tables) instead of direct sections

Five ADR 0027 tables are pure ingest-time views over other artifact
data:

- `transaction_hash_index`: trivially per-tx.
- `transaction_participants`: UNION over tx/ops/events/invocations.
- `lp_positions` + `account_balances_current` +
  `account_balance_history`: routed from `account_states[].balances[]`
  by `asset_type`.

Emitting them as redundant sections would:

- double the artifact size for `transaction_participants`.
- create two sources of truth for balances (history and current are
  materialised views of the same change stream).
- tempt the parser to compute them inconsistently with the sections
  they derive from.

Keeping them derivable at ingest centralises the derivation rule and
matches ADR 0027's view that these tables are optimisations, not
independent data.

### Why `event_order` is array-index-derived

ADR 0027 §13 requires `event_order SMALLINT NOT NULL`. Parser
`NftEvent` struct does not carry `event_order`. Deriving from array
index is safe: parser emits events in ledger order (deterministic);
determinism test ensures stable ordering.

### Why `invocations[].function_name` is non-null

Earlier drafts flagged a mismatch: `ExtractedInvocation.function_name`
is typed `Option<String>`, suggesting `None` was possible for
contract-creation invocations. Actual parser behaviour
(`crates/xdr-parser/src/invocation.rs:251,267`) emits the sentinels
`"createContract"` and `"createContractV2"` for those paths — the
`None` branch is never taken. Artifact types `function_name: string`
without the null option; ADR 0027 §10 `NOT NULL` is satisfied without
ingester sentinel substitution.

`operations[].function_name` remains `string | null` because per ADR
0018 it is emitted only for `INVOKE_HOST_FUNCTION`; null for other op
types.

---

## Alternatives Considered

### Alternative 1: Surrogate IDs in artifact JSON

**Description:** Emit `source_id: 12345` instead of `source_account: "G…"`.

**Pros:** No StrKey→id resolution at ingest; smaller JSON.

**Cons:** Couples artifact to DB schema (ADR 0026). External consumers
must maintain a parallel account table. Re-emit on any accounts reorg.
Defeats "portable canonical corpus."

**Decision:** REJECTED.

### Alternative 2: Binary format (CBOR / MessagePack / Protobuf)

**Description:** Emit `.cbor.zst` or `.pb.zst` instead of JSON.

**Pros:** ~30-50% smaller pre-compression; faster parse.

**Cons:** Loses grep/jq/ad-hoc inspection. zstd narrows the size gap.

**Decision:** REJECTED for v1.

### Alternative 3: Batch file per Galexie `.xdr.zst` batch

**Description:** One artifact per Galexie batch (multiple ledgers).

**Pros:** Fewer S3 objects, fewer HEAD requests.

**Cons:** Resume logic harder; DB ingester can't parallelise per
ledger; batch granularity leaks Galexie implementation.

**Decision:** REJECTED — per-ledger matches ADR 0011.

### Alternative 4: Omit `result_meta_xdr`

**Description:** Carry extracted fields only; no raw meta blob.

**Pros:** Shaves the biggest field per transaction.

**Cons:** Loses forensic replay. Re-fetch from Galexie raw bucket is
expensive at historical scale.

**Decision:** REJECTED — keep raw XDR.

### Alternative 5: Nested transaction structure with `detail` sub-object

**Description:** `transactions[i] = { hash, ledger_sequence, detail: { memo, signatures, ... } }`.

**Pros:** Separates "list columns" from "detail-only" fields.

**Cons:** List/detail split is a DB concern, not a wire-format concern.
ADR 0018 lists individual fields as peers.

**Decision:** REJECTED — flat structure.

### Alternative 6: `transaction_participants[]` inline per tx

**Description:** Flat array of unique G-addresses per tx.

**Pros:** Trivial DB ingest — no UNION needed.

**Cons:** Redundant with existing fields. Two sources of truth.

**Decision:** REJECTED — derivation at ingest time (in Derivation map).

### Alternative 7: Nullable placeholders for out-of-band fields

**Description:** Keep `name`, `total_supply`, `holder_count`, `tvl`,
`volume`, `fee_revenue`, etc. as `null`-always fields.

**Pros:** Artifact → DB row mapping is bijective field-wise.

**Cons:** ~10 always-null fields × millions of records. Creates false
expectation that artifact carries this data. Couples artifact test
code to fields the parser never populates. Violates "XDR-derivable
only" ground rule.

**Decision:** REJECTED — excluded outright. Out-of-band pipelines own
those columns.

---

## Consequences

### Positive

- Single frozen contract for live lambda (0147), backfill (0145), and
  future DB ingester.
- Byte-identical output between live and backfill paths (determinism
  test in task 0146).
- Forensic replay possible — raw XDR blobs carried.
- Version-tagged — v2 can coexist under `parsed-ledgers/v2/`.
- External consumers can read the bucket with standard JSON tooling.
- Shape mirrors ADR 0027 closely — DB ingester is mechanical mapping +
  5 documented ingest-time derivations.
- Every parser-produced field has a home; every non-parser field is
  explicitly excluded and attributed to its out-of-band pipeline.
- 17/18 ADR 0027 tables covered: 10 direct, 4 derived (Derivation
  map: `transaction_hash_index`, `transaction_participants`,
  `account_balances_current`, `account_balance_history`), 3
  enrichment-dependent (`tokens`, `nfts`, `liquidity_pool_snapshots`
  partial columns owned by tasks 0124/0125/0135 + future NFT metadata
  task). The remaining **`lp_positions` is explicitly out of scope for
  v1** — parser does not emit pool-share trustlines today (skipped at
  `state.rs:225-226`); blocked by task 0126.

### Negative

- Artifact size: StrKey + hex is bulkier than binary. Mitigation: zstd
  level 3 compresses well; measured ratio ~3-5× on mainnet ledgers.
- XDR blobs double-represent data (parsed fields + raw blob). Accepted
  for forensic replay.
- Additive-only v1 discipline requires reviewer vigilance — tempting
  "small fixes" that rename fields must be flagged as version bumps.
- `tokens`, `nfts`, `liquidity_pool_snapshots` will show partially
  populated rows during the transition window (v1 artifact ingester
  running, enrichment pipelines not yet deployed). Each enrichment
  task must handle the `NULL → value` `UPDATE` path cleanly.
- `lp_positions` remains empty for the M1 window (endpoint E19 "pool
  participants" returns no rows). Closing requires task 0126
  unblock: parser extension to emit pool-share trustlines + an
  additive artifact v1 amendment (new section or re-allowed
  `asset_type = "pool"` in `balances[]`).
- `transactions.inner_tx_hash` is populated by the artifact builder
  via SHA-256 of the inner transaction XDR encoding (see §transactions
  builder-computed fields). If the PR 2/3 builder implementation
  defers the hash computation, rows start NULL; the UI fee-bump
  detail link depends on it, so treat as a PR 2/3 acceptance criterion.

---

## Open questions (for review before PR 1 freeze in task 0146)

1. **`ledger_entry_changes[]` placement** (per-tx vs root-level):
   per-tx chosen because entry changes carry `operation_index` tying
   them to operations. Confirm before freeze.
2. **`contract_metadata[].metadata` shape standardisation**: SEP-47
   proposes a schema; currently unstable. Pass-through.
3. **`nft_metadata[].metadata` (future)**: SEP-0050 deliberately
   unstructured. Once the NFT-metadata-enrichment task adds the field
   back to the artifact (or an adjacent section), decide whether to
   pass through raw or normalise.
4. **WASM bytecode in `wasm_uploads[]`**: include raw bytes (base64)?
   Default: **exclude** — tens to hundreds of KB per upload,
   refetchable from chain if needed.
5. **ScVal-decoded JSON determinism**: does `scval_to_typed_json`
   produce deterministic insertion order? Task 0146 determinism test
   catches drift.
6. **`nft_events[].token_id` vs `nft_metadata[].token_id` form
   mismatch** (JSON vs string): accept as parser reality in v1, or ask
   parser to unify? v2 could unify once direction is chosen.

---

## References

- [ADR 0011: S3 offload — lightweight DB schema](0011_s3-offload-lightweight-db-schema.md)
- [ADR 0018: Minimal transactions detail to S3](0018_minimal-transactions-detail-to-s3.md)
- [ADR 0023: Tokens typed metadata columns](0023_tokens-typed-metadata-columns.md)
- [ADR 0024: Hashes as BYTEA binary storage](0024_hashes-bytea-binary-storage.md)
- [ADR 0026: Accounts surrogate BIGINT id](0026_accounts-surrogate-bigint-id.md)
- [ADR 0027: Post-surrogate schema + endpoint realizability](0027_post-surrogate-schema-and-endpoint-realizability.md)
- [SEP-0001: Stellar.toml](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md)
- [SEP-0023: Muxed Accounts](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0023.md)
- [SEP-0041: Soroban Token Standard](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [SEP-0050: Non-Fungible Tokens](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md)
