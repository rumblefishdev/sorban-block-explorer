---
id: '0535'
title: 'BUG: a link in content is indistinguishable from plain text, and the app carries two definitions of one'
type: BUG
status: active
related_adr: []
related_tasks: ['0062', '0467', '0472']
tags:
  [frontend, ui, accessibility, design-system, priority-medium, effort-small]
links: []
history:
  - date: 2026-09-03
    status: backlog
    who: stkrolikiewicz
    note: >
      Found while asking why the SAC → asset link on a contract detail page
      goes unnoticed. It is not under-emphasised: `IdentifierDisplay` renders a
      link in `text.primary` with `textDecoration: none`, so the only signal is
      the mouse cursor. Scope is the shared component and the rule, not that one
      chip.
  - date: 2026-09-03
    status: active
    who: stkrolikiewicz
    note: >
      Promoted to active. No research phase needed — the defect and the two
      competing link definitions are both located in the code, and the open
      question is the rule, which is settled in the body.
---

# BUG: a link in content is indistinguishable from plain text

## Summary

[`IdentifierDisplay`](../../../libs/ui/src/identifiers/IdentifierDisplay.tsx)
is the shared renderer for every identifier in the explorer — account, contract,
asset, hash, pool, token. A linked one is drawn in exactly the same colour and
weight as an unlinked one:

```ts
color: theme.palette.text.primary,   // identical to surrounding body text
textDecoration: 'none',
'&:hover': { color: theme.palette.surface.primaryMainAlt },  // colour only
cursor: linked ? 'pointer' : 'inherit',
```

The **only** thing separating a link from static text is `cursor: pointer`. On a
mouse you must hover to find out; on touch there is no signal at all.

## Why this is a defect and not a preference

`linked={false}` is real and used — `ContractSummary` alone passes it on lines
51 and 137, next to identifiers that ARE linked. So one screen shows clickable
and non-clickable identifiers rendered identically, and nothing but trial and
error tells them apart.

WCAG 1.4.1 (Use of Color) requires that information not be carried by a visual
difference alone; here there is not even a colour difference to fall back on.

Observed, not theorised: the SAC → asset link shipped by 0472 was not noticed by
someone who knows this product and was looking at that page.

## The second half — there are two definitions of "link"

| where                                                                                                | style                        |
| ---------------------------------------------------------------------------------------------------- | ---------------------------- |
| `IdentifierDisplay` (42 usages, 32 files)                                                            | no underline, hover = colour |
| MUI `Link underline="hover"` — `NftDetailPage`, `LedgerDetailPage`, `transactions/cells`, and 3 more | underline on hover           |
| `SecondaryNav`, `Footer`, `NavButton`, `DomainChip`                                                  | `textDecoration: 'none'`     |

The same element behaves differently depending on which page you are on. 0062
(which built the component) set out "visually identical everywhere: same
truncation rules, font, hover behavior, link styling" — that held for
truncation and font, not for the link.

## Implementation

### 1. The rule, written down once

**Links in content are underlined. Navigation and chrome are not.** The nav,
footer and nav buttons keep `textDecoration: 'none'` — position and container
already say they are interactive. This rule is the deliverable; the CSS is the
easy part.

### 2. Affordance in `IdentifierDisplay`, gated on `linked`

```ts
...(linked && {
  textDecoration: 'underline',
  textDecorationColor: alpha(theme.palette.text.primary, 0.35),
  textUnderlineOffset: '3px',
}),
'&:hover': linked ? {
  color: theme.palette.surface.primaryMainAlt,
  textDecorationColor: 'currentColor',
} : undefined,
```

Three details that are load-bearing:

- **`textDecorationColor` at ~35% alpha, not a full-strength rule.** Identifiers
  live in dense tables — every hash in the transactions list is a link — and a
  solid underline on each row reads as noise rather than as affordance.
- **`textUnderlineOffset`.** Identifiers are monospace and full of descenders
  (`g`, `y`, `p` in every hash); the default underline cuts through them.
- **Gated on `linked`.** This is the whole point: the underline must be exactly
  what distinguishes `linked={true}` from `linked={false}`, which today is
  invisible.

### 3. Converge the MUI `Link` sites — narrower than planned

Of the six `underline="hover"` sites, only **one** turned out to be a content
link: `transactions/cells.tsx`, the asset code in the "value moved" cell. It now
shares `contentLinkSx` (the extracted helper both this and `IdentifierDisplay`
consume) laid over its own `text.secondary`, so it keeps the amount / code /
count colour hierarchy that cell depends on — `IdentifierDisplay` would have
rendered the code in `text.primary` at weight 500, louder than the amount beside
it.

The other five — `NftDetailPage`, `LedgerDetailPage` and their two skeleton
counterparts — are breadcrumbs (`NFTs /`, `Ledger /`), not content. On reading
the actual components: position and container already say "this takes you back
up a level", the same argument that keeps `SecondaryNav`/`Footer`/`NavButton`
out of scope. `underline="hover"` is already a reasonable breadcrumb treatment,
so these are left alone. Recorded as a correction, not silently dropped — the
"six sites" framing in the original scope assumed a headcount the code did not
support.

### 4. Chips stay as they are

[`ContractFaceChip`](../../../web/src/pages/contracts/ContractFaceChip.tsx)
wraps a `<Chip>` in a `RouterLink` with an explicit `textDecoration: 'none'`.
A chip already carries affordance in its background and shape, and underlining
text inside one reads as a rendering bug. If chip links need strengthening it is
a hover treatment on the chip surface, not on its label — separate question,
not this task.

## Acceptance Criteria

- [x] A linked identifier is distinguishable from an unlinked one **without
      hovering** — `IdentifierDisplay.test.tsx` asserts `textDecoration:
underline` when `linked` and `none` when not, in jsdom (not a rendering
      approximation — a real assertion on computed style)
- [x] The transactions list, where every row carries a linked hash, still reads
      as a table rather than a wall of underlines — verified visually (see
      Notes: an HTML mock at the shipped values, not the live app, because the
      local dev proxy needs a `DEV_API_KEY` this environment does not have)
- [x] Light and dark both checked — `IdentifierDisplay.test.tsx` renders both
      theme modes; the visual check above covered both too
- [~] The MUI `Link underline="hover"` sites converge where they are actually
  content — one of six (`cells.tsx`); the other five are breadcrumbs and
  correctly stay `underline="hover"` — see the corrected §3 above
- [x] Nav, footer and chips unchanged — not touched; breadcrumbs also unchanged
      per the finding above
- [x] **Docs updated** — N/A: component styling, not the shape of the system.
- [x] **API types regenerated** — N/A, frontend only.

## Implementation Notes

`contentLinkSx` (`libs/ui/src/theme/linkAffordance.ts`) is the one definition,
exported from the package root. `IdentifierDisplay`'s `makeIdentifierSx` became
a `theme => ({...})` function (was a plain object) so it can spread
`contentLinkSx(theme)` in — MUI's `Box` with a polymorphic `component` rejects
an `sx` **array** on some overloads, which is what an earlier version of this
tried; spreading avoids the type error entirely rather than working around it.

Not verified against the deployed app: the local dev proxy
(`DEV_API_PROXY_TARGET` + `DEV_API_KEY` in `web/.env.development`, gitignored)
needs a key this environment doesn't have, so `/contracts/…` and `/transactions`
both 500. Verified instead with a static HTML mock built from the exact shipped
values (mono font, real light/dark `text.primary`/`background` from
`theme/colors.ts`, the 0.35 alpha, the 3px offset) — screenshotted and sent to
the user, not just described. A real-app pass with a working proxy would still
be worth doing before calling this fully closed on a device.

## Not in scope

**Giving links their own colour** (the Etherscan-blue convention). It would
collide with [[0467]], which is mid-flight: light mode just moved to graphite
and the brand yellow is deliberately reserved. Introducing a new semantic colour
now fights that work. Underline first; revisit colour once 0467 closes, if it is
still wanted.

Making search hits distinguishable from one another is [[0484]] — a different
problem (telling two `USDC` rows apart), not affordance.
