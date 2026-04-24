---
id: '0165'
title: 'API: filter synthetic sentinel accounts from public endpoints'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0048', '0049', '0053', '0160', '0161']
tags: [priority-medium, effort-small, layer-api, layer-backend, audit-gap]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/db/migrations/0002_identity_and_ledgers.sql
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0160 post-completion audit. Task 0160 seeded an
      all-zero-Ed25519 StrKey
      (`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF`,
      `xdr_parser::XLM_SAC_ISSUER_SENTINEL`) into `accounts` so
      `assets.issuer_id` FK resolves for XLM-SAC rows. The sentinel
      is a synthetic marker, not a real Stellar account — but nothing
      filters it from public-facing queries today. If an external
      tx touches that address (legitimate or prank), UI / search /
      transaction history would conflate it with the XLM-SAC
      identity.
---

# API: filter synthetic sentinel accounts from public endpoints

## Summary

Exclude synthetic account StrKeys (today just the XLM-SAC issuer
sentinel, future: any other placeholder) from:

- `GET /accounts` list pagination
- `GET /accounts/:id` — return 404 for synthetic StrKeys, or a
  dedicated "synthetic marker" response indicating the account is an
  indexer artifact
- `GET /search?q=…` — exclude from results
- Transaction / operation endpoints must still surface txs touching
  this StrKey (for data completeness) but label the counterparty as
  "native XLM-SAC issuer marker" in the JSON response.

## Context

Task 0160 introduced option (c) for XLM-SAC identity: synthesise
`asset_code = "XLM"` + issuer sentinel (all-zero Ed25519 StrKey).
The sentinel satisfies `ck_assets_identity` + FK integrity. But
it surfaces as a regular `accounts` row:

- `first_seen_ledger = 0`, `last_seen_ledger = 0`,
  `sequence_number = 0`, `home_domain = NULL` at seed time.
- `upsert_accounts` will update `last_seen_ledger` via
  `GREATEST(accounts.last_seen_ledger, EXCLUDED.last_seen_ledger)`
  if any ExtractedAsset path emits it (current code routes it via
  `staging.rs:365-368`).
- Technically anyone can send a payment on-chain to `GAAAA…WHF`
  (its pubkey is valid StrKey; nobody has the private key for
  signing but payments can be received). Such txs would appear in
  transaction_participants / operations touching this account.

Without filtering, the backend API (0049, 0048, 0053) will return
this artifact in listings and search, confusing users.

## Implementation

1. Expose the sentinel constant to API crate (or define a shared
   `SYNTHETIC_ACCOUNTS: &[&str]` list in `domain` crate — 0161's
   native asset singleton won't touch accounts but future synthetic
   markers might).
2. Backend module `accounts` (0048):
   - `list_accounts` WHERE `account_id <> ALL($synthetics)`
   - `get_account(id)` returns 404 (or a dedicated
     `"synthetic": true, "purpose": "xlm_sac_issuer_marker"`
     response — decide at implementation)
3. Backend module `search` (0053):
   - exclude sentinel StrKey from `/search?q=` results
4. Backend module `assets` (0049):
   - when rendering an SAC row with `issuer.account_id =
XLM_SAC_ISSUER_SENTINEL`, annotate as "native XLM" in the
     response instead of rendering the raw StrKey — UI shouldn't
     ever show the sentinel verbatim.
5. Transaction / operation endpoints (0046, 0070, 0071):
   - still return participant / operation data referencing the
     sentinel (correctness over hiding), but add an enum flag
     `"counterparty_kind": "synthetic"` so UI can render
     accordingly.

## Acceptance Criteria

- [ ] Shared `SYNTHETIC_ACCOUNTS` constant (Rust) accessible from
      all API modules.
- [ ] `/accounts` list excludes sentinel entries.
- [ ] `/accounts/:id` with sentinel StrKey returns documented
      non-generic response (404 or synthetic marker shape).
- [ ] `/search` excludes synthetic StrKeys from results.
- [ ] `/assets/:id` for SAC with sentinel issuer renders issuer
      metadata with "native XLM" label, not the raw StrKey.
- [ ] Transaction / operation endpoints tag counterparties that
      match synthetic StrKeys with a distinguishable kind.
- [ ] Integration test: query `/search?q=GAAAA` — sentinel excluded.
- [ ] Integration test: query `/assets/<xlm-sac-contract>` — issuer
      rendered as "native XLM" label, not bare sentinel.

## Notes

Scope includes ONLY API layer. Indexer persist path (0160) keeps
seeding the sentinel row unchanged; removing it would break
`assets.issuer_id` FK. The filtering is a presentation concern.

**Related security note:** nobody has the private key for all-zero
Ed25519. Theoretical attack vector (someone deliberately sending
payment to the sentinel to pollute its activity) is low-risk,
harmless beyond UX confusion. Filtering mitigates the UX side; a
future hardening task could add a CHECK constraint or immutable
flag on the sentinel row itself.
