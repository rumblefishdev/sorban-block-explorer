---
title: 'Assets vs Tokens in the block explorer schema'
type: research
status: mature
spawns:
  - ../README.md
tags: [schema, naming, stellar-taxonomy, tokens, assets]
links:
  - https://developers.stellar.org/docs/tokens/anatomy-of-an-asset
  - https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md
  - https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md
history:
  - date: '2026-04-22'
    status: mature
    who: stkrolikiewicz
    note: >
      Research note originally drafted as freestanding
      `docs/assets-vs-tokens-taxonomy-note.md`. Moved into task 0154's
      `notes/` directory on creation of that task so the
      task-to-research lineage stays in one place.
  - date: '2026-04-22'
    status: mature
    who: stkrolikiewicz
    note: >
      Translated from Polish to English to match the rest of the lore
      surface (per review feedback on PR #107). SQL excerpts annotated
      as pre-ADR-0031 (asset_type was `VARCHAR` at drafting; now
      `SMALLINT` + `token_asset_type_name` helper) and pre-ADR-0030
      (soroban_contracts had a natural `VARCHAR(56)` PK at drafting;
      now a `BIGSERIAL` surrogate with `contract_id VARCHAR(56) UNIQUE`).
      The excerpts are kept verbatim to preserve the drafting-time
      state that motivated the rename; the analysis and conclusions
      remain valid under the current schema.
---

# Note: Assets vs Tokens in the block explorer schema

> Document summarising the team discussion about the `tokens` / `assets`
> naming in our table and its alignment with the official Stellar
> taxonomy. Also covers the relationship with `soroban_contracts` and
> the catalogue of possible fungible assets. Not an ADR — this is a
> research note for internal discussion before making the decision.

> **Schema state at drafting** (late 2026-04-22): the SQL excerpts below
> reflect the schema as it was on the morning of the drafting day. The
> relevant shape changes landed later the same day as part of ADRs 0030
> (`soroban_contracts` `BIGSERIAL` surrogate) and 0031 (enum columns
> `SMALLINT` + Rust enum). The excerpts are kept verbatim because they
> are the state that motivated the rename discussion; every conclusion
> in this note still holds under the current schema. Where the exact
> type matters for the rename argument, a side note is added.

## TL;DR

In the official Stellar taxonomy "Stellar Assets" and "Contract Tokens"
are **two equal categories**, not synonyms. Our `tokens` table actually
holds both (`native`, `classic`, `sac`, `soroban`) — it is _de facto_
an `assets` table named after the Soroban-first iteration of the
project.

On top of that we have a `soroban_contracts` table holding deployed
contracts, to which `tokens` links via FK. The word "token" does
**two distinct working jobs** for us: (a) it classifies _contract
role_ (`contract_type = 'token'`), (b) it is the name of the table
that holds assets. Renaming to `assets` eliminates that ambiguity —
the schema then mirrors the Stellar distinction exactly: _the contract
is a token (role), it represents an asset (value)_.

Technical Design declares "Soroban-first" but explicitly requires full
classic support, and the current schema reflects that. The table name
is an artefact of an earlier iteration, not a considered decision — no
ADR justifies it. The choice (keep vs rename) is worth making
consciously and writing down.

---

## 1. Starting point

A developer's claim: "there are no tokens on Stellar, only assets".
Question — does that justify a table rename?

**Short answer**: neither "only assets" nor "only tokens" is fully
true. Stellar has three equal categories and the words are used
precisely.

---

## 2. Official Stellar taxonomy

The [Anatomy of an Asset](https://developers.stellar.org/docs/tokens/anatomy-of-an-asset)
page defines three tokenisation models as **equal categories**:

**1. Stellar Assets (with built-in SAC)** — issued by Stellar accounts
(`G...`). Identified by the pair `(asset_code, issuer)`. State in
trustlines. Every such asset has a deterministic `C...` address for
the SAC (Stellar Asset Contract) — deploying the SAC is enough to use
the asset from Soroban.

**2. SEP-41 Contract Tokens (Soroban-native)** — deployed as WASM
contracts, identified by a `C...` address. Balances in contract data
entries. Spec: [SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md).

**3. ERC-3643 / SEP-57 (T-REX) Tokens** — a SEP-41 extension with
compliance (KYC, roles). Same `C...` identity as SEP-41.

Crucial: **Stellar itself calls category 1 "Assets" and categories 2
and 3 "Tokens"**. They are not synonyms. The word "token" in
Stellar-speak has a specific meaning: a contract-based entity
implementing the SEP-41 Token Interface.

Other official sources confirm this:

- [SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md) — the spec itself is called "Token Interface"
- [CAP-46-6 Built-in Token Contract in Soroban](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md) — the core proposal says "Token Contract"
- [Stellar Asset Contract (SAC)](https://developers.stellar.org/docs/tokens/stellar-asset-contract) — the name expands to _Stellar Asset Contract_: it takes a classic asset and exposes it as a token inside Soroban. Direction of the bridge: asset → token
- [Create Contract Tokens on Stellar](https://developers.stellar.org/docs/tokens/token-interface) — Stellar docs consistently use "Contract Tokens" for the Soroban side

The Rust SDK client is `soroban_sdk::token::TokenClient` and
`token::StellarAssetClient`. Even for a classic asset invoked through
its SAC, the client lives in the `token::` module.

---

## 3. What Galexie delivers

Galexie ([Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie))
exports the native stellar-core format — `LedgerCloseMeta` in XDR. It
carries the **complete ledger state**, classic and Soroban together:

- all classic operations (`Payment`, `ChangeTrust`, `ManageSellOffer`,
  `PathPaymentStrictSend`, `CreateClaimableBalance`, `AllowTrust`,
  `SetOptions`)
- all `LedgerEntryChanges` (accounts, trustlines, offers, claimable
  balances, liquidity pool shares)
- Soroban operations (`InvokeHostFunction`, `ExtendFootprintTtl`,
  `RestoreFootprint`)
- Soroban meta: contract events, contract data entry changes, WASM
  deployments
- transaction results, fees, diagnostic events

Implication: **classic assets per se are present in the source data**.
Circle's USDC, every trustline, every classic payment between
G-accounts — all of it is in there.

Confirmation: [stellar-core integration docs](https://github.com/stellar/stellar-core/blob/master/docs/integration.md),
[stellar-core transactions README](https://github.com/stellar/stellar-core/blob/master/src/transactions/readme.md).

---

## 4. Which fungible assets can appear in our application

At the schema level (CHECK constraint in `0005_tokens_nfts.sql`) we
have four genuinely represented classes plus two edge cases.

### 4.1 Four classes from `asset_type`

**`native`** — XLM, the only Stellar native token. No issuer, no
`contract_id`. `uidx_tokens_native` enforces exactly one such row.

**`classic`** — credit assets issued by `G...` accounts, identified
by the pair `(asset_code, issuer)`. Two sub-categories at the
protocol level: alphanumeric-4 (up to 4 characters: USDC, EURC, yXLM,
AQUA) and alphanumeric-12 (5–12 characters). Held in trustlines.
**SAC not yet deployed.**

**`sac`** — a classic credit asset with a deployed SAC. It has
**both identities**: `(code, issuer)` and `contract_id`. The SAC
address is deterministic from `(code, issuer)` and can be computed
offline. Balances in trustlines (for G) and in contract data (for C).
In practice every popular classic asset has its SAC deployed because
Blend / Soroswap depend on it.

**`soroban`** — purely contract-based SEP-41 tokens that never
existed on classic. Only `contract_id`, no code / issuer. Balances
only in contract data entries. Examples: Blend tokens, governance
tokens, Soroswap LP share tokens (Soroswap issues its own SEP-41s
representing LP shares of its AMM — it does not use native Stellar
LPs).

### 4.2 Edge cases outside the `tokens` table

**Classic liquidity pool shares** — Stellar has native LPs at the
protocol level (`AssetType.ASSET_TYPE_POOL_SHARE`). Technically
fungible assets in stellar-xdr, but for us they are not in `tokens`
— separate `liquidity_pools` + `lp_positions` tables (task 0126).

**T-REX / SEP-57 tokens** — a SEP-41 extension with compliance. They
would currently fall into `'soroban'` (no dedicated value). For now
this is a non-issue because the T-REX ecosystem on Stellar is nascent.

### 4.3 Classification pitfalls already affecting you

- **Task 0118 (NFT false positives)** — some contracts emit SEP-41-
  compliant `transfer` events but represent NFTs (SEP-56 or their own
  standards). Deciding fungible vs non-fungible from events alone
  fails — the contract's interface has to be inspected.
- **Task 0120 (Soroban-native non-SAC detection)** — `sac` vs
  `soroban` cannot be told apart from events (both emit SEP-41
  `transfer`); only from the `is_sac` flag on `soroban_contracts`,
  which comes from deployment events (SACs use a deterministic
  `HostFunction::CreateContract` with `ContractIdPreimageFromAsset`).
  If the parser misses that, a SAC ends up as `soroban` — false
  positive.
- **Non-standard token contracts** — they implement most of SEP-41
  but, e.g., do not expose `decimals()` or use a different topic
  schema for `transfer`. Effectively require a whitelist or fuzzy
  pattern matching.

### 4.4 Tabular summary

| `asset_type` | Identity                  | Held in                                    | `soroban_contracts` row? | Example       |
| ------------ | ------------------------- | ------------------------------------------ | ------------------------ | ------------- |
| `native`     | none                      | trustlines (G) / contract data (C via SAC) | no (`contract_id` NULL)  | XLM           |
| `classic`    | `(code, issuer)`          | trustlines                                 | no                       | yUSDC w/o SAC |
| `sac`        | `(code, issuer)` + `C...` | trustlines + contract data                 | **yes (FK enforced)**    | USDC with SAC |
| `soroban`    | `C...`                    | contract data                              | **yes (FK enforced)**    | Blend BLND    |

---

## 5. What we actually do — current write-path state

File: `crates/indexer/src/handler/persist/mod.rs`. The `persist_ledger`
method runs a 14-step pipeline in a single atomic DB transaction
(ADR 0027).

It handles **both classic and Soroban** — "Soroban-first" only in the
sense of UX priorities, not data scope. Consistent with Technical
Design §1.1:

> **Classic + Soroban** — Support both classic Stellar operations
> (payments, offers, path payments, etc.) and Soroban operations
> (invoke host function, contract events, token swaps).

### 5.1 The `tokens` table in the real schema

Migration `crates/db/migrations/0005_tokens_nfts.sql`, **as of
drafting (pre-ADR-0031)**:

```sql
asset_type VARCHAR(20) NOT NULL
CHECK (asset_type IN ('native', 'classic', 'sac', 'soroban'))

CONSTRAINT ck_tokens_identity CHECK (
    (asset_type = 'native'  AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
 OR (asset_type = 'classic' AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 'sac'     AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
 OR (asset_type = 'soroban' AND issuer_id IS NULL      AND contract_id IS NOT NULL)
)

CREATE UNIQUE INDEX uidx_tokens_native ON tokens ((asset_type))
    WHERE asset_type = 'native';
CREATE UNIQUE INDEX uidx_tokens_classic_asset ON tokens (asset_code, issuer_id)
    WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX uidx_tokens_soroban ON tokens (contract_id)
    WHERE asset_type IN ('soroban', 'sac');
```

After ADR 0031 (same day), `asset_type` is `SMALLINT` backed by the
Rust `TokenAssetType` enum; the `CHECK (asset_type IN (…))` predicate
becomes a numeric range check, and label rendering goes through the
`token_asset_type_name` SQL helper. The four-variant domain is
unchanged; the partial unique indexes and `ck_tokens_identity` carry
over verbatim. After ADR 0030, `tokens.contract_id` is a `BIGINT` FK
to `soroban_contracts.id` (not the old `VARCHAR(56)` FK to
`soroban_contracts.contract_id`). The argument for the rename is
type-shape agnostic, so the conclusions below apply under either
encoding.

SACs sit in both partial unique indexes because they carry both
identities. The `upsert_tokens` function at
`persist/write.rs:743-910` splits staged rows into the four classes
and uses a dedicated path for each.

### 5.2 Drift between design doc and migrations

`docs/architecture/technical-design-general-overview.md` §6.7
describes an older shape:

```sql
asset_type VARCHAR(10) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban'))
UNIQUE (asset_code, issuer_address)
UNIQUE (contract_id)
```

Differences vs reality (post-ADR 0030/0031):

- doc: 3 `asset_type` values; reality: **4** (`native` added for XLM).
- doc: `VARCHAR(10)`; reality: `SMALLINT` with `token_asset_type_name`
  helper (ADR 0031). The drafting-time schema had `VARCHAR(20)`.
- doc: plain `UNIQUE`; reality: **partial unique indexes** per
  `asset_type`.
- doc: no `ck_tokens_identity`; reality: present.
- doc: `contract_id VARCHAR(56)` FK to
  `soroban_contracts.contract_id`; reality: `contract_id BIGINT` FK
  to `soroban_contracts.id` (ADR 0030).

Analogous drift affects other tables in the design §6
(`transaction_hash_index`, `transaction_participants`,
`wasm_interface_metadata`, `lp_positions`, `nft_ownership`,
`account_balances_current` / `account_balances_history`). This
document **does not propose** updating the design — it only records
that the drift exists.

---

## 6. Relationship with the `soroban_contracts` table

This is the crucial angle for the naming question, because the
semantic collision of the word "token" lives in this relationship.

### 6.1 Schema

From migration `0002_identity_and_ledgers.sql`, **as of drafting
(pre-ADR-0030/0031)**:

```sql
CREATE TABLE soroban_contracts (
    contract_id   VARCHAR(56) PRIMARY KEY,
    wasm_hash     BYTEA REFERENCES wasm_interface_metadata(wasm_hash),
    deployer_id   BIGINT REFERENCES accounts(id),
    deployed_at_ledger BIGINT,
    contract_type VARCHAR(50),      -- 'token', 'nft', 'dex', 'lending', 'other'
    is_sac        BOOLEAN NOT NULL DEFAULT false,
    metadata      JSONB,
    ...
);
```

And the FK from `tokens`:
`contract_id VARCHAR(56) REFERENCES soroban_contracts(contract_id)`.

After ADR 0030 (contracts surrogate), `soroban_contracts` has a
`BIGSERIAL id` primary key and `contract_id VARCHAR(56)` becomes a
`UNIQUE` natural key. After ADR 0031, `contract_type` is `SMALLINT`
with the `contract_type_name` SQL helper. FKs from `tokens` / `nfts`
point at `soroban_contracts.id`, not `contract_id`. The role-vs-
value-separation argument below is independent of the encoding; the
semantic collision analysed in §6.2 holds under both.

### 6.2 What it actually maps

This is a **clean, 1-to-1 projection of the Stellar distinction**:

- `soroban_contracts` = deployed contracts (everything with `C...`)
- `soroban_contracts.contract_type = 'token'` = "this contract
  implements the SEP-41 Token Interface"
- `soroban_contracts.is_sac = true` = "this contract is a SAC for
  some classic asset"
- `tokens.contract_id → soroban_contracts` = "here is the asset that
  this token-contract represents"

Exactly the split: **token = contract interface, asset = unit of
value**. The schema _already distinguishes them structurally_.

### 6.3 Integrity enforcement

`ck_tokens_identity` explicitly states when `contract_id` must be
NOT NULL:

- `native`, `classic` → `contract_id IS NULL` (no contract)
- `sac`, `soroban` → `contract_id IS NOT NULL` + FK to
  `soroban_contracts`

This means **every `sac`/`soroban` row in `tokens` requires a
corresponding row in `soroban_contracts`, structurally**. The
database enforces this.

NFTs analogously:
`nfts.contract_id VARCHAR(56) NOT NULL REFERENCES soroban_contracts(contract_id)`.

### 6.4 What `soroban_contracts` additionally holds that `tokens` does not

For every `sac` / `soroban` row in `tokens`, the corresponding row in
`soroban_contracts` holds:

- `wasm_hash` — FK to `wasm_interface_metadata`, i.e. the
  implementation (every SAC has the same stub WASM; Soroban-native
  contracts each have their own)
- `deployer_id`, `deployed_at_ledger`, `wasm_uploaded_at_ledger`
- `is_sac` — the canonical SAC-vs-non-SAC flag
- `contract_type` — classification of the contract role
- `metadata` JSONB — interface signatures (ADR 0023)
- `search_vector` — GIN index for search

### 6.5 Example: USDC with SAC on mainnet

1. Row in `accounts` — Circle's issuer (`GA5ZSEJY...KZVN`).
2. Row in `wasm_interface_metadata` — the SAC stub WASM (shared by
   every SAC).
3. Row in `soroban_contracts`: `contract_id = CCW6...MI75`,
   `is_sac = true`, `contract_type = 'token'`, FK →
   `wasm_interface_metadata`.
4. Row in `tokens`: `asset_type = 'sac'`, `asset_code = 'USDC'`,
   `issuer_id` → Circle's accounts row, `contract_id = CCW6...MI75`
   → FK to `soroban_contracts`.

For Blend BLND (Soroban-native): row in `soroban_contracts` with
`is_sac = false`, own `wasm_hash`, `contract_type = 'token'`; row in
`tokens` with `asset_type = 'soroban'`, only `contract_id`, no
code / issuer.

### 6.6 Edge case: XLM

XLM has a SAC deployed on mainnet, actively used by DeFi
(`CAS3J...YHXP`). In our schema `ck_tokens_identity` forces
`contract_id IS NULL` for `native` — so the XLM row in `tokens`
**does not link** to the XLM SAC. If the XLM SAC is detected by the
parser during contract detection, it can land in `soroban_contracts`
as a separate row, but `tokens.native` does not know about it.

Data-model gap: `asset_type = 'sac'` requires `issuer_id IS NOT NULL`,
and XLM has no issuer. So the XLM SAC cannot be represented as `sac`.
Consequence: "show all contract events for XLM" — no JOIN path.

Worth verifying how the parser handles this path (or consciously
documenting it as a known limitation).

---

## 7. The core: semantic collision of the word "token"

The facts above add up to one concrete observation:

The word "token" does **two distinct working jobs in two tables**:

1. In `soroban_contracts.contract_type = 'token'` → classifies the
   _contract role_ as SEP-41 Token Interface. "Token" = interface
   type.
2. In the table name `tokens` → holds _units of value_ (fungible),
   including classic assets that have no contract and no SEP-41
   surface.

That is an ambiguity baked into the naming. A sample team-chat
problem: "this token is in the `tokens` table" — which token are we
talking about? A contract with `contract_type='token'`, or a row in
`tokens`? Not a trivial question, because classic assets in `tokens`
have no counterpart in `soroban_contracts`.

### 7.1 Arguments for keeping `tokens` (status quo)

- Nothing to migrate.
- Convenient team vocabulary — everyone knows what we mean.
- "Token" is natural in Soroban-world (SEP-41, SDK).
- Internal convention over Stellar jargon.

### 7.2 Arguments for `assets`

- Consistency with Stellar's official taxonomy (the "Anatomy of an
  Asset" page is the umbrella; "Stellar Assets" is one category
  inside it).
- The table already actually holds classic + native (= "Stellar
  Assets" in Stellar jargon), so `tokens` is misleading.
- A new developer reading the schema expects `tokens` =
  contract-based — but classic XLM lives there too.
- **Eliminates the collision with
  `soroban_contracts.contract_type = 'token'`** — "the contract is
  a token (role), it represents an asset (value)" becomes
  unambiguous.
- A UI search ("find USDC") expects one result, and our schema
  already does it that way — the name "assets" better reflects
  reality.

### 7.3 Illustrative rename (if the decision lands on this side)

Table: `tokens` → `assets`.

`asset_type` values:

- `native` → stays.
- `classic` → `classic_credit` (more precise, since XLM is also
  classic).
- `sac` → stays (unambiguous).
- `soroban` → `soroban_sep41` (more precise, leaves room for
  `soroban_trex`).

Optional: `soroban_contracts.contract_type = 'token'` →
`'sep41_token'`, to make explicit that this is the interface role, not
the `assets` table. Current `'token'` is fine after the table rename
because the collision disappears.

Structure (partial uniques, `ck_tokens_identity`, FK) stays the same.

Code changes: rename the table, rename the column in UI / API if we
want end-to-end consistency, update queries in `write.rs` and axum
endpoints. PostgreSQL migration: `ALTER TABLE tokens RENAME TO
assets` + optional enum-value remap.

NFTs analogously: `soroban_contracts.contract_type = 'nft'` + the
`nfts` table (instances). Renaming `nfts` **is not needed** — no
ambiguity there, because the table holds instances (`unique
(contract_id, token_id)`), not contracts.

---

## 8. What this note is NOT about

- Whether to fix the drift in the Technical Design doc — a separate
  matter, not coupled here.
- Reorganising the `native_token` / `classic_token` / etc. split in
  the write-path code — independent from the table name.
- API change (`/tokens/:id` vs `/assets/:id`) — can be considered
  together with the table rename or separately; the API may carry a
  different nomenclature than the DB.
- Resolving the XLM ↔ XLM SAC link edge case — noted in §6.6, but
  independent from naming.

---

## 9. Inventory of places to change (rename scope)

A sweep of the codebase + the design doc shows where the word "token"
is used as an umbrella covering classic / native (i.e. off Stellar-
speak) and where it is used legitimately in the "SEP-41 contract"
sense. Below is the full scope of a potential rename, with the
distinction marked.

### 9.1 DB schema — the centre of the change

Migration `crates/db/migrations/0005_tokens_nfts.sql`:

- table `tokens` → `assets`.
- constraints `ck_tokens_asset_type`, `ck_tokens_identity` →
  `ck_assets_*`.
- indexes `uidx_tokens_native`, `uidx_tokens_classic_asset`,
  `uidx_tokens_soroban`, `idx_tokens_type`, `idx_tokens_code_trgm` →
  `uidx_assets_*`, `idx_assets_*`.
- migration filename `0005_tokens_nfts.sql` stays (historical); a new
  migration ships `ALTER TABLE tokens RENAME TO assets` + renaming of
  constraints / indexes.

FKs in other tables (`operations`, `soroban_events`,
`soroban_invocations`, `nfts`) point to `tokens.id` — the rename
follows them automatically without SQL changes.

### 9.2 Rust code

**`crates/domain/src/token.rs`**:

- file renamed to `asset.rs`.
- `pub struct Token` → `Asset`.
- docstring line 1 ("Token domain type matching the `tokens`
  PostgreSQL table") updated.

**`crates/xdr-parser/src/types.rs`**:

- `pub struct ExtractedToken` → `ExtractedAsset`.

**`crates/xdr-parser/src/state.rs`**:

- `pub fn detect_tokens(deployments)` → `detect_assets`.
- `ExtractedToken` imports updated.

**`crates/xdr-parser/src/classification.rs`** — token classification
logic, likely `TokenClassification`, `classify_token`. Needs a closer
audit during implementation.

**`crates/indexer/src/handler/persist/staging.rs`**:

- `pub(super) struct TokenRow` → `AssetRow`.
- `pub token_rows: Vec<TokenRow>` → `asset_rows`.
- parameter `tokens: &[ExtractedToken]` → `assets: &[ExtractedAsset]`.

**`crates/indexer/src/handler/persist/write.rs:743-910`**:

- `upsert_tokens()`, `upsert_tokens_native`,
  `upsert_tokens_classic_like`, `upsert_tokens_soroban` →
  `upsert_assets*`.

**`crates/indexer/src/handler/persist/mod.rs`**:

- `tokens` parameter in the `persist_ledger` signature.
- "12. tokens" comment in the pipeline.
- `tokens_ms` field in `StepTimings` (lines 60, 202) — note: visible
  in logs and metrics, the rename breaks Grafana / CloudWatch
  dashboards.

**`crates/indexer/src/handler/process.rs:127`**:

- `let tokens = xdr_parser::detect_tokens(&deployments);`.

**Tests `crates/indexer/tests/persist_integration.rs`**:

- helper `make_sac_token()` → `make_sac_asset()`.
- `ExtractedToken` imports.

### 9.3 Technical Design Overview

This is where the confusion is densest. Three lines are outright
**smoking guns** — the doc itself admits the name "tokens" covers
something wider:

> **Line 158**: _"Balances — native XLM balance and trustline/token balances"_

In Stellar-speak a trustline balance is an asset balance, not a
token balance. Classic misuse.

> **Line 163**: _"List of all known tokens (classic Stellar assets and Soroban token contracts)"_

The doc itself explicitly expands the name in parentheses — a strong
signal that the name is too narrow.

> **Line 370**: _"Paginated list of tokens (classic assets + Soroban token contracts)"_

Same thing, in the API section.

Other places to update in the doc (approximate line numbers):

- 46, 58-59, 85-86 — route tables with `/tokens`, `/tokens/:id`.
- 161, 165, 170, 172, 174, 177 — Tokens page / Token detail
  description.
- 280 — ASCII diagram "Tokens" module in backend Lambda.
- 368-376 — "Tokens" endpoints section.
- 414 — search params `type=...,token,...`.
- 470 — ASCII diagram RDS listing the `tokens` table (rename to stay
  consistent with DB).
- 739 — "Derived-state upserts (`accounts`, `tokens`, `nfts`,
  `liquidity_pools`)".
- 948, 951 — §6.7 header and `CREATE TABLE tokens`.
- 1072, 1086, 1110-1111, 1208 — estimate tables and deliverables.

### 9.4 API endpoints (public contract)

From the design §2.3:

- `GET /tokens` → `/assets`.
- `GET /tokens/:id` → `/assets/:id`.
- `GET /tokens/:id/transactions` → `/assets/:id/transactions`.
- query param `type=...,token,...` on `/search` — decide whether
  "token" stays as a filter type or becomes "asset".

**This is a public contract.** If the API is pre-launch, the rename
is painless. If it is already out, versioning (`/v1/tokens` old,
`/v2/assets` new) or aliases are required.

### 9.5 ADRs — left alone

Files that historically use "token" in titles and content:

- `0022_schema-correction-and-token-metadata-enrichment.md`
- `0023_tokens-typed-metadata-columns.md`
- `0027_post-surrogate-schema-and-endpoint-realizability.md`
  (`tokens` in the body)

**Do not rename.** ADRs are historical records. A new ADR for the
rename decision updates future readers' context without rewriting
history.

### 9.6 Places where "token" is used **correctly** — leave alone

- `soroban_contracts.contract_type = 'token'` — contract role
  (SEP-41).
- `nfts.token_id` — standard NFT terminology
  (`(contract_id, token_id)` = instance identifier).
- "Token Interface" / "SEP-41 Token" — official protocol term.
- `soroban_sdk::token::TokenClient`, `token::StellarAssetClient` —
  Rust SDK naming, not ours.
- "Detect token contracts (SEP-41)" in the design line 669 —
  correct, "token contract" = SEP-41 contract.
- "Token swap" in Soroban DEX descriptions — correct in that
  context.

### 9.7 Scope summary

Files touched: ~15–20. Most changes are mechanical (rename of
structs / functions / imports / columns). Risky spots: the DB
migration (ALTER TABLE + renaming of constraints and indexes in a
single transaction) and the API (versioning if already public). The
rest is rename-all-at-once guarded by `cargo check`.

Effort estimate: 1–2 days for one developer, including the DB
migration with rollback, design update, code rename, and a green
test suite. Most of it is mechanics, few hard calls.

---

## 10. Proposed decision format

Option A: keep `tokens`, but add a short ADR establishing that it is
an umbrella and that we consciously diverge from Stellar's taxonomy
(rationale: we do not want a migration, internal consistency). No
code changes.

Option B: rename to `assets`. The ADR captures the decision + DB
migration + query updates. Larger one-time effort, cleaner
nomenclature afterwards, removed collision with
`soroban_contracts.contract_type = 'token'`, no recurring
discussions.

Option C: status quo with no ADR. We live with the drift between our
name and Stellar jargon. Risk: recurring questions on every new
developer / on the public API documentation.

Personal preference: **A or B, depending on how much a migration
costs right now**. C (silent status quo) is the worst, because the
question keeps coming back.

---

## Sources

### Official Stellar

- [Anatomy of an Asset — Stellar Docs](https://developers.stellar.org/docs/tokens/anatomy-of-an-asset) — key page, taxonomy of the three models
- [Create Contract Tokens on Stellar — Stellar Docs](https://developers.stellar.org/docs/tokens/token-interface)
- [Stellar Asset Contract (SAC) — Stellar Docs](https://developers.stellar.org/docs/tokens/stellar-asset-contract)
- [SEP-41: Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [CAP-46-6: Built-in Token Contract in Soroban](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md)
- [soroban_sdk::token::TokenInterface — Rust SDK docs](https://docs.rs/soroban-sdk/latest/soroban_sdk/token/trait.TokenInterface.html)

### Galexie / data pipeline

- [Galexie — Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie)
- [Introducing Galexie (Stellar blog)](https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data)
- [stellar-core integration.md — emits LedgerCloseMeta over pipe](https://github.com/stellar/stellar-core/blob/master/docs/integration.md)
- [stellar-core transactions README](https://github.com/stellar/stellar-core/blob/master/src/transactions/readme.md)

### Community / third-party (confirming both terms are in use)

- [Navigating Classic Assets and Smart Contract Tokens on Soroban — Cheesecake Labs](https://cheesecakelabs.com/blog/native-tokens-vs-soroban-tokens/)
- [stellar-cli issue #934: refers to 'stellar asset contract' as 'token'](https://github.com/stellar/stellar-cli/issues/934) — an example of the Stellar team itself seeing the term-mixing as real nuance

### Our files

- `docs/architecture/technical-design-general-overview.md` — §1.1 (Classic + Soroban goal), §4.1 (pipeline), §6.4 (`soroban_contracts`), §6.7 (`tokens` schema — v1)
- `crates/indexer/src/handler/persist/mod.rs` — `persist_ledger` (14 steps, ADR 0027)
- `crates/indexer/src/handler/persist/write.rs:743-910` — `upsert_tokens` + per-kind variants
- `crates/db/migrations/0002_identity_and_ledgers.sql:40-57` — `soroban_contracts` schema
- `crates/db/migrations/0005_tokens_nfts.sql:16-47` — `tokens` schema (4 kinds, partial uniques, `ck_tokens_identity`, FK to `soroban_contracts`)
- `lore/2-adrs/0022_schema-correction-and-token-metadata-enrichment.md`
- `lore/2-adrs/0023_tokens-typed-metadata-columns.md`
- `lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md`
