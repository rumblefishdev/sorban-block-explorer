---
id: '0413'
title: "BUG: net-settled understates issuer-side claimable-balance / LP of the issuer's own asset"
type: BUG
status: superseded
related_adr: []
related_tasks: ['0393']
tags:
  ['clickhouse', 'xdr-parser', 'phase-future', 'effort-medium', 'priority-low']
links: []
history:
  - date: 2026-07-20
    status: backlog
    who: karolkow
    note: 'Spawned from 0393 deep review (finding #9). Low: issuer-of-own-asset niche.'
  - date: 2026-09-06
    status: superseded
    who: karolkow
    by: ['0540']
    note: >
      Superseded by 0540. The `net_settled` column was removed on 2026-09-04; the issuer-side claimable-balance and pool understatement is closed by 0540's reader fix — the claimable balance (`B…`) and the pool (`L…`) are holders now, and the real-corpus test that was named after this gap sees the `B…` balance receive its dSTARDUST.
---

# BUG: net-settled understates issuer-side claimable-balance / LP of the issuer's own asset

## Summary

`ledger_balance_deltas` reads value only from `AccountEntry` (native) and
`TrustLineEntry` (credit) — every other entry type returns `None`
(`entry_balance`, `ledger_value.rs:116-130`). The net-settled formula rests on
"at least one leg is an Account/Trustline." That holds for ordinary transfers,
but breaks when a credit asset's **issuer** is a party (issuers have no trustline
— they mint/burn implicitly) **and** the counterparty is a non-account entry the
reader skips (ClaimableBalance / LiquidityPool).

## Context

Verified niche (deep review): non-issuer claimable-balance create/claim and
ordinary LP deposits ARE captured via the counterparty's Account/Trustline delta;
`PoolShare → None` is correct (shares are not an asset amount). Only the
issuer-of-own-asset case slips through:

- Issuer `CreateClaimableBalance` of its own USDC (100): the 100 is minted into
  the `ClaimableBalanceEntry`; the issuer has no USDC trustline decrement, and the
  CB entry is skipped → USDC `net_settled = 0`/dash on the create tx (the value
  only surfaces later on the claimant's claim tx).
- Issuer `LiquidityPoolDeposit` of its own asset → the amount lands in the
  `LiquidityPoolEntry` reserve (skipped), no issuer trustline decrement → that
  asset's value understated.

## Implementation

- Extend `entry_balance` to emit **virtual** participant balances for
  `ClaimableBalanceEntry.amount` and `LiquidityPoolEntry` reserves, feeding the
  same before/after delta + netting flow. Guard against double counting the
  counterparty legs the current path already captures.

## Acceptance Criteria

- [ ] Issuer-side CB create / LP deposit of the issuer's own asset surfaces the
      correct net-settled value.
- [ ] No double count for the common (non-issuer) CB/LP cases already captured via
      the counterparty Account/Trustline delta.
- [ ] `PoolShare` still excluded (shares are not an asset amount).
