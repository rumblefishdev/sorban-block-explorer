---
id: '0160'
title: 'BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction'
type: BUG
status: active
related_adr: ['0023', '0027', '0036']
related_tasks: ['0120', '0124', '0154', '0161']
tags:
  [
    priority-high,
    effort-medium,
    layer-indexer,
    layer-xdr-parser,
    layer-db,
    audit-gap,
  ]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/operation.rs
  - crates/xdr-parser/src/types.rs
  - crates/indexer/src/handler/process.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/src/handler/persist/staging.rs
  - crates/db/migrations/0005_tokens_nfts.sql
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from post-0154 audit. Empirical: after reindexing 1000
      ledgers on develop, `assets` table is empty. Root cause is
      pre-existing (inherited through 0120 + 0154 rename, both
      mechanical). Not a regression.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Highest-impact of the three audit-gap tasks (0160/0161/0162)
      — SAC/classic is the dominant asset population on mainnet, so this
      directly unblocks `assets` table completeness.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: >
      Exploration pivot. Original plan assumed `executable.stellar_asset`
      subtree carries asset_code/issuer — it does not. XDR
      `ContractExecutable::StellarAsset` is a marker variant with no
      embedded asset data. Real source: deployment tx's CreateContract
      operation args, `ContractIdPreimage::FromAsset(Asset)`. Plan
      rewritten below. Effort bumped small → medium.
---

# BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction

## Summary

`xdr_parser::detect_assets` emits an `ExtractedAsset` row for every SAC
deployment with `asset_type = Sac` but `asset_code = None` and
`issuer_address = None`. The staging path `upsert_assets_classic_like`
filters those rows out because both fields are required for the
classic-like INSERT. Net effect: **SAC deployments silently dropped, no
row ever lands in `assets`.** Gap has existed since initial
implementation; 0120 (detection wiring) and 0154 (tokens→assets rename)
both inherited it unchanged.

## Context

### Reproduction

- `crates/xdr-parser/src/state.rs:513-522` — SAC branch pushes
  `ExtractedAsset` with `asset_code: None`, `issuer_address: None`.
- `crates/indexer/src/handler/persist/write.rs:1095-1101` —
  `upsert_assets_classic_like` does `let Some(code) = r.asset_code
else { continue; }` and the analogous issuer guard. SAC rows fall
  through both.
- `ck_assets_identity` for `asset_type = 2` (Sac) requires
  `asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id
IS NOT NULL`. The staging skip is correct defensive behaviour —
  without it the INSERT would violate the CHECK. The real fix is
  upstream: populate the fields in the parser.

### Where the underlying asset actually lives (exploration finding)

Initial plan assumed `executable.stellar_asset` subtree carries the
classic asset identity. It does not. `ContractExecutable::StellarAsset`
is a **marker-only XDR variant** — the ContractInstance entry renders
as `{"type": "stellar_asset"}` with no sibling code/issuer keys
(confirmed in `scval.rs:75-84`).

The underlying classic `Asset` (code + issuer, or native) is carried
in the **deployment transaction's CreateContract operation**,
specifically `CreateContractArgs.contract_id_preimage` with variant
`FromAsset(Asset)`. Today the parser sees it but drops it:

- `operation.rs:328-332` — CreateContract arm extracts only
  `executable`, not `contract_id_preimage`.
- `operation.rs:340-346` — CreateContractV2 does the same.
- `invocation.rs:248-257` — auth entry `CreateContractHostFn` also
  drops `contract_id_preimage`.

`ExtractedOperation.details` is in-memory only (no `details JSONB`
column in `operations` schema — `0003_transactions_and_operations.sql`),
but that is fine: we only need correlation during `process_ledger`,
not persistence.

### Timing — correlation feasibility

`process.rs:130` loop over `all_ledger_entry_changes` runs **after**
`all_operations` is populated (lines 43-78). The `tx_hash` is already
in the tuple (`_tx_hash` ignored today). Correlation by tx_hash is
cheap and co-located.

Audit note: 2026-04-10 pipeline audit §5.1 flagged
`tokens.asset_code` / `tokens.issuer_address` as nullable only for
Soroban-native — implicit expectation that SAC has them populated,
but no task captured the extraction.

## Implementation

### 1. Extract `contract_id_preimage` in operation parser

`crates/xdr-parser/src/operation.rs` — extend both CreateContract
arms to include preimage in the details JSON. New helper:

```rust
fn format_contract_id_preimage(p: &ContractIdPreimage) -> Value {
    match p {
        ContractIdPreimage::Address(addr) => json!({
            "type": "from_address",
            "address": format_sc_address(&addr.address),
            "salt": hex::encode(&addr.salt),
        }),
        ContractIdPreimage::Asset(asset) => json!({
            "type": "from_asset",
            "asset": format_asset(asset),
        }),
    }
}
```

`format_asset(&Asset)` helper returning shape matching the trustline
convention (`state.rs:225-246`):

- `Asset::Native` → `{"type": "native"}`
- `Asset::CreditAlphanum4 { asset_code, issuer }` →
  `{"type": "credit_alphanum4", "asset_code": "USDC", "issuer": "G..."}`
  (strip trailing NULs from 4-byte code, encode AccountId → StrKey)
- `Asset::CreditAlphanum12 { asset_code, issuer }` → analogous

Update CreateContract/V2 extraction:

```rust
HostFunction::CreateContract(args) => json!({
    "hostFunctionType": "createContract",
    "executable": format_contract_executable(&args.executable),
    "contract_id_preimage": format_contract_id_preimage(&args.contract_id_preimage),
}),
```

Same pattern for `CreateContractV2` and the auth-entry path in
`invocation.rs:248`.

### 2. Build tx_hash → SAC asset map in process.rs

`crates/indexer/src/handler/process.rs` — before the
`all_ledger_entry_changes` loop at line 129, build:

```rust
// For each tx: if it has a CreateContract op whose
// contract_id_preimage is FromAsset, capture (code, issuer).
// Assumes at most one SAC deploy per tx (MVP — see Notes).
let sac_assets_by_tx: HashMap<String, (Option<String>, Option<String>)> =
    build_sac_asset_map(&all_operations);
```

Helper signature (implement in `xdr-parser` or indexer — prefer
`xdr-parser` so parser is self-contained):

```rust
pub fn extract_sac_asset_from_create_contract(
    op: &ExtractedOperation,
) -> Option<(Option<String>, Option<String>)>;
// Reads op.details["contract_id_preimage"]["type"] == "from_asset",
// then details["contract_id_preimage"]["asset"]{"type","asset_code","issuer"}.
// Returns Some((None, None)) for native (XLM-SAC sentinel applied downstream).
// Returns Some((Some("USDC"), Some("G..."))) for credit.
// Returns None if not a from_asset CreateContract.
```

### 3. Thread map into `extract_contract_deployments` / `detect_assets`

Two signature options; prefer B for minimal surface change:

**A.** `extract_contract_deployments(changes, tx_source, sac_asset_for_tx: Option<(String, String)>)`
— deployment carries SAC identity natively.

**B.** `detect_assets(deployments, interfaces, sac_assets_by_contract: &HashMap<String, (String, String)>)`
— parser stays per-fn clean; `detect_assets` does the merge.

Pick **A** because SAC identity is part of the deployment's own data,
not a detection detail. Extend `ExtractedContractDeployment`:

```rust
// crates/xdr-parser/src/types.rs — add to ExtractedContractDeployment:
pub sac_asset_code: Option<String>,    // None for native XLM-SAC
pub sac_asset_issuer: Option<String>,  // None for native XLM-SAC
```

`extract_contract_deployments` gains `sac_asset_for_tx: Option<&(Option<String>, Option<String>)>`
and populates the new fields when `is_sac = true`. `detect_assets` SAC
branch reads directly from `deployment.sac_asset_*`.

### 4. XLM-SAC sentinel (option c)

**Decision (emerged):** for `Asset::Native` preimage, synthesise:

- `asset_code = "XLM"`
- `asset_issuer = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"`
  (all-zero Ed25519, valid StrKey; encoded once as a constant in
  `xdr-parser`).

Rationale:

- Preserves `asset_type = 2` (Sac) semantics — the row remains a SAC,
  not a misclassified Soroban-native (option b).
- Keeps `ck_assets_identity` for `asset_type = 2` satisfied without
  schema change (option a).
- Zero-address sentinel is self-evidently not a real Stellar account.
  Any UI rendering "native XLM-SAC" can special-case this issuer.

**FK prerequisite — sentinel account row.** `assets.issuer_id`
references `accounts(id)`. The sentinel G-address must exist in
`accounts` before the first XLM-SAC INSERT, or the FK fails.

Runtime `INSERT ... WHERE NOT EXISTS` races under parallel backfill
(two workers both see NOT EXISTS, both INSERT, UNIQUE violation).
**Seed via migration** alongside 0161's native singleton:

```sql
-- In 0005_tokens_nfts.sql (or a dedicated combined seed edit with 0161):
INSERT INTO accounts (str_key, first_seen_ledger, last_seen_ledger, ...)
VALUES ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF', 0, 0, ...);
```

Coordinate exact migration file + row columns with 0161.

### 5. Issuer → account_keys staging

`staging.rs` — when a SAC asset row is materialised with a real
(non-sentinel) issuer G-address, the issuer must be added to
`staged.account_keys` so `upsert_accounts` creates / upserts it before
`upsert_assets_classic_like` resolves `issuer_id`. Without this,
`resolve_id(account_ids, issuer_key, "asset.issuer")` hard-fails on
first-seen issuers.

Mirror the existing pattern used for trustline issuers in
`staging.rs:882-886`.

### 6. `upsert_assets_classic_like` — monotonic `asset_type` promotion

Latent bug surfaced by enabling SAC writes: if a future classic path
(not in this task) inserts `asset_type = 1` for a (code, issuer) pair
that already has a SAC row (`asset_type = 2, contract_id NOT NULL`),
the current DO UPDATE leaves `asset_type` unchanged. But in parallel
backfill, order-inverted commits can downgrade:

- Worker B commits SAC deploy first (type=2, contract_id=7).
- Worker A commits classic trustline second (type=1, contract_id=NULL).
- `DO UPDATE SET contract_id = COALESCE(NULL, 7) = 7` (OK), but if
  `asset_type` were set to `EXCLUDED.asset_type = 1`, the row becomes
  `(type=1, contract_id=7)` — `ck_assets_identity` violation.

Fix — monotonic promotion, order-independent:

```sql
ON CONFLICT (asset_code, issuer_id)
  WHERE asset_type IN (1, 2)
  DO UPDATE SET
    contract_id  = COALESCE(EXCLUDED.contract_id, assets.contract_id),
    asset_type   = GREATEST(EXCLUDED.asset_type, assets.asset_type),
    name         = COALESCE(EXCLUDED.name, assets.name),
    total_supply = COALESCE(EXCLUDED.total_supply, assets.total_supply),
    holder_count = COALESCE(EXCLUDED.holder_count, assets.holder_count)
```

`GREATEST(1, 2) = 2` — SAC info sticks. `GREATEST(2, 2) = 2` — no-op on
replay. Parallel-safe in both commit orders. Defensive today (classic
path doesn't exist yet), mandatory the moment it does.

### 7. Tests

- **Unit** (`xdr-parser` operation.rs): `CreateContract` with
  `ContractIdPreimage::Asset(CreditAlphanum4 { "USDC", G... })` →
  `ExtractedOperation.details.contract_id_preimage.asset.asset_code
== "USDC"` etc.
- **Unit**: same for `CreditAlphanum12`.
- **Unit**: `Asset::Native` → `details.contract_id_preimage.asset.type
== "native"`.
- **Unit**: `ContractIdPreimage::Address` (non-SAC) → no asset data,
  helper returns `None`.
- **Unit** (`xdr-parser` state.rs): `detect_assets` with a SAC
  deployment carrying `sac_asset_code = Some("USDC")`,
  `sac_asset_issuer = Some("G...")` → `ExtractedAsset` with those
  fields set.
- **Unit**: SAC deployment with `sac_asset_code = None`,
  `sac_asset_issuer = None` (native) → `ExtractedAsset` with
  sentinel applied (`"XLM"`, zero-address).
- **Integration** (`persist_integration.rs`): synthetic ledger with
  CreateContract op (credit_alphanum4) + matching ContractInstance
  create → `assets` row has non-NULL `asset_code`, `issuer_id`,
  `contract_id`. Regression: the existing SAC-only integration test
  must be updated — it previously passed because SAC rows got
  silently dropped; now it must assert populated identity.
- **Integration**: XLM-SAC path → `assets` row with sentinel issuer,
  `asset_code = "XLM"`.
- **Integration**: parallel order-swap for classic→SAC promotion —
  insert `(USDC, G..., type=2, contract_id=7)` then
  `(USDC, G..., type=1, contract_id=NULL)` → final row is
  `(type=2, contract_id=7)`, no CHECK violation.

## Acceptance Criteria

- [ ] `operation.rs` extracts `contract_id_preimage` for both
      `CreateContract` and `CreateContractV2`; helper
      `format_contract_id_preimage` handles `FromAddress` + `FromAsset`.
- [ ] `format_asset` helper covers `Native`, `CreditAlphanum4`,
      `CreditAlphanum12` variants, output shape matches trustline
      convention (`type`, `asset_code`, `issuer`).
- [ ] `process.rs` builds tx_hash → SAC-asset map before deployment
      extraction loop, threads it into `extract_contract_deployments`.
- [ ] `ExtractedContractDeployment` gains `sac_asset_code`,
      `sac_asset_issuer` typed fields.
- [ ] `detect_assets` SAC branch populates `ExtractedAsset.asset_code`
      and `.issuer_address` from deployment fields.
- [ ] XLM-SAC (native preimage) emits sentinel
      (`"XLM"`, all-zero StrKey) — sentinel account seeded via
      migration, coordinated with 0161.
- [ ] Issuer G-addresses added to `staging.account_keys` so
      `upsert_accounts` resolves `issuer_id` before
      `upsert_assets_classic_like` runs.
- [ ] `upsert_assets_classic_like` DO UPDATE uses
      `asset_type = GREATEST(EXCLUDED.asset_type, assets.asset_type)`
      for monotonic parallel-safe promotion.
- [ ] Unit test coverage per §7.
- [ ] Integration test — SAC credit deploy → `assets` row with all
      three identity fields non-NULL.
- [ ] Integration test — XLM-SAC deploy → row with sentinel identity.
- [ ] Integration test — classic↔SAC order-swap does not violate
      `ck_assets_identity`.
- [ ] Existing SAC regression tests in `persist_integration.rs`
      updated to assert populated identity fields.
- [ ] `nx run rust:build` / `rust:test` / `rust:lint` green.

## Design Decisions

### From Plan

1. **Populate SAC identity in `ExtractedContractDeployment` (struct
   fields, not `metadata: Value`)** — identity data deserves typed
   fields; free-form `metadata` stays for future SEP-1 / on-chain
   extensions (0124, 0156).

### Emerged

2. **Data source pivot — operation, not ContractInstance.** Original
   plan assumed `executable.stellar_asset` subtree carries asset
   identity. XDR exploration proved it does not
   (`ContractExecutable::StellarAsset` is a marker variant). Pivoted
   to reading `CreateContractArgs.contract_id_preimage.FromAsset` from
   the deployment transaction's operation payload.

3. **XLM-SAC → option (c) synthesised sentinel.** Rejected option
   (a) (schema CHECK loosening — invasive) and (b) (route as
   `Soroban` — semantic mismatch). Chose sentinel
   (`"XLM"`, all-zero StrKey) to preserve `asset_type = 2` semantics
   without schema change.

4. **Sentinel account seeded via migration, not runtime INSERT.**
   Parallel backfill races under `INSERT ... WHERE NOT EXISTS`
   (two workers both observe NOT EXISTS, both INSERT, UNIQUE
   violation). Migration seed avoids the race entirely; coordinates
   with 0161's native asset singleton work.

5. **Tx-correlation by `tx_hash` assumes 1:1 CreateContract per tx.**
   Multiple SAC deploys in a single tx theoretically possible but
   rare in practice. Edge case flagged in Notes; follow-up task can
   introduce preimage-hash ↔ contract_id correlation if metrics show

   > 1 on mainnet.

6. **Factory-deployed SACs (via inner invocations) out of scope.**
   `invocation.rs:248` sees `CreateContractHostFn` in auth entries
   but this task only correlates top-level operation CreateContract.
   Documented as known coverage gap; separate task if needed.

7. **`asset_type` monotonic promotion via `GREATEST`.** Not part of
   the original SAC-identity scope, but surfaces immediately once SAC
   rows start actually INSERTing. Order-independent, parallel-safe,
   zero-cost defensive now + correct when classic extraction lands.

## Notes

Audit gap. Priority high because SAC/classic is the dominant asset
population on mainnet; `assets` empty without this fix.

**Parallel backfill safety:** this task does NOT introduce
counter-style race risks (no +1/-1 inline updates). All writes are
idempotent identity upserts with monotonic promotion. Safe to run
under parallel backfill without a feature flag.

**Coordination with 0161:** sentinel account seed + native asset
singleton seed are both migration edits to `0005_tokens_nfts.sql`
(or a combined DML migration). Decide at implementation time whether
to land them in the same PR or sequence; either order works as long
as native singleton and sentinel account both exist before the first
SAC INSERT.

## Future Work → Backlog

- **Factory-deployed SAC detection.** Spawn follow-up if mainnet
  metrics show non-trivial population of SACs deployed via
  `CreateContractHostFn` auth-entry path (not top-level operation).
- **Multi-SAC-per-tx correlation.** If observed on mainnet, replace
  tx_hash keyed map with preimage-hash → contract_id correlation.
