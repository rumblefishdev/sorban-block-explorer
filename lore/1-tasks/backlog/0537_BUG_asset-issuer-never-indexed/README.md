---
id: '0537'
title: 'BUG: an asset issuer that never transacts is never indexed — and the API fabricates an identity for it'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0411', '0419', '0393']
tags:
  [
    'indexer',
    'api',
    'clickhouse',
    'data-completeness',
    'phase-future',
    'effort-medium',
    'priority-medium',
  ]
links:
  - crates/db-clickhouse/src/persist/stage.rs
  - crates/api/src/common/ch.rs
history:
  - date: 2026-09-03
    status: backlog
    who: karolkow
    note: >
      Found while reviewing the net-settled read restored in 0411. Two defects
      that compound: the indexer discards an asset's issuer address although it
      holds it at parse time, and the API then fabricates `CODE-` out of the
      missing half instead of admitting it does not know. 463 issuers are
      unresolvable on prod today, affecting 531 assets. Currently DORMANT — zero
      value-carrying rows and zero SAC labels are affected — but the 0419
      backfill will wake it, because pre-July history contains assets whose
      issuers were active then and silent now. Must land BEFORE that backfill.
---

# BUG: asset issuers that never transact are never indexed

## Summary

An asset carries its issuer's address in the ledger. We hash that address into
`issuer_id`, store the hash, and **throw the address away**. The `accounts`
table is populated only from accounts that took part in a transaction, so an
issuer who minted an asset and then went quiet never lands there — and the
address is unrecoverable, because the hash is one-way.

The API then compounds it: with the issuer missing it builds `format!("{code}-{issuer}")`
anyway, producing `"USDC-"` — a string that looks like a valid asset identity,
passes no validation, and links nowhere.

## Measured on production (2026-09-03)

|                                                                      |                            |
| -------------------------------------------------------------------- | -------------------------- |
| Classic assets whose issuer is unresolvable                          | **531** of 447,156 (0.12%) |
| Distinct unresolvable issuers                                        | **463**                    |
| Those issuers appearing anywhere else (participants, op source/dest) | **0**                      |
| **Value-carrying rows affected, whole indexed range**                | **0**                      |
| **SAC contracts losing their asset label**                           | **0**                      |

**The defect is real but dormant.** Those assets are dead — nobody moves them —
so nothing reaches a user today. That is a property of the current data, not of
the code.

## Why it will stop being dormant

The 0419 backfill re-ingests history from before 2026-07-29. That history
contains assets whose issuers were **active then and silent since**. Their
value rows will be written, the issuer will still be missing, and the API will
serve `CODE-` identities to real users. Fixing this after the backfill means
backfilling twice.

## Root cause

`stage.rs` has the address in hand:

```rust
let issuer_id = t.issuer_address.as_deref().map(ids::account_id).unwrap_or(0);
```

`ids::account_id` is `hash64(strkey)` — one-way. The `AccountRow` batch is
built from `account_keys`, i.e. accounts that appear in the ledger's
transactions. An issuer is not one of them, so the address is never written and
cannot be recovered later from any table: every schema column is `issuer_id
Int64`; the only `String` account address in the database is
`accounts.account_id`.

## The API side is the odd one out

Every other module refuses to build an identity from a missing half:

```rust
// contracts/queries.rs — skips the row rather than inventing it
let issuer = issuers.get(&r.issuer_id).filter(|s| !s.is_empty())?;
if r.asset_code.is_empty() { return None; }
```

```rust
// liquidity_pools/queries.rs — keeps the Option, never flattens to ""
let issuer_a = accounts.get(&r.asset_a_issuer_id).map(String::as_str);
```

The value read restored in 0411 is the only place that uses
`unwrap_or_default()` on a resolved account. Note that `contracts` even guards
with `.filter(|s| !s.is_empty())` — somebody has been bitten by this before.

Two further smells in the same block, worth fixing together:

- `contract_strkey.unwrap_or_default()` for bespoke Soroban tokens yields an
  empty asset identity, so the link points at `/assets/` with no id;
- `let Ok(net_settled) = … else { continue }` drops a row **silently** — no
  log, no counter, the value simply disappears.
- the issuer batch is collected for `asset_type != 0`, which includes type 3;
  that branch never uses an issuer, so those ids are fetched for nothing.

## Fix, in dependency order

1. **Indexer (the cause).** Add each asset's `issuer_address` to the account
   batch, so an issuer is recorded the first time its asset is seen. No new
   data source needed — the address is already parsed.
2. **API (the symptom).** Stop fabricating: skip and `tracing::warn!` when the
   issuer or the contract StrKey is missing, matching `contracts/queries.rs`.
   Narrow the issuer batch to `asset_type == 1`.
3. **Backfill.** Ride the 0419 re-ingest — recomputing values already reads the
   ledgers that carry these addresses, so no separate pass is needed.
4. **Re-measure.** The five figures above should go to zero. If they do not,
   there is a second source of unresolvable issuers.

## Deliberately out of scope

Making `asset: Option<String>` on the wire, so a value with an unknown issuer
could render as an amount without a link rather than being dropped. That is a
breaking change to the 0411 response contract and belongs in its own task, if
the dropped-row behaviour ever proves visible.

## Acceptance Criteria

- [ ] Indexer records an asset's issuer address when the asset is first seen
- [ ] API skips rather than fabricates, and logs when it does — no `unwrap_or_default()`
      on a resolved account anywhere in `common/ch.rs`
- [ ] Silent `continue` on an unparsable value replaced with a warn
- [ ] Issuer batch narrowed to classic credit assets
- [ ] Lands **before** the 0419 backfill runs
- [ ] Post-backfill: unresolvable issuers re-measured and down to zero, or the
      remainder explained
