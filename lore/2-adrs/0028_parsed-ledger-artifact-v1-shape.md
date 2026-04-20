---
id: '0028'
title: 'ParsedLedgerArtifact v1 — canonical shape of parsed_ledger_{seq}.json'
status: proposed
deciders: [stkrolikiewicz]
related_tasks: ['0146', '0145', '0147']
related_adrs: ['0011', '0012', '0018', '0023', '0024', '0026', '0027']
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
---

# ADR 0028: ParsedLedgerArtifact v1 — canonical shape of `parsed_ledger_{seq}.json`

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

Consequence: 10 DB columns (`tokens.{name, total_supply, holder_count,
description, icon_url, home_page}`, `nfts.{collection_name, name,
media_url, metadata}`, `liquidity_pool_snapshots.{tvl, volume,
fee_revenue}`) are written by separate pipelines, not by the DB
ingester reading this artifact. Each is tracked by a dedicated task
(0124, 0125, 0135, future NFT-metadata-enrichment).

### Encoding conventions

| Domain type                                | JSON type                   | Notes                                             |
| ------------------------------------------ | --------------------------- | ------------------------------------------------- |
| Account ID (G-address)                     | string (56 chars)           | StrKey `G…`                                       |
| Muxed account (M-address)                  | string (69 chars) or `null` | StrKey `M…`; `null` when not muxed                |
| Contract ID                                | string (56 chars)           | StrKey `C…`                                       |
| SHA-256 hash                               | string (64 chars)           | lowercase hex                                     |
| Pool ID (32-byte)                          | string (64 chars)           | lowercase hex                                     |
| XDR blob (envelope/result/meta)            | string                      | base64 (Stellar convention)                       |
| Classic fixed-point amount `NUMERIC(28,7)` | string                      | decimal with 7 fractional digits preserved        |
| Soroban raw i128 amount `NUMERIC(39,0)`    | string                      | decimal integer, no fraction                      |
| Ledger sequence                            | number (u32)                | JSON integer                                      |
| Unix timestamp (seconds)                   | number (i64)                | seconds since epoch, UTC                          |
| Memo value                                 | string or `null`            | encoding varies by `memo_type`; see §transactions |
| ScVal-decoded                              | JSON value                  | produced by `xdr_parser::scval_to_typed_json`     |
| Enum discriminator                         | string                      | lowercase snake_case                              |

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

| ADR 0027 table             | Derivation source                                                                                                                                                                   | Rule                                                                       |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `transaction_hash_index`   | `transactions[].hash` + `ledger_metadata.{sequence, closed_at}`                                                                                                                     | one row per tx; 1:1 mapping                                                |
| `transaction_participants` | UNION over `transactions[]`: `source_account`, `fee_account`, `operations[].{source_account, destination}`, `invocations[].caller_account`, `events[].{transfer_from, transfer_to}` | `INSERT … ON CONFLICT DO NOTHING` per `(tx, account)`                      |
| `lp_positions`             | `account_states[].balances[]` where `asset_type = "pool"`                                                                                                                           | upsert with watermark on `last_updated_ledger` per `(pool_id, account_id)` |
| `account_balances_current` | `account_states[].balances[]` where `asset_type ≠ "pool"`                                                                                                                           | upsert with watermark on `last_updated_ledger` per PK                      |
| `account_balance_history`  | `account_states[].balances[]` where `asset_type ≠ "pool"`                                                                                                                           | `INSERT … ON CONFLICT DO NOTHING`; one row per observed change             |

Cross-ledger derivation:

| ADR 0027 column                             | Derivation                                                                                                                     |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `soroban_contracts.wasm_uploaded_at_ledger` | lookup `wasm_uploads[].uploaded_at_ledger` from a prior artifact whose `wasm_uploads[]` first contained the target `wasm_hash` |

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
  "ledger_sequence":      53795300,
  "application_order":    0,
  "source_account":       "G…",
  "source_account_muxed": "M…" | null,
  "fee_account":          "G…" | null,
  "fee_account_muxed":    "M…" | null,
  "is_fee_bump":          false,
  "inner_tx_hash":        "64hex" | null,
  "fee_charged":          100,
  "successful":           true,
  "result_code":          "txSUCCESS",
  "operation_count":      3,
  "has_soroban":          true,
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

- `application_order` — 0-based tx index within the ledger.
- `operation_count` — `operations.len()`.
- `has_soroban` — true if any `operations[].op_type == "INVOKE_HOST_FUNCTION"`.
- `source_account_muxed`, `fee_account`, `fee_account_muxed`,
  `inner_tx_hash`, `is_fee_bump` — derived from the
  `TransactionEnvelope` returned by `extract_envelopes`; the builder
  matches on `TxV0` / `TxV1` / `TxFeeBump` variants and extracts
  MuxedAccount / inner-tx fields.

`ledger_sequence` is redundant with `ledger_metadata.sequence` but
mirrors ADR 0027 §3's per-tx column for trivial DB ingest.

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
  "topic0":          string | null,
  "topics":          [ ScVal-decoded JSON, ... ],
  "data":            ScVal-decoded JSON,
  "transfer_from":   "G…" | null,
  "transfer_to":     "G…" | null,
  "transfer_amount": "100" | null
}
```

`topic0` is the first topic's symbol name lifted to scalar (matches ADR
0027 `soroban_events.topic0`).

`transfer_from` / `transfer_to` / `transfer_amount` are populated only
when `topic0 IN ('transfer', 'mint', 'burn')` per ADR 0018.
`transfer_amount` here is `NUMERIC(39,0)` — Soroban raw i128 decimal,
no fraction (distinct from `operations[].transfer_amount` which is
classic fixed-point).

### `transactions[].invocations[]`

Flat list with `parent_index` — avoids nested objects, mirrors ADR 0027
§10 table shape.

```json
{
  "invocation_index": 0,
  "parent_index":     null | u32,
  "depth":            0,
  "contract_id":      "C…" | null,
  "caller_account":   "G…" | null,
  "function_name":    string | null,
  "function_args":    ScVal-decoded JSON,
  "return_value":     ScVal-decoded JSON | null,
  "successful":       true
}
```

Root invocations: `parent_index: null`, `depth: 0`. `depth` is
redundant with `parent_index` but kept for read-side convenience.

**Known mismatch**: ADR 0027 §10 declares `function_name NOT NULL`.
`ExtractedInvocation.function_name` is `Option<String>` (None for
contract-creation invocations). Artifact faithfully carries `null`;
ingester decides policy (substitute sentinel, or skip row).

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
constraint plus the pool-share special case:

| `asset_type` | `asset_code` | `issuer_address`  | `pool_id`      | routes to                                                      |
| ------------ | ------------ | ----------------- | -------------- | -------------------------------------------------------------- |
| `native`     | `null`       | `null`            | `null`         | `account_balances_current`                                     |
| `classic`    | required     | required (G-addr) | `null`         | `account_balances_current` (covers SAC classic-side trustline) |
| `pool`       | `null`       | `null`            | required (hex) | `lp_positions`                                                 |

```json
{
  "asset_type":          "native" | "classic" | "pool",
  "asset_code":          string | null,
  "issuer_address":      "G…" | null,
  "pool_id":             "64hex" | null,
  "balance":             "0.0000000",
  "last_updated_ledger": u32
}
```

`balance` is `NUMERIC(28,7)` (classic fixed-point stroops).

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
  "pool_id":         "64hex",
  "ledger_sequence": u32,
  "reserve_a":       "0.0000000",
  "reserve_b":       "0.0000000",
  "total_shares":    "0.0000000"
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
  "wasm_hash":          "64hex",
  "wasm_byte_len":      12345,
  "uploaded_at_ledger": u32,
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

`uploaded_at_ledger` duplicates `ledger_metadata.sequence` for the
emitting ledger; carried inline so the DB ingester can populate
`soroban_contracts.wasm_uploaded_at_ledger` on a later deploy without
maintaining a stateful lookup table in the Lambda (see §Rationale).

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

### Why `topic0` lifted to scalar

Redundant with `topics[0]` but directly indexable. Matches ADR 0027
`soroban_events.topic0` column; DB ingester reads one field.

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

### Why `wasm_uploads[].uploaded_at_ledger` is inline

ADR 0027 §8 `wasm_interface_metadata` has no `uploaded_at_ledger`
column, but ADR 0027 §7 `soroban_contracts.wasm_uploaded_at_ledger`
must be populated. DB ingester options:

- **Stateful tracking** in Lambda: maintain a map `wasm_hash → ledger`.
  Complex under parallel ingest.
- **Inline**: 8 bytes per entry, trivial ingester logic.

Inline wins.

### Why `event_order` is array-index-derived

ADR 0027 §13 requires `event_order SMALLINT NOT NULL`. Parser
`NftEvent` struct does not carry `event_order`. Deriving from array
index is safe: parser emits events in ledger order (deterministic);
determinism test ensures stable ordering.

### Why `function_name: string | null` despite DB `NOT NULL`

`ExtractedInvocation.function_name: Option<String>` (None for
contract-creation invocations). Artifact faithfully carries `null`;
ingester resolves (substitute sentinel or skip row).

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
- 18/18 ADR 0027 tables covered: 10 direct, 5 derived (Derivation
  map), and the enrichment columns on `tokens`, `nfts`,
  `liquidity_pool_snapshots` explicitly owned by named follow-up tasks.

### Negative

- Artifact size: StrKey + hex is bulkier than binary. Mitigation: zstd
  level 3 compresses well; measured ratio ~3-5× on mainnet ledgers.
- XDR blobs double-represent data (parsed fields + raw blob). Accepted
  for forensic replay.
- Additive-only v1 discipline requires reviewer vigilance — tempting
  "small fixes" that rename fields must be flagged as version bumps.
- Parser/DB `function_name` nullability mismatch leaked into this ADR
  as a known issue — ingester handles.
- `tokens`, `nfts`, `liquidity_pool_snapshots` will show partially
  populated rows during the transition window (v1 artifact ingester
  running, enrichment pipelines not yet deployed). Each enrichment
  task must handle the `NULL → value` `UPDATE` path cleanly.

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
