---
id: '0486'
title: 'BUG: the NFT collection link is half-dead and its destination never names the collection'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0472', '0483', '0392', '0309', '0540']
tags: [frontend, nfts, contracts, priority-medium, effort-small]
links:
  - 'https://github.com/rumblefishdev/soroban-block-explorer/issues/368'
history:
  - date: '2026-08-13'
    status: backlog
    who: karolkow
    note: >
      Found by adversarial review of 0472, which added the header chip that
      links an NFT contract to `/nfts?contract={C…}`. The link inherits two
      pre-existing pipeline gaps at once, and the destination view — newly
      addressable, but built as a filtered list — never says what collection
      it is showing.
  - date: '2026-09-08'
    status: backlog
    who: karolkow
    note: >
      Second entry point found, from the other side: task 0540's `Balance
      change` column links a non-fungible movement, and landing on this view
      was rejected on sight — a list with a 56-character contract id typed
      into its search box is not where someone clicking an NFT expects to be.
      0540 now resolves the token id and links to the PIECE where it can, so
      the column reaches this view only in the cases it cannot (several pieces
      in one transaction, or a collection whose ownership rows are still
      quarantined). That narrows the exposure; it does not fix the view, and
      the reaction is evidence the anonymity is a real defect and not a
      reviewer's nitpick.
---

# BUG: a link that fails half the time, to a page that does not say what it is

## The two halves

**1. The link is keyed on classification; the tokens land by a different
pipeline — and they disagree in both directions.** Measured on prod
(2026-08-13):

|                                                                              |        |
| ---------------------------------------------------------------------------- | ------ |
| contracts classified NFT (`contract_type = 2`)                               | 132    |
| of those with ZERO rows in `nfts`                                            | **66** |
| contracts with rows only in `nfts_pending` (quarantined), classified `Other` | **67** |

Classification reads the WASM interface ("this contract IS an NFT");
`nfts` fills from events + the pending-promotion drain (task 0392's known
gap), and bespoke NFTs the classifier misses land as `Other` (the ~65-contract
classifier gap, task 0309 family). So half the linked pages render "No NFTs
match your filters", while 67 contracts that DO hold quarantined tokens get
no link at all.

**2. The destination is anonymous.** `/nfts?contract={C…}` renders the title
"NFTs", the generic subtitle, and a raw 56-char StrKey sitting in a free-text
search box. Nothing names the collection — although every row carries the
name, and the Contract ID column repeats the same value 20 times. The only
"Clear filters" affordance appears in the empty state and silently drops the
scope. A link labelled "View this collection" lands on a page that never says
which collection, or that it is a collection view at all.

## Who reaches this view (2026-09-08)

Two links now, and both hit the same anonymous page:

| From                                            | When                                                                                                                                           |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| the contract-header chip (0472)                 | always, for a contract classified NFT                                                                                                          |
| the account page's `Balance change` cell (0540) | a non-fungible movement whose piece cannot be named — several pieces in one transaction, or a collection with no promoted `nft_ownership` rows |

The second one is bounded by the same two pipeline gaps this task's first half
is about: a collection stuck in `nft_ownership_pending` has no token id to link
to, so it falls through to here. Closing 0392 / the 0309 classifier gap shrinks
that path as well as the "No NFTs match your filters" one.

## Scope

1. **Collection header.** When `contract` is set and passes `isContractId`,
   the list page titles itself as a collection view: the collection name
   (already on every row / in enrichment) or the truncated contract id,
   subtitle "NFTs issued by CBHU…A6GR" linking back to the contract page.
   Render the active contract filter as a removable chip, not typed text;
   hide the redundant Contract ID column in this mode.
2. **Honest empty state.** An empty collection reads "this collection has no
   indexed tokens" (with the pending caveat if applicable), not the generic
   "no NFTs match your filters" that blames the user's filter.
3. **Link gating — decide, do not default.** The contract endpoint carries no
   token count to gate on; adding one is 0483's API territory. Until then the
   options are: keep the always-on link (defensible once the empty state is
   honest) or hide it — record the call either way.

## Not in scope

- Promoting quarantined tokens (task 0392) or fixing the classifier (0309
  family) — this task makes the surface honest about their gaps, not close
  them.
- The collection NAME on the contract page chip itself — 0483.

## Acceptance criteria

- [ ] `/nfts?contract={C…}` names the collection and links back to the
      contract; filter shown as a removable chip; vitest case
- [ ] Empty collection shows the honest empty state; vitest case
- [ ] Link-gating decision recorded (keep always-on vs gate via 0483)
- [ ] Docs: frontend-overview NFTs section
