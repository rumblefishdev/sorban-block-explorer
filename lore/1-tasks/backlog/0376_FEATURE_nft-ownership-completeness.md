---
id: '0376'
title: 'NFT completeness: multi-owner, contract-owner, pending visibility, collection union'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0359']
tags: [priority-medium, effort-large, layer-indexer, nft]
links: []
history:
  - date: 2026-07-13
    status: backlog
    who: karolkow
    note: 'Spawned from 0359 tracker. Bundles K1-6, K2-5, K2-6, K3-7.'
---

# NFT ownership completeness

## Summary

Close the NFT ownership gaps: the single current-owner slot loses history,
contract-held NFTs show a NULL owner (22% of NFTs / 51% of transfer rows),
pending NFTs are invisible (71K), and collection activity is not unioned on the
contract page.

## Re-measured 2026-09-08 (production)

The contract-owner gap, counted from the current tables rather than the 0359
sample: **339 of 1 089 distinct NFT owners (31%) cannot be resolved through
`accounts`, and all 339 resolve in `soroban_contracts`.** So the owner is known
for every one of them — `nfts/queries.rs` simply resolves owners against
`accounts` alone, and a contract owner therefore renders as null. A read-side
omission, not missing data.

Found while sweeping for contradictions during task 0540; the shared-vocabulary
side of it is [[0542]].

## Context

Spawned from 0359. The NFT owner is a single-slot current value; contract owners
(C-address) are dropped like other non-G participants (overlaps 0373).

## Implementation

- **K1-6** — multi-owner / owner-history (mitigated today by `/transfers`).
- **K2-5** — resolve contract-owner NULL (C-address owners; ties to 0373 non-G).
- **K2-6** — make pending NFTs visible (71K; see memory: nfts_pending load-bearing).
- **K3-7** — union NFT collection activity onto the contract page.

## Acceptance Criteria

- [ ] owner history retained (not single-slot) — K1-6
- [ ] contract-owner resolved (no NULL) — K2-5
- [ ] pending NFTs visible — K2-6
- [ ] collection activity unioned on contract page — K3-7
