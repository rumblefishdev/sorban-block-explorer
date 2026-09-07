---
id: '0469'
title: 'BUG: issuer home_domain is null for major issuers that carry one on-chain (USDC, AQUA)'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0450', '0188', '0316', '0443']
tags: [indexer, data-quality, assets, priority-high, effort-medium]
links: []
history:
  - date: '2026-09-03'
    status: backlog
    who: karolkow
    note: >
      Corrected the "measured and refuted" section. The whole-row clobber is
      real and verified against the chain, but at roughly 0.2% of accounts it
      cannot explain the 22.2% coverage gap, so the leading hypothesis stands
      as the answer to that. Found while shipping 0443, which reads the field
      on two pages with two different techniques and made them disagree.
  - date: '2026-08-10'
    status: backlog
    who: karolkow
    note: >
      Spawned from the 0450 audit. The assets-list issuer-domain column ships
      and renders correctly, but is empty for the two largest classic assets
      on the network. Verified against the chain: the USDC issuer carries
      `circle.com` and the AQUA issuer `aqua.network`, while our API returns
      `issuer_home_domain: null` for both. Coverage across classic assets is
      22.2 %. A whole-row RMT clobber was suspected and refuted by
      measurement — this is missing capture, not overwriting.
---

# BUG: `home_domain` never captured for issuers that do not transact

## Summary

`accounts.home_domain` is null for issuers that demonstrably carry one on the
ledger. The assets-list column added by [[0450]] therefore renders blank for
the very assets it was built to serve.

## Evidence

| Asset                 | Issuer          | On-chain `home_domain` | Our API |
| --------------------- | --------------- | ---------------------- | ------- |
| USDC                  | `GA5ZSEJY…KZVN` | `circle.com`           | `null`  |
| AQUA (61 863 holders) | `GBNZILST…AQUA` | `aqua.network`         | `null`  |

On-chain values read from Horizon's account endpoint — a raw ledger-entry
field, not a derived one, so the usual caution about validating against a
source that re-derives the field does not apply here.

Coverage: **75 288 of 338 963 classic assets (22.2 %)** resolve an issuer
domain. Across all accounts, 1 014 072 carry one.

## Partly un-refuted — the clobber is real, but it is not the main cause

The `accounts` table is a ReplacingMergeTree and its own schema comment warns
that reads must take the latest version, so a whole-row clobber looked like
the obvious cause: a batch that sees the account only as a participant writes
a fresh row with a bumped `last_seen_ledger` and a null domain, and the newest
row wins.

**That measurement said zero. It is wrong** — corrected 2026-09-03 while
shipping [[0443]], which reads `home_domain` on two pages and made the
disagreement visible.

A verified counterexample, `GA22QHQPZVMQDBOCTOETGVAZBDKTPTK3STYQF5AEQEOFT6TATU6AFWY3`:

| ledger     | closed     | `home_domain`          |
| ---------- | ---------- | ---------------------- |
| 63 407 950 | 2026-07-10 | `coollifeclc.xmint.io` |
| 63 635 833 | 2026-07-25 | `NULL`                 |

`accounts FINAL` returns `NULL`. The chain, asked directly via
`getLedgerEntries` and decoded with the official `stellar xdr` CLI, returns
`coollifeclc.xmint.io`. The clobbering row predates the 2026-08-10
measurement, so this is not drift since — the earlier count was simply wrong.

Reproduce (per prefix slice — a full-table `GROUP BY` exceeds the read memory
limit):

```sql
SELECT count() FROM (
  SELECT id FROM accounts WHERE account_id LIKE 'GA22%' GROUP BY id
  HAVING maxIf(last_seen_ledger, home_domain IS NULL)
       > maxIf(last_seen_ledger, home_domain IS NOT NULL)
     AND countIf(home_domain IS NOT NULL) > 0)
```

**Scale, and why the hypothesis below still stands.** Three slices measured
2026-09-03: `GA22%` 13, `GB33%` 6, `GC44%` 6 — 25 accounts in roughly 11 700,
about 0.2%. Every one of the 13 in the first slice was checked against the
chain: **13 of 13 still carry the domain there**, so none is a user clearing
it. But 0.2% cannot explain a 22.2% coverage figure. The clobber is a real
second mechanism, not the explanation — treat it as a correctness bug in its
own right (it belongs to the class audited by [[0316]]) and keep the leading
hypothesis below as the answer to the coverage question.

**Consequence today, user-visible.** `crates/api/src/accounts/queries.rs`
reads with `FINAL` and serves `null`; the transaction-detail path added by
[[0443]] reads with `argMax(home_domain, last_seen_ledger)`, which skips the
NULL and matches the chain. The same account therefore shows its federated
address on a transaction page and not on its own account page. `argMax` is a
read-side patch, not a fix: it cannot distinguish "this batch did not carry
the field" from "the user cleared it", because the writer
(`crates/db-clickhouse/src/persist/stage.rs:838`,
`home_domain: ov.and_then(|o| o.home_domain.clone())`) writes `NULL` for both.

## Leading hypothesis (unproven)

`extract_account_states` reads `home_domain` from the account entry inside
ledger-entry changes (`crates/xdr-parser/src/state.rs:474`). An issuer's own
`AccountEntry` appears in those changes only when the issuer's own account is
modified — issuing an asset does not touch it, the holder's trustline does.
An issuer that set its domain before our indexing window and has not submitted
a transaction since would therefore never be seen, while `last_seen_ledger`
still advances through the participant path.

This explains both the null and the 22.2 % coverage, but it has not been
proven. Establish it before building anything.

## Implementation sketch

If the hypothesis holds, the value cannot come from the change stream alone —
it needs the account's current state. Options to weigh, not a decision:

- seed from a ledger-entry snapshot for accounts first seen before the window
- resolve on demand for issuers only (a bounded set) via the same runtime
  enrichment path that already fetches SEP-1 documents (task 0188)
- accept the gap and make it legible rather than silent

Whatever is chosen, a blank column must not read as "this issuer has no
domain" when the truth is "we never looked".

## Acceptance criteria

- [ ] Root cause established and recorded — hypothesis confirmed or replaced
- [ ] The clobbered minority is fixed too, or explicitly deferred to [[0316]]
      with its count restated — a fix that raises coverage while still serving
      `null` for accounts whose value we hold on an older row is half a fix
- [ ] Whatever the read path becomes, the account endpoint and the
      transaction-detail endpoint agree on the same account
- [ ] USDC and AQUA resolve their real domains, or the UI states plainly that
      the value is unknown rather than absent
- [ ] Coverage measured before and after, on the same population
- [ ] No regression in the 22.2 % that already resolve
- [ ] **Docs updated** — the accounts/assets contract under
      `docs/architecture/**` states where `home_domain` comes from and what a
      blank means
- [ ] **API types regenerated** — if the wire shape changes
