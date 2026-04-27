---
id: '0038'
title: 'Permit native XLM-SAC with NULL identity in ck_assets_identity'
status: accepted
deciders: [stkrolikiewicz]
related_tasks: ['0160']
related_adrs: ['0023', '0036', '0037']
tags: [database, schema, sac, identity, assets]
links:
  - crates/db/migrations/20260427000000_sac_identity_native_allowance.up.sql
  - crates/db/migrations/20260427000000_sac_identity_native_allowance.down.sql
  - crates/xdr-parser/src/sac.rs
  - crates/indexer/src/handler/persist/write.rs
history:
  - date: '2026-04-27'
    status: accepted
    who: stkrolikiewicz
    note: >
      ADR drafted post-implementation. Loosening shipped in
      branch `fix/0160_sac-asset-identity-extraction` (PR #120, task 0160
      re-open scope) — migration `20260427000000_sac_identity_native_allowance`.
      Snapshot ADR 0037 §326-332 records the pre-loosening shape; this
      ADR is the "thin follow-up" 0037 §533 explicitly invites for small
      schema deltas.
---

# ADR 0038: Permit native XLM-SAC with NULL identity in `ck_assets_identity`

**Related:**

- [ADR 0023: Tokens typed metadata columns](0023_tokens-typed-metadata-columns.md) — original `ck_assets_identity` shape
- [ADR 0036: Rename `tokens` to `assets`](0036_rename-tokens-to-assets.md) — table renamed; constraint moved with it
- [ADR 0037: Current schema snapshot](0037_current-schema-snapshot.md) — schema snapshot prior to this loosening (anchor migration `20260424000000`); §326-332 records the strict form
- [Task 0160: SAC deployments never land in assets](../1-tasks/active/0160_BUG_sac-asset-identity-extraction.md)

---

## Context

Task 0160 surfaced that `xdr_parser::detect_assets` was silently dropping every SAC deployment because the persist layer required `(asset_code, issuer_id)` for `asset_type = 2` (SAC). The initial fix (sentinel approach: synthesised `"XLM"` + all-zero Ed25519 issuer StrKey for native XLM-SAC) was rejected at PR review because:

1. The synthesised issuer is not a Stellar convention — Horizon and the SDKs render `Asset::Native` as `{"asset_type":"native"}` with no issuer field at all.
2. The sentinel would leak into downstream API responses, exposing a fake account that has no on-chain existence.

The native XLM Stellar Asset Contract (`CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` on mainnet) is a real, deterministic on-chain entity that wraps the native asset for use in Soroban — by spec, it has no `(asset_code, issuer)` pair. The schema must represent that honestly.

---

## Decision

`ck_assets_identity` accepts a second shape under `asset_type = 2`:

```sql
ALTER TABLE assets DROP CONSTRAINT ck_assets_identity;
ALTER TABLE assets ADD CONSTRAINT ck_assets_identity CHECK (
    (asset_type = 0
        AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
 OR (asset_type = 1
        AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 2
        AND contract_id IS NOT NULL
        AND (
            (asset_code IS NOT NULL AND issuer_id IS NOT NULL)  -- classic SAC
         OR (asset_code IS NULL     AND issuer_id IS NULL)       -- native XLM-SAC
        ))
 OR (asset_type = 3
        AND issuer_id IS NULL      AND contract_id IS NOT NULL)
);
```

For `asset_type = 2` the row now must carry `contract_id` plus **either** both `(asset_code, issuer_id)` (classic-credit SAC) **or** neither (native XLM-SAC). The mixed cases — one of `(asset_code, issuer_id)` set, the other NULL — remain blocked by SQL operator precedence (`AND` binds tighter than `OR`), so the "both or neither" invariant for SAC identity holds.

Shipped as forward-only migration `20260427000000_sac_identity_native_allowance.up.sql` with a matching `.down.sql` that restores the strict pre-loosening form (operator must purge any native XLM-SAC rows before downgrade).

---

## Rationale

The schema already routes deduplication through partial unique indexes:

- `uidx_assets_classic_asset (asset_code, issuer_id) WHERE asset_type IN (1, 2)` — Postgres treats `NULL` as distinct from any other value in UNIQUE (default `NULLS DISTINCT`), so native XLM-SAC rows (NULL+NULL) are simply ignored by this index.
- `uidx_assets_soroban (contract_id) WHERE asset_type IN (2, 3)` — covers both classic SAC and native SAC dedup; one row per contract regardless of variant.

Loosening the CHECK is therefore the single, minimal change required: existing dedup machinery already accommodates the new shape. The persist layer in `crates/indexer/src/handler/persist/write.rs` splits `asset_type = 2` rows into two paths (`sac_credit` keyed by classic identity, `sac_native` keyed by `contract_id`) and routes them to the correct upsert helper.

Scope is intentionally narrow: only the constraint changes. No new columns, no new indexes, no FK shape changes. Compatible with every code path that wrote a SAC row before this ADR (those paths now also write classic SAC, just with the additional XLM-SAC variant unlocked).

---

## Alternatives Considered

### Alternative 1: Synthesised XLM-SAC issuer sentinel

**Description:** Mint a synthetic `accounts` row for the all-zero Ed25519 StrKey (`GAAA…AWHF`) and use it as `issuer_id` for native XLM-SAC rows. No CHECK constraint change required; seed handled in a DML edit to migration `0002`.

**Pros:**

- Minimal schema delta — no migration touches the `assets` table itself.
- All `asset_type = 2` rows have the same shape (always `contract + code + issuer`), simplifying downstream consumers.

**Cons:**

- Sentinel `GAAA…AWHF` is not a Stellar convention; leaks a fake account into the public API.
- Editing migration `0002` breaks sqlx checksum on every already-applied DB.
- Operator inspecting the database sees an "issuer" account that has no on-chain existence — debugging hazard.
- Diverges from Horizon / SDK rendering of native asset, requiring downstream filters to hide the sentinel.

**Decision:** REJECTED at PR review (#120 round 1). All four downsides materialised in code review. Reverted in commits `e7f0b6c..635e031` of the same branch; replaced by this loosening.

### Alternative 2: Route native XLM-SAC as `asset_type = 3` (Soroban-native)

**Description:** Reuse the existing `asset_type = 3` (Soroban-native) shape — `(NULL, NULL, contract_id)` — for native XLM-SAC. No constraint change; just classify XLM-SAC under a different type.

**Pros:**

- Zero schema change.

**Cons:**

- Semantic mismatch: `asset_type = 3` means "Soroban-native SEP-41 token" (a contract that is a token from inception, no classic backing). Native XLM-SAC IS a wrap of the classic native asset — same semantic class as classic-credit SACs. Routing it to `3` would mislead every downstream consumer about what the contract actually represents.
- Asset-list UI / public API would need a special case to render XLM-SAC as "native" rather than "Soroban-native" — pushing the conditional into many places instead of one CHECK constraint.

**Decision:** REJECTED — `asset_type` is a semantic classifier, not a "did it fit in this shape" tag. Reusing `3` would corrupt the meaning of the column for every other consumer.

### Alternative 3: Leave native XLM-SAC unindexed

**Description:** Detect XLM-SAC deployments but don't insert a row. Native asset already has a row at `asset_type = 0`; treat the SAC contract as "the well-known wrap" without separate indexing.

**Pros:**

- No schema change.

**Cons:**

- Loses the `contract_id` mapping. The explorer's `/contracts/{C…}` endpoint should be able to navigate from the XLM-SAC contract to its asset row; without an entry, that link breaks.
- Inconsistent with classic-credit SAC handling (which gets its own row distinct from the classic-credit row at `asset_type = 1`). Asymmetry without justification.
- Foregoes the `holder_count` / `total_supply` reporting that 0135 will populate per-SAC.

**Decision:** REJECTED — explorer needs the SAC contract addressable as an asset row regardless of underlying classic identity.

---

## Consequences

### Positive

- Native XLM-SAC honestly represented: `(asset_type=2, asset_code=NULL, issuer_id=NULL, contract_id=CAS3J7G…OWMA)`. API responses match Horizon / SDK conventions for native asset (no synthetic issuer surfaced).
- `assets` table gains a real row for the well-known mainnet XLM-SAC contract; future `holder_count` / `total_supply` workers (task 0135) populate it like any other SAC.
- Single source of truth for the loosening: one constraint, one migration, no cross-artifact synchronisation (the rejected sentinel approach required keeping a Rust const + migration DML + drift-guard test in sync).
- "Both or neither" invariant for SAC identity preserved — operator precedence in the CHECK guarantees no half-classic SAC row can be written.

### Negative

- `assets.asset_type = 2` rows now have two possible shapes; downstream code that consumes SAC rows must handle both. The persist layer already does (`write.rs` split into `sac_credit` / `sac_native` upsert paths); any future read-path consumer that filters on `asset_code IS NOT NULL` for SAC rows will silently miss XLM-SAC.
- Down migration requires manual purge of native XLM-SAC rows before the strict constraint can be re-applied. Documented inline in the down script.
- ADR 0037's snapshot DDL (§326-332) is now stale wrt the live schema. See _Open questions_ below.

---

## Open questions

- **Coordination with ADR 0037**: 0037 is a "current schema snapshot" ADR authored by @fmazur, anchored on migration `20260424000000` (12 migrations applied). The loosening adds a 13th migration and changes the CHECK shape recorded at 0037 §326-332. Per 0037's own §533 _"a thin follow-up ADR referencing this one is an acceptable substitute for small deltas"_ this ADR is exactly that. Decision deferred to @fmazur whether 0037's body should be updated inline (re-anchored to migration `20260427000000`, DDL block refreshed) or remain frozen at the original anchor with this ADR serving as the official delta. The `related_adrs` list in 0037's frontmatter is updated to include this ADR either way. No code or runtime impact.

---

## Delivery Checklist

Per [ADR 0032](./0032_docs-architecture-evergreen-maintenance.md), any ADR that changes the shape of the system MUST be landed together with the corresponding updates to `docs/architecture/**`. Tick each that applies before marking the ADR `accepted`:

- [ ] `docs/architecture/technical-design-general-overview.md` updated (or N/A) — **N/A — pre-existing drift from task 0164: §6.7 still shows the dropped `description` / `home_page` columns and a generic `-- CHECK ck_assets_identity` placeholder. Refreshing the constraint shape there in isolation would land mid-drift; this ADR scopes the architecture-doc update to `database-schema-overview.md` (the canonical reference). Pre-existing drift left for a separate cleanup task.**
- [x] `docs/architecture/database-schema/database-schema-overview.md` updated — `ck_assets_identity` block refreshed to match the new shape (§assets table) with reference to this ADR
- [ ] `docs/architecture/backend/backend-overview.md` updated (or N/A) — **N/A — backend reads the constraint indirectly via the persist layer; no constraint shape is documented there**
- [ ] `docs/architecture/frontend/frontend-overview.md` updated (or N/A) — **N/A — constraint is DB-internal; frontend consumes API responses, not table CHECK shapes**
- [ ] `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` updated (or N/A) — **N/A — indexing-pipeline overview describes data flow shape, not the persist-layer CHECK constraint**
- [ ] `docs/architecture/infrastructure/infrastructure-overview.md` updated (or N/A) — **N/A — infrastructure-level concerns**
- [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` updated (or N/A) — **N/A — XDR-parsing overview covers parser output shape (`ExtractedAsset`); the typed `SacAssetIdentity` enum surfaces NULL+NULL for native XLM-SAC and is consumed by the persist layer that knows about the loosened CHECK. No XDR-parsing-overview rewrite needed.**
- [x] This ADR is linked from each updated doc at the relevant section — link added to `database-schema-overview.md` §assets

---

## References

- Migration: [`20260427000000_sac_identity_native_allowance.up.sql`](../../crates/db/migrations/20260427000000_sac_identity_native_allowance.up.sql)
- Stellar XLM-SAC mainnet contract: `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` (deterministic per stellar-core: `SHA256(network_id || XDR(ContractIdPreimage::Asset(Asset::Native)))`)
- Stellar Horizon "Anatomy of an Asset": `https://developers.stellar.org/docs/learn/encyclopedia/transactions-specialized/anatomy-of-an-asset` — native asset has no issuer field
- PR review thread (round 1, sentinel rejection): https://github.com/rumblefishdev/soroban-block-explorer/pull/120
