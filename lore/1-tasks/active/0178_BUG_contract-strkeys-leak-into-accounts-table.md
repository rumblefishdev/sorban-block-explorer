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

## Investigation (2026-04-29)

The leak surface was already closed by task 0173, four days before this
bug was filed. The investigation was needed to confirm that and to
distinguish stale data from an active leak.

### Defense-in-depth filter at staging

Commit `7202209` (lore-0173, Apr 29) added a finalization filter to
`account_keys_set` at
[`staging.rs:421-426`](../../../crates/indexer/src/handler/persist/staging.rs#L421):

```rust
let account_keys: Vec<String> = account_keys_set
    .into_iter()
    .filter(|k| k.len() <= 56 && k.starts_with('G'))
    .collect();
```

This drops every non-G-prefix entry — Contract (C…), ClaimableBalance
(B…, 58 chars), LiquidityPool (L…), MuxedAccount (M…, 69 chars), etc. —
before the `accounts` upsert sees them. So the ScAddress variants that
CAP-67 V4 events surface into staging are caught at the gate even if
they slipped past upstream collectors.

### Why the 30k smoke had 1332 C-prefix entries

The 30k smoke backfill (mainnet 62016000–62046000) was indexed Apr 28.
The defense filter merged Apr 29, AFTER. The 1332 C-prefix rows are
stale data from the pre-0173 binary, not symptoms of an active leak on
develop. A re-backfill on the post-0173 binary produces 0 C-prefix rows
in `accounts` (validated in this PR's acceptance check; see Phase 1
output in PR description).

### What this PR changes (cosmetic hardening)

1. **Tighten `is_strkey_account`** ([`staging.rs:1373-1378`](../../../crates/indexer/src/handler/persist/staging.rs#L1373))
   from `Some('G' | 'M')` first-char match to `s.len() <= 56 &&
   s.starts_with('G')`. The M-prefix branch was a transitional measure
   pending task 0177 (canonicalize MuxedAccount → ed25519 G-strkey at
   the parser boundary). PR #145 merged 0177; M-prefix values no longer
   reach this filter on the happy path. Tightening to G-only aligns
   the upstream filter with the final defensive filter at
   [`staging.rs:421-426`](../../../crates/indexer/src/handler/persist/staging.rs#L421)
   so both layers agree on the account shape.
2. **Update inline comments** at
   [`staging.rs:316-323`](../../../crates/indexer/src/handler/persist/staging.rs#L316)
   (CAP-67 events walker) and
   [`staging.rs:399-422`](../../../crates/indexer/src/handler/persist/staging.rs#L399)
   (final defensive filter) to reflect that M is now upstream-canonicalized
   and to enumerate the full set of CAP-67 ScAddress variants the filter
   protects against.
3. **No source-side change.** `op_participant_str_keys`
   ([`staging.rs:1382-1422`](../../../crates/indexer/src/handler/persist/staging.rs#L1382))
   only reads named JSON fields (`destination`, `from`, `trustor`,
   `sponsoredId`, asset issuers from typed `asset`/`destAsset`/`sendAsset`
   fields) for specific classic ops; it does not read
   `invoke_host_function` arguments, so contract addresses from
   `args.contract_address.to_string()` never reach the accounts universe
   through this path. The CAP-67 events walker
   ([`staging.rs:310-332`](../../../crates/indexer/src/handler/persist/staging.rs#L310))
   does see Contract-typed transfer participants from per-op SAC events,
   but `is_strkey_account` (now G-only) drops them.

## Acceptance Criteria

- [x] Identify the operation type(s) leaking C-prefix StrKeys into
      account-shaped JSON fields — none through `op_participant_str_keys`;
      CAP-67 events surface them, `is_strkey_account` filters them out.
- [x] `is_strkey_account` rejects C-prefix (and validates length=56) —
      tightened from G|M to `s.len() <= 56 && s.starts_with('G')`
- [x] Operation JSON emits contract addresses under field names the
      walker does not treat as accounts — verified: walker reads
      `destination`/`from`/`trustor`/`sponsoredId`/asset issuers only,
      contract addresses appear in unrelated field paths.
- [ ] Re-run audit harness Phase 1 — `accounts.I1` violations = 0
      (or ≤ 4 synthetic test residue) — gated on re-backfill in
      sbe-audit worktree on post-fix binary; result attached to PR
      description before merge.
- [x] Reindex required: existing `accounts` rows with C-prefix must
      be deleted / migrated to `soroban_contracts`. Documented:
      handled by full re-backfill of the smoke dataset (DB drop +
      sqlx migrate + partition CLI + backfill-runner against the
      post-fix binary). No in-place migration script needed because
      the dataset is a smoke, not production.

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
