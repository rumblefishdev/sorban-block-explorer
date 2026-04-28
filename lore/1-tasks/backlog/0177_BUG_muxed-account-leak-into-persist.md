---
id: '0177'
title: 'BUG: muxed transaction source leaks 69-char M-key into accounts.account_id VARCHAR(56)'
type: BUG
status: backlog
related_adr: ['0026', '0037']
related_tasks: ['0044', '0145', '0175']
tags:
  [priority-high, layer-parser, layer-persist, audit-driven, blocks-backfill]
links:
  - crates/xdr-parser/src/envelope.rs
history:
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Surfaced by task 0175 30k smoke backfill. Crashed at ledger
      62017877 (1878/30000 indexed) with PG error 22001
      "value too long for type character varying(56)". Root cause:
      `envelope::envelope_source` returns `MuxedAccount::to_string()`,
      which for `MuxedAccount::MuxedEd25519` emits a 69-char M-key
      instead of the underlying 56-char G-key. Per ADR 0026 +
      task 0044 the persisted account_id must always be the
      ed25519 public key (G-prefix, 56 chars).
---

# Muxed transaction source leaks 69-char M-key into persist write path

## Summary

[`crates/xdr-parser/src/envelope.rs:260`](../../../crates/xdr-parser/src/envelope.rs#L260):

```rust
InnerTxRef::V1(tx) => tx.source_account.to_string(),
```

`tx.source_account` is a `MuxedAccount` enum:

```
union MuxedAccount switch (CryptoKeyType type) {
  case KEY_TYPE_ED25519:        uint256 ed25519;
  case KEY_TYPE_MUXED_ED25519:  struct { uint64 id; uint256 ed25519; } med25519;
};
```

`stellar_xdr::curr::MuxedAccount::to_string()` emits the StrKey form
of whichever variant the value carries. For `Ed25519` that's a 56-char
`G…` StrKey; for `MuxedEd25519` it's a 69-char `M…` StrKey.

Our schema column [`accounts.account_id VARCHAR(56)`](../../../crates/db/migrations/0002_identity_and_ledgers.sql#L31)
rejects 69-char inputs with PG error `22001`, killing the indexer
mid-batch. Mainnet has continuous muxed-account traffic from major
exchanges using muxing as customer-deposit-id discriminator — every
multi-thousand-ledger range hits this within minutes.

## Reproduction

```bash
cargo run -p backfill-runner -- run --start 62016000 --end 62046000
# panics at ledger ~62017877 with:
#   PgDatabaseError code 22001: "value too long for type character varying(56)"
```

The 1878 ledgers indexed before the crash (62016000–62017877) contain
zero muxed sources by chance — the actual frequency is much higher
once the sample window widens.

## Root cause

ADR 0026 + the persist-layer surrogate-id rule require accounts to
be keyed by the ed25519 public key (G-prefix, 56 chars). The XDR
extraction layer should unwrap muxed → underlying ed25519 unconditionally
before the value reaches `accounts.account_id`.

The current `InnerTxRef::source_account()` accepts both variants and
emits whichever one was on the wire. For V0 envelopes it's correct
because `source_account_ed25519` is bare bytes (no muxing wrap on the
type), but for V1 it loses the discriminator and surfaces M-keys
intact.

## Fix

Add a single helper on `InnerTxRef` (or as a free function in `envelope.rs`)
that always returns the ed25519 G-strkey:

```rust
fn ed25519_strkey(m: &MuxedAccount) -> String {
    let pk = match m {
        MuxedAccount::Ed25519(p)        => p.0,
        MuxedAccount::MuxedEd25519(med) => med.ed25519.0,
    };
    stellar_strkey::ed25519::PublicKey(pk).to_string()
}
```

Then change [`envelope.rs:260`](../../../crates/xdr-parser/src/envelope.rs#L260):

```rust
InnerTxRef::V1(tx) => ed25519_strkey(&tx.source_account),
```

The same pattern needs to be applied wherever a `MuxedAccount` ends up
as a stored StrKey — likely several call sites in `operation.rs`
(operation source/destination), `sac.rs` (host-fn auth), and possibly
`invocation.rs`. Code review should grep for every `MuxedAccount::to_string()`
and `MuxedAccount` → String conversion in the parser and audit each.

## Acceptance Criteria

- [ ] `envelope_source` returns 56-char G-strkey for both `Ed25519`
      and `MuxedEd25519` `MuxedAccount` variants
- [ ] Unit test in `crates/xdr-parser/tests/` with a synthetic V1 envelope
      carrying a `MuxedEd25519` source — extracted source_account is
      56 chars, starts with `G`
- [ ] Audit every `MuxedAccount::to_string()` / `MuxedAccount` → String
      conversion site in `crates/xdr-parser/src/`; document each in
      the PR (kept as G-emission, or fixed to G-emission)
- [ ] Re-run 30k smoke backfill against the same range
      (`62016000`–`62046000`) — completes without 22001
- [ ] Re-run task 0175 audit-harness `--table accounts` and
      `--table transactions` — zero new mismatches introduced

## Notes

- **Blocks the 30k smoke** that's verifying audit harness Phase 1 +
  Phase 2a coverage on real data. Backfill is dead at the time of
  spawn; restart blocked on this fix.
- **Sub-account ID is lost.** Once we unwrap muxed → ed25519 at the
  parser, the 8-byte muxing ID disappears from our index. That's
  consistent with ADR 0026's choice to key on the underlying account
  and not surface mux IDs in the explorer; if a future endpoint needs
  the mux ID, it would have to come from re-parsed envelope XDR (per
  ADR 0029 read-time fetch).
- Related to but distinct from task 0044 path validators (which only
  validates StrKey shape on input — doesn't normalise muxed →
  ed25519 since path params are documented G-only).
