---
id: '0178'
title: 'BUG: 1332 contract StrKeys (C-prefix) leak into `accounts.account_id` instead of `soroban_contracts`'
type: BUG
status: active
related_adr: ['0026', '0030', '0037']
related_tasks: ['0044', '0173', '0175', '0177']
tags: [priority-high, layer-parser, layer-persist, audit-driven, taxonomy]
links:
  - crates/indexer/src/handler/persist/staging.rs
  - crates/xdr-parser/src/operation.rs
history:
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Surfaced by task 0175 Phase 1 SQL invariants on full 30k smoke
      backfill (ledgers 62016000–62046000). 1332 rows in `accounts`
      have C-prefix StrKeys, which are Soroban contract addresses
      and should live in `soroban_contracts`, not `accounts`. Per
      ADR 0026 + ADR 0030 the two registries are kept disjoint
      (account = ed25519 G-key, contract = C-prefix StrKey hash).
  - date: '2026-04-29'
    status: active
    who: stkrolikiewicz
    note: >
      Defense-in-depth filter at staging.rs:421-423 (commit 7202209,
      lore-0173, Apr 29) already drops non-G/oversize strkeys before
      accounts upsert: `k.starts_with('G') && k.len() <= 56`. 30k
      smoke was indexed Apr 28, *before* the filter merged, so the
      1332 baked-in C-prefix entries are stale data from the
      pre-0173 binary, not an active leak. Re-backfill on develop
      validates 0 violations. This task lands cosmetic hardening:
      `is_strkey_account` tightened from G|M to G+len56 (M-path
      obsolete since 0177 muxed canonicalization at parser).
---

# Contract StrKeys leaking into accounts table

## Summary

After indexing 30,001 ledgers (mainnet 62016000–62046000), the
`accounts.account_id` column contains 1,332 rows with a `C…` prefix
— Soroban contract StrKeys, not Stellar accounts. Per ADR 0026 +
ADR 0030 these are distinct registries; mixing them violates the
surrogate-id invariant and any downstream consumer that joins on
`accounts` will silently include contracts in account-shaped queries.

Phase 1 invariant `accounts.I1` (`StrKey shape`) flagged 1336 total
violations:

- 4 synthetic `GAAA…` test residue (acceptable, integration-test
  fixtures)
- **1332 C-prefix StrKeys** ← this bug
- 0 muxed M-keys (task 0177 hot-fix held)

Sample: `CCQTGB3YIFHAGPNR7RFDKGW3IPPKQOVUAED6KJFIFTQ5ZWAWEM2FB7UO`
(plus 1331 more).

## Reproduction

```bash
DATABASE_URL=... crates/audit-harness/run-invariants.sh \
    --out /tmp/audit.md
# accounts I1 violations: 1336 (1332 C-prefix + 4 test residue)
```

```sql
SELECT count(*) FROM accounts WHERE account_id LIKE 'C%';
-- 1332
```

## Hypothesis

The persist staging layer
([`crates/indexer/src/handler/persist/staging.rs:1303`](../../../crates/indexer/src/handler/persist/staging.rs#L1303))
filters by prefix `G`/`M` only:

```rust
fn is_strkey_account(s: &str) -> bool {
    matches!(s.chars().next(), Some('G' | 'M'))
}
```

But the JSON-detail walker
([`op_participant_str_keys`](../../../crates/indexer/src/handler/persist/staging.rs#L1314))
that feeds `is_strkey_account` reads StrKeys out of operation `details`
JSON. Some op variants — likely `invoke_host_function` (contract
addresses surface in `args.contract_address.to_string()`,
[`operation.rs:321`](../../../crates/xdr-parser/src/operation.rs#L321))
or `set_trust_line_flags` for SAC trustlines — emit C-prefix StrKeys
into JSON fields the walker treats as account references.

Two candidate fix points:

1. **Tighten filter at staging:** require `is_strkey_account` to also
   verify length 56 and prefix `G` (not `M`, which is also wrong post
   task 0177 unwrap). Drops anything that doesn't match a real account.
2. **Don't emit contract StrKeys into account-shaped JSON keys:**
   audit `operation.rs` JSON construction. Contract addresses belong
   in a separate field name (e.g. `contract_id`) that the walker
   doesn't slurp into the accounts universe.

The right fix is probably both: defensive filter + clean source.

## Acceptance Criteria

- [ ] Identify the operation type(s) leaking C-prefix StrKeys into
      account-shaped JSON fields (likely invoke_host_function and/or
      SAC trustline ops)
- [ ] `is_strkey_account` rejects C-prefix (and validates length=56)
- [ ] Operation JSON emits contract addresses under field names the
      walker does not treat as accounts
- [ ] Re-run audit harness Phase 1 — `accounts.I1` violations = 0
      (or ≤ 4 synthetic test residue)
- [ ] Reindex required: existing `accounts` rows with C-prefix must
      be deleted / migrated to `soroban_contracts`. Document in PR.

## Notes

- **Operationally severe.** Every join from `transactions.source_id`
  / `operations_appearances.source_id` / `nfts.current_owner_id`
  etc. to `accounts.account_id` could silently surface a contract
  StrKey where a G-key was expected. API responses passing these
  to clients would be lying about the entity type.
- **Discovered alongside 0177 muxed leak.** Both are
  unwrap-at-extraction failures: 0177 lets M-keys through, 0178 lets
  C-keys through. Different prefix, same class of bug.
- **Companion to ADR 0030** — that ADR mandates surrogate BIGINT for
  contracts; this bug is the symptom of the parser side not enforcing
  the contract/account split before persist.
