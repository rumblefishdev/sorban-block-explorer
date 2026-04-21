---
id: '0134'
title: 'BUG: add envelope/meta ordering validation in indexer'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0024', '0029']
tags: [priority-medium, effort-small, layer-indexer, audit-F18]
milestone: 1
links:
  - crates/xdr-parser/src/envelope.rs
  - crates/indexer/src/handler/process.rs
  - crates/xdr-parser/src/transaction.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F18 (MEDIUM). Silent data corruption risk from envelope/meta mismatch.'
---

# BUG: add envelope/meta ordering validation in indexer

## Summary

`extract_envelopes()` and `collect_tx_metas()` are extracted independently from different
fields of `LedgerCloseMeta`. There is no assertion that their lengths match and no
hash-based cross-check that each envelope corresponds to its meta. On mismatch, the code
silently produces corrupted data (wrong operations attributed to wrong transactions) via
`envelopes.get(i)` returning `None`.

For V1/V2 parallel Soroban phases (`TransactionPhase::V1`), the flattening order across
execution stages and clusters relies on protocol convention with no runtime verification.

## Context

Currently correct by Stellar protocol convention. But if a future protocol change,
Galexie bug, or edge case causes ordering divergence, the indexer will silently pair
envelopes with wrong metas. The `parse_error` flag will be set on missing envelopes,
but a reordering (same count, wrong order) would produce silently corrupt data.

## Implementation

1. Add `assert_eq!(envelopes.len(), tx_infos.len())` in `process.rs` after extraction.
   On mismatch, fail the entire ledger (do not silently degrade).
2. Add hash-based cross-check: compare the `transaction_hash` from
   `TransactionResultPair` (already extracted as `TxInfo.hash`) against a hash computed
   from the corresponding envelope. **Important:** Stellar's transaction hash is
   `SHA256(network_id || ENVELOPE_TYPE_TX || tx_body)` — NOT `SHA256(full_envelope)`.
   This requires the network passphrase and hashing only the inner transaction body
   (without signatures). The network passphrase must be passed into the extraction
   pipeline (from config). Alternatively, a simpler length assertion (step 1) may be
   sufficient given that ordering is guaranteed by protocol convention.
3. If hash check fails for any transaction, log error and set `parse_error = true` for
   that specific transaction instead of silently using wrong data.

## Acceptance Criteria

- [ ] Length assertion: indexer fails loudly if envelope count != meta count
- [ ] Hash cross-check: each envelope's computed hash (using `SHA256(network_id ||
ENVELOPE_TYPE_TX || tx_body)`) matches its result pair hash — this is **required**,
      not optional, because length assertion alone cannot detect reordering (same count,
      wrong order) which produces silently corrupt data
- [ ] Network passphrase passed into extraction pipeline from config (needed for hash)
- [ ] On hash mismatch: transaction marked with parse_error, not silently corrupted
- [ ] Tests: simulated length mismatch triggers expected error behavior
- [ ] Tests: simulated reordering (swapped envelope/meta pairs) detected by hash check
