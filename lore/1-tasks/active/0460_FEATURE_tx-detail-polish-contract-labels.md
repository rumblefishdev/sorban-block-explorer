---
id: '0460'
title: 'FEATURE: transaction-detail polish — contract labels + small UX debts from 0453'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0453', '0297']
tags:
  [frontend, backend, transaction-detail, ux, priority-medium, effort-medium]
links: []
history:
  - date: '2026-07-30'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0453: the deliberate deferrals and small debts that did
      not merit their own tasks, grouped so they do not rot as prose.
  - date: '2026-07-30'
    status: active
    who: karolkow
    note: >
      Activated to ship items 8 (failed-banner redesign) and 10 (picker
      width) directly on develop per the post-ship review.
---

# FEATURE: transaction-detail polish

## Scope (each independently shippable)

1. **Contract display names** — stellar.expert shows `[Kale] CDL7…`; the
   name lives ON-LEDGER in instance storage `Symbol("METADATA")` (the
   off-ledger verdict in 0156/0283/0297 was chain-refuted — see project
   memory). Surface it in the invoke headline + Authorized calls tree.
2. **Picker mini-headlines** — SHIPPED 2026-07-31: each picker row carries
   the humanizeOp sentence as a second line, elided to one line. Rows whose
   type has no sentence template omit the line rather than echo the label
   above it. Closed the second half of issue #370.
3. **Fee-bump fee breakdown** — SE shows non-refundable/refundable split;
   needs result-meta fee fields (may ride on 0457's enrichment).
4. **Pre-Protocol-23 note** — scope corrected 2026-07-30 after the 0462
   research: tx-level-only events now attach via the emitter-match
   fallback where provable, so the note is needed ONLY for the truly
   empty case — an invoke tx whose archive meta has no diagnostic events
   at all (trace absent, auth-tree fallback shown). One quiet line there:
   "no execution record for this older transaction — outcomes in the
   Events section", until 0457 renders real effects for that tier.
5. **Adaptive index** — the deferred layout option: hide the picker when a
   transaction has exactly one operation (87% of mainnet); revisit with
   the team.
6. **Toggle-as-preference decision** — the wave-5 question parked for
   evidence: an Etherscan-style "always expand raw" preference; add the
   cheap click-counter telemetry first (product call).
7. **XdrRow / DisclosureRow unification** — when a third disclosure-style
   consumer appears, not before.
8. **Failed-transaction banner redesign** — SHIPPED 2026-07-30 (develop):
   replaced the floating red box with a full-bleed thin status strip under
   the Summary header, borrowing the error Chip's palette (`surface.error`
   bg, `text.error` text + 16px ErrorOutline icon, `stroke.error` bottom
   border) so it stays in the house style without adding a MuiAlert theme.
   The "not applied" chip on the operation card is untouched.
9. **Story-chip placement**: next to the Success chip it reads as a second
   status — move next to the page title or the Operations header.
10. **Picker width** — SHIPPED 2026-07-30 (develop): grid is now
    `md: 5/7, lg: 4/8` (was 50/50); pairs with item 5, adaptive index.
11. **Clickable values in headlines/facts** — SHIPPED 2026-07-31, without
    the ReactNode rewrite this item assumed. `humanizeOp` still returns a
    string; a presentation component splits it and replaces each SHORT form
    with the house identifier control. The full values are collected from
    the light row and heavy details — never parsed out of the sentence — so
    no address can be invented, and the ~20 sentence templates stayed
    untouched. Only well-formed 56-char strkeys with a detail page link;
    hex balance ids and asset codes stay plain (same rule as #14). Assets
    in sentences are still plain text — the sentences print a code, not a
    canonical id, so there is nothing to resolve; revisit with 0456.
12. **Route-strip edge labels**: hop amounts are the pools' actual payouts
    while the headline shows the min/max bound — label edges (e.g. "actual")
    so the two numbers read as different facts, not a mismatch.
    Finding (2026-07-30): `claimedAtoms` carries BOTH sides of each fill
    (`amountSold` + `amountBought`, `operation.rs::append_pool_claims`), and
    path-payment hops chain exactly (output of hop N = input of hop N+1) —
    so an order-book hop whose NEXT hop is a single pool fill can borrow
    that atom's `amountBought` as its own output (e.g. the missing VELO→XLM
    edge = the XLM the XLM/KALE pool took in). FE-only; only valid when the
    next hop has exactly one atom (multi-fill hops split the input).
    Better root fix (same day): `claim_atoms()` already returns the
    ORDER-BOOK atoms too (full sold/bought amounts + seller) — only
    `claim_lp_atoms` filters them out before `claimedAtoms` is built. Emit
    them with a `source: orderBook|pool` marker and every edge gets its
    amount directly; pool-volume consumers filter on `poolId`. Belongs to
    0457's scope (it owns fill amounts), supersedes the borrowing trick.
13. **Raw-data completeness** — SHIPPED 2026-07-30 (feat/0462 branch), both
    halves: (a) `result_meta_xdr` exposed on `heavy` (the parser always had
    it — `ExtractedTransaction.result_meta_xdr`; the 0046 spec's "NOT
    returned" decision reversed, DTO field + XdrRow added, types
    regenerated; visible after the next backend deploy); (b) every XDR row
    carries a "Decode in Stellar Lab" deep link (shipped earlier this
    branch, `type` param included — Lab mis-guesses without it).
14. **Strkey-aware JSON viewer** — SHIPPED 2026-07-30 (feat/0462 branch):
    strings matching `^[GCL][A-Z2-7]{55}$` inside `HighlightedJson` render
    as the house `IdentifierWithCopy` (link + copy button,
    `tone`/`fontSize` inherit) while keeping JSON string colour and
    quotes; near-miss strings and raw base64 untouched. Muxed M-addresses
    left as strings (no detail route).
    **Corroborated bytes decode — built, then WITHDRAWN same day** (review:
    raw JSON must stay raw; an annotation that sometimes appears and
    sometimes not reads as magic even at zero false positives). The
    technique stays on record for a NON-raw surface if one ever wants it:
    decode a 32-byte value as C/G/L candidates and show it only when the
    result already occurs elsewhere in the same transaction. The strkey
    encoder lives on in `strkeyDecode.ts` (the trace's `on X` uses it).
15. **Root-call full inline arguments** (stellar.expert parity, review
    thread 2026-07-30): SE renders the ROOT invocation with every literal
    argument inline (wrapping over 2-3 lines), e.g.
    `swap_collateral(GC4Q…, [SolvBTC] CBIJ…, …, 13171, …) → 81404538`,
    while our root falls back to `(6 args)` under the 40-char budget.
    Proposal: depth-0 call gets an unlimited budget with wrapping allowed
    (nested calls keep the one-line budget); pairs with item 1 (display
    names — the `[SolvBTC]` half of SE's line) and optionally SE-style
    type subscripts.
16. **Claimable-balance claimants are a bare count** — SHIPPED 2026-07-30
    (feat/0462 branch): `claimants` in the op details IS now the vec —
    `[{destination, predicate}, …]`, predicate as full recursive tagged
    JSON, no lossy summary and no parallel count field (review ruling the
    same day: no version-compat dual shapes — one truth shape, counts
    derived from the list). humanizeOp names one or two claimants outright
    ("Escrowed 5 USDC for GA5X…GKTM and GBMD…OIPZ"), three-plus become
    "for N claimants" from the same list; list absent → no clause at all
    (nothing fabricated). Addresses inside Operation details JSON
    auto-link via the strkey-aware viewer (#14). Until the backend deploy
    the live page shows "Escrowed 5 USDC" with no claimant clause — the
    deploy wakes it, verified via the issue-close flow.
    Original finding (0462 review, tx `a821ee85…` op 3): the card said
    "for 2 claimants" but could not name them — the parser reduced the
    claimants XDR vec to `claimants: 2`.

17. **One event line, shown once per operation** — SHIPPED 2026-07-31,
    found by reviewing production right after the deploy. The card's own
    events section had been dormant (it filters on `op_index`, which the
    API only started emitting that day) and came back rendering a weaker
    copy of what the trace already showed. The event line is now a shared
    component; the card renders it only when there is no execution trace,
    which is the classic-operation case where nothing else shows events.
    Canonical asset ids shorten like every other identifier — at 60 chars
    they had pushed every classic transfer into a bare `(4 fields)`.

18. **"Escrowed … for A and B" reads as a destination.** The claimants are
    who MAY claim later, under a predicate; the operation moves the asset out
    of the payer into a NEW ledger entry with its own `B…` address, and
    nobody on the claimant list receives anything at creation time. The
    sentence item 16 shipped is accurate and still misread, because the actual
    recipient is named nowhere on the card — it appears only inside the event
    line, as a `B…` the reader has no reason to recognise. Reviewed against a
    real transaction (2026-09-09): the payer is ALSO one of the two claimants,
    every operation creates its own distinct `B…`, and the other claimant has
    not one row against it anywhere in the transaction. Fix is wording plus
    naming the escrow itself, not new data.
19. **The amount and the asset are printed twice per operation** — once in the
    event line, once in Operation details — with nothing saying they are one
    fact from two sources. A reader cannot tell whether they are looking at
    two numbers or one repeated, which is the same failure mode item 17 fixed
    for the event line itself.
20. **A canonical asset id is not an identifier anywhere on the card.** Bare
    strkeys became copyable links in items 11 and 14, but `CODE:ISSUER` matches
    no strkey pattern, so it stays plain text in the event line AND inside the
    details JSON — not copyable, not linkable, while every address beside it
    is both. Item 11 already recorded the cause ("the sentences print a code,
    not a canonical id") and deferred the resolution to [[0456]]; this item is
    the presentation half, and it is now visible in two places rather than one.
21. **An operation card gives no sense of scale.** A transaction of 85
    identical claimable-balance operations renders one card with one amount,
    while the account page's `Balance change` column sums the whole
    transaction. Both are correct and they disagree on screen with nothing
    reconciling them. The picker shows the count; the card does not say which
    of how many it is.

## Status (2026-07-31)

Shipped: **2, 5, 8, 9, 10, 11, 13, 14, 16, 17**. Open: **1** (contract
display names), **3** (fee split, waits on 0457), **4** (pre-P23 note),
**6** (toggle preference — product call), **7** (XdrRow unification —
waits for a third consumer), **12** (route-strip "actual" labels),
**15** (root-call full inline args), and — added 2026-09-09 from a
fresh-eyes review of a claimable-balance transaction — **18** (escrow wording
and naming the escrow), **19** (amount and asset printed twice), **20**
(canonical asset id is not an identifier), **21** (no sense of scale on a
multi-operation transaction). The task stays active until each of those is
shipped or withdrawn with a reason.

## Acceptance criteria

- [ ] Each item shipped or explicitly withdrawn with a recorded reason
      (10 of 21 shipped; items 18-21 added 2026-09-09)
