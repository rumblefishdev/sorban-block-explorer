---
id: '0176'
title: 'BUG: ledger.hash extracted as SHA256(LedgerHeaderHistoryEntry) instead of canonical ledger hash'
type: BUG
status: backlog
related_adr: ['0024', '0037']
related_tasks: ['0175']
tags: [priority-high, layer-parser, audit-driven, bug]
links:
  - crates/xdr-parser/src/ledger.rs
history:
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Surfaced by task 0175 Phase 2a (DB↔Horizon diff) on first smoke
      run — 5/5 sampled ledgers show hash mismatch. Sample seq=62016100:
      our DB hash `e29398...329f`, Horizon hash `ed89f8...4425`,
      previous-ledger hash `028ba5...49e2`. Three completely different
      values — we are not storing prev_hash either, just a fully
      synthetic value derived by hashing the wrong bytes.
---

# Ledger hash extracted from wrong XDR scope

## Summary

`crates/xdr-parser/src/ledger.rs:18-30` computes
`SHA256(LedgerHeaderHistoryEntry XDR)` and persists that as
`ledgers.hash`. The canonical Stellar ledger hash is
`SHA256(LedgerHeader XDR)` — and `LedgerHeaderHistoryEntry` already
carries that hash inline as its `.hash` field. Two issues in one:

1. Wrong scope of XDR being hashed
2. Redundant — we should read `header_entry.hash` directly

Net effect: **every `ledgers.hash` value in our DB is wrong** wrt the
canonical Stellar ledger hash. Every endpoint that surfaces ledger
hash returns garbage. Cross-checks against Horizon, archive XDR,
stellar.expert, and any external consumer fail.

## Reproduction

Phase 2a audit harness:

```bash
DATABASE_URL=postgres://... cargo run -p audit-harness --bin horizon-diff -- \
    --table ledgers --sample 5
```

Output: 5/5 rows mismatched on `hash` field.

Manual:

```bash
psql -tAc "SELECT encode(hash,'hex') FROM ledgers WHERE sequence=62016100"
# our: e2939860408d882b27aba6115f6d6590558d71b22623d05a721c3b152c1a329f

curl -s https://horizon.stellar.org/ledgers/62016100 | jq .hash
# theirs: ed89f88f97aebbeaf8bc9799561c950cd596d9f2471570ed4662374edbc44425
```

## Root cause

```rust
// crates/xdr-parser/src/ledger.rs:18-30
let hash = header_entry
    .to_xdr(limits)
    .map(|xdr| hex::encode(Sha256::digest(&xdr)))
    ...
```

`header_entry` is `&LedgerHeaderHistoryEntry`. Its full XDR shape:

```
struct LedgerHeaderHistoryEntry {
    Hash hash;                      // <-- already the canonical ledger hash
    LedgerHeader header;
    union switch (int v) { ... } ext;
};
```

So we serialize `(hash, header, ext)` and SHA256 that, instead of using
the `hash` field directly. The result is a completely synthetic value
that exists nowhere on the Stellar network.

## Fix

Replace the SHA256-of-entry computation with a direct read of
`header_entry.hash`:

```rust
let hash = hex::encode(header_entry.hash.0);
```

(`Hash` in stellar-xdr is `[u8; 32]` wrapped — `.0` unwraps the array.)

The `to_xdr(limits)` import + `sha2::Digest`/`Sha256` imports become
unused and should be cleaned up. The `XdrSerializationFailed` error
arm in this function disappears.

## Acceptance Criteria

- [ ] `extract_ledger` reads `header_entry.hash` directly; no SHA256
      over the entry XDR
- [ ] Unit test in `crates/xdr-parser/tests/` against a known mainnet
      ledger fixture — extracted hash matches Horizon's `hash` field
- [ ] Re-running `audit-harness/horizon-diff --table ledgers` against
      a re-indexed ledger range produces zero hash mismatches
- [ ] **Reindex required** — existing rows have wrong hashes; document
      in PR that operators must drop and re-backfill

## Notes

- Discovered by the very first run of Phase 2a (task 0175). Validates
  the harness premise — automated bulk diff catches systematic parsing
  bugs that manual audit would miss until someone tried to render a
  hash next to Horizon's.
- Post-fix: re-run Phase 2a against the re-indexed data to confirm zero
  mismatches before declaring closed.
- **Reindex blast radius:** every `ledgers.hash` row updated; downstream
  references (`transactions.hash` is independent, `transaction_hash_index`
  unrelated). No cascade beyond `ledgers`.
