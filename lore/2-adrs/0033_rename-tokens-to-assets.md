---
id: '0033'
title: 'Rename `tokens` table → `assets`; remap `classic` → `classic_credit`'
status: accepted
deciders: [karolkow]
related_tasks: ['0154']
related_adrs: ['0022', '0023', '0027', '0030', '0031', '0032']
tags: [schema, naming, refactor, tokens, assets, api]
links:
  - lore/1-tasks/active/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md
  - lore/1-tasks/active/0154_REFACTOR_rename-tokens-to-assets/notes/S-asset-type-label-remap.md
history:
  - date: '2026-04-23'
    status: accepted
    who: karolkow
    note: >
      ADR accepted as part of task 0154 implementation.
  - date: '2026-04-23'
    status: accepted
    who: karolkow
    note: >
      Aligned ADR text with actual approach: base migration
      `0005_tokens_nfts.sql` edited in place (no ALTER TABLE, no
      reversible pair) — justified by pre-production state.
---

# ADR 0033: Rename `tokens` table → `assets`; remap `classic` → `classic_credit`

**Related:**

- [Task 0154: REFACTOR rename tokens → assets](../1-tasks/active/0154_REFACTOR_rename-tokens-to-assets/README.md)

---

## Context

The `tokens` table holds all fungible asset types (`native`, `classic`, `sac`,
`soroban`), including classic Stellar credit assets that have no SEP-41 contract
surface. The name "tokens" is a legacy artefact of the Soroban-first iteration.

Simultaneously, `soroban_contracts.contract_type = 'token'` classifies the SEP-41
contract _role_. The word "token" does two distinct jobs in the same schema,
causing recurring team confusion: "this token is in the tokens table" is
ambiguous — it could mean a contract with `contract_type='token'` or a row in
`tokens`.

The official Stellar taxonomy treats "Stellar Assets" and "Contract Tokens" as
two equal, distinct categories (see research note for sources). Our table is
_de facto_ an assets table; renaming aligns schema to taxonomy and eliminates
the ambiguity. Full analysis: `notes/R-assets-vs-tokens-taxonomy.md`.

The `classic` label additionally sits ambiguously next to `native` — XLM is
also "classic" in the broad sense. The XDR term for credit assets is
`CREDIT_ALPHANUM4` / `CREDIT_ALPHANUM12`; `classic_credit` mirrors that
precisely. Remap rationale: `notes/S-asset-type-label-remap.md`.

---

## Decision

1. **Table rename** — base migration `0005_tokens_nfts.sql` edited in place
   so it creates `assets` directly. No `ALTER TABLE` / reversible pair —
   justified because no production deployment exists yet; staging is
   reset on redeploy. FK columns in `operations`, `soroban_events`,
   `soroban_invocations`, `nfts` reference `assets` from the start.

2. **Constraint / index rename** — all `ck_tokens_*`, `uidx_tokens_*`,
   `idx_tokens_*` renamed to `ck_assets_*`, `uidx_assets_*`, `idx_assets_*`
   (applied in the same base migration edit).

3. **Label remap** — `asset_type` value `classic` → `classic_credit`. The
   `token_asset_type_name` SQL helper updated accordingly. `native`, `sac`,
   `soroban` unchanged.

4. **Rust enum** — `TokenAssetType::Classic` → `TokenAssetType::ClassicCredit`;
   `as_str()` returns `"classic_credit"`. Name `TokenAssetType` stays (renaming
   to `AssetType` would clash with the existing XDR-level `domain::AssetType`).

5. **Rust domain + ingest** — `crates/domain/src/token.rs` → `asset.rs`;
   `struct Token` → `struct Asset`. Propagated through `xdr-parser` and
   `indexer` (see task 0154 scope).

6. **Metric field** — `tokens_ms` → `assets_ms` in `StepTimings`.

7. **API / docs spec** — `/search?type=` value `token` → `asset` for
   consistency with the renamed table. `/tokens*` routes documented as
   `/assets*` in architecture docs (routes not yet implemented).

8. **`soroban_contracts.contract_type = 'token'`** — unchanged. Contract role
   label stays; the ambiguity disappears once the table is renamed.

---

## Rationale

- Stellar's own taxonomy calls these "Assets" at the umbrella level; our table
  holds all four sub-types including classic ones with no contract surface.
- Eliminating the token/token collision in schema discussions has concrete team
  value; the effort is bounded (mechanical rename, guarded by `cargo check`).
- `classic_credit` matches the XDR `CREDIT_ALPHANUM*` term; low extra cost
  applied in the same base migration edit.
- Hard rename (no alias, no reversible migration pair) chosen because
  neither production DB nor live API routes exist yet — no external
  consumers, no staging data, to protect.

---

## Alternatives Considered

### Keep `tokens`, add ADR documenting the divergence

**Pros:** Zero migration, no code change.

**Cons:** Recurring onboarding friction; schema/taxonomy mismatch stays; name
misleads new developers about what the table holds.

**Decision:** REJECTED — the mismatch causes real ambiguity in team
conversations and doc reviews.

### Rename `soroban` → `soroban_sep41` in the same migration

**Pros:** Full protocol-alignment for all four values.

**Cons:** Speculative (T-REX ecosystem nascent on Stellar mainnet); extra churn
for no user today; additive column `compliance_flavour` is the right model if
T-REX arrives.

**Decision:** REJECTED — see `notes/S-asset-type-label-remap.md`.

---

## Consequences

### Positive

- Schema term matches Stellar's official taxonomy.
- `soroban_contracts.contract_type = 'token'` (contract role) vs `assets` table
  (value) is now unambiguous.
- `classic_credit` removes the XLM/"classic" cognitive overlap.

### Negative

- Base migration rewritten in place — acceptable here (no production
  DB to preserve), but locks out any future replay of pre-rename state
  from SQL alone.
- ~15–20 Rust files touched; guarded by `cargo clippy -- -D warnings`
  - `cargo test -p indexer --test persist_integration` (4/4 pass).
- `tokens_ms` metric field renamed — no external dashboards yet, so no gap.

---

## References

- [Stellar Anatomy of an Asset](https://developers.stellar.org/docs/tokens/anatomy-of-an-asset)
- [SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [ADR 0031: SMALLINT enum columns](0031_enum-columns-smallint-with-rust-enum.md)
- [ADR 0032: docs/architecture evergreen policy](0032_docs-architecture-evergreen-maintenance.md)
