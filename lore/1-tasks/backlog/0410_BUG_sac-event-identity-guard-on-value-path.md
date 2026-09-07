---
id: '0410'
title: 'BUG: SAC event-identity guard on value path — verify emitter == asset SAC (full H2 fix)'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0393']
tags:
  [
    'clickhouse',
    'indexer',
    'xdr-parser',
    'security',
    'phase-future',
    'effort-medium',
    'priority-high',
  ]
links: []
history:
  - date: 2026-07-18
    status: backlog
    who: karolkow
    note: 'Spawned from 0393 future work (H2 interim gate). Code/docs/PR #355 already reference this task id by number.'
  - date: 2026-07-20
    status: backlog
    who: karolkow
    note: 'Deep review: the deterministic guard already exists in sac.rs (derive_sac_contract_id + sac_override_from_event_topics, unit-proven, used by the NFT path). Scope is WIRING not build. Added hybrid (ledger_balance_deltas for account-side legs) + net_id mismatch alarm.'
  - date: 2026-07-21
    status: backlog
    who: karolkow
    note: >
      RE-OPENED. The 2026-07-20 supersede was half right: value no longer reads
      events, but PRESENCE still maps an event-supplied SEP-11 asset string onto
      the real classic surrogate with no emitter binding, so forged rows can be
      injected into a real asset's transaction list. Re-scoped from the value path
      to the presence path; same gate, different call site. Also adds a fix for a
      topic-index mismatch that would let a 5-topic event bypass the gate.
      Priority raised — this is exploitable today (not currently exploited).
  - date: 2026-09-01
    status: backlog
    who: karolkow
    note: >
      RE-VERIFIED against develop, still open and still exploitable. Found
      INDEPENDENTLY by the PR #438 (task 0374) blast-radius investigation,
      which swept every entity producer for "identity taken from an
      attacker-controlled payload instead of the authenticated
      owner/emitter" — this is the ONLY pre-existing member of that class in
      the codebase. Both defects this task names are unchanged in code; the
      topic-index mismatch is confirmed still real. See "Re-verification"
      below for the fresh evidence and the narrowed blast radius.
---

# BUG: SAC event-identity guard on value path — verify emitter == asset SAC (full H2 fix)

> **⚠️ RE-OPENED + RE-SCOPED (2026-07-21). The 2026-07-20 "superseded" call was
> half right and closed this too early.**
>
> What that call got right: the **VALUE** path no longer reads events at all
> (task 0393 moved it to ledger balance deltas), so there is nothing left to gate
> there. That part is genuinely resolved.
>
> What it missed: **PRESENCE still derives classic-asset identity from an event
> string, with no emitter binding.** `event_asset` takes only the topic — it does
> not receive the emitter at all (`xdr-parser/src/event_filters.rs:116-130`) — and
> `event_asset_surrogate` maps the result straight onto the REAL classic surrogate:
>
> ```rust
> EventAsset::Credit { code, issuer } => Some(ids::credit_asset_id(code, issuer)),
> ```
>
> (`db-clickhouse/src/persist/stage.rs`; the `emitting_contract_id` argument is
> consulted only for `Bespoke`.) Those rows land in `operation_asset_appearances`,
> which is what the asset detail page's transaction list reads. **So any contract
> can inject transactions into the transaction list of a real classic asset**
> (USDC, EURC, …) by emitting one event carrying that asset's SEP-11 string.
>
> Scope is therefore unchanged in mechanism (wire the existing
> `sac_override_from_event_topics` gate) but **moved from the value path to the
> presence path**. Everything below about the guard still applies; ignore the
> references to `token_events_net_settled` and `policy_null`, which were deleted
> with the event-value path.

## Re-verification 2026-09-01 — still open, independently rediscovered

Re-checked against `develop` while auditing PR #438 (task 0374). That review
ran a codebase-wide sweep for one bug class: **an entity's identity taken from
a payload the emitting contract chooses freely, instead of from the
authenticated owner/emitter, with no downstream check.** Three agents graded
every entity producer in `xdr-parser` and `db-clickhouse/persist`.

**Result worth knowing: this task is the ONLY pre-existing member of that
class.** Every other producer anchors identity to an authenticated source — a
ledger-entry owner, the event `contract_id`, the tx source, or a cryptographic
derivation corroborated against the emitter (`nft.rs`'s
`derived_sac == emitter` is the reference). So this is not one of many; it is
the last one.

Both defects named above are unchanged in today's code:

1. **Presence path still unguarded.** `event_asset_surrogate`
   (`stage.rs`) maps a `Credit` claim straight onto the real classic
   surrogate — `EventAsset::Credit { code, issuer } => Some(ids::credit_asset_id(code, issuer))`
   — with the emitter consulted only for `Bespoke`. `event_asset`
   (`event_filters.rs`) receives the topic alone and never the emitter.

2. **Topic-index mismatch still real** — and worth restating precisely,
   because the decoder has changed shape since this task was written. It no
   longer reads a hardcoded index: `parse_token_event` now picks a FIXED index
   PER VERB (`transfer` → 3, `mint`/`burn`/`clawback` → 2). The gate
   (`sac.rs`) still reads `topics.last()`. For the documented arities those
   coincide; for a longer topic list they do not, so the gate would inspect a
   different element than the one the decoder trusted. Aligning both on one
   index (plus the over-long-topics test) stays a MUST-FIX alongside the
   wiring.

**Blast radius, measured rather than assumed** — the reason this stayed off
the #438 critical path:

- **Presence only.** The value path has read authenticated ledger deltas since
  0393, so a forged event moves no amount, no balance, and no aggregate. What
  a forged row buys is a line in a real asset's transaction list.
- **Bespoke tokens fall out at read.** The reader INNER JOINs the `assets`
  dimension, so a contract with no asset row contributes nothing.
- Not introduced by #438, and not made worse by it.

## Summary

Task 0393 shipped an **interim** trust gate ("H2"): a Native/Credit asset
identity taken from a Soroban token event's trailing SEP-11 string topic
(`"USDC:GISSUER…"`) is **not cryptographically bound to the emitting contract**,
so any contract could forge a real-asset amount. Until this task, the value path
writes such claims as `NULL` (a dash) rather than attribute a spoofable figure.
This task closes the gate properly so genuine SAC transfers inside Soroban txs
surface their value again.

## Context

- Interim gate lives in `crates/db-clickhouse/src/persist/stage.rs`
  (`token_events_net_settled`, `~line 2279`): only a bespoke `EventAsset::Contract`
  identity — which IS the emitter — is attributed a value; Native/Credit event
  claims → `policy_null` → `NULL`.
- Cost of the interim: a genuine SAC transfer of a classic asset inside a Soroban
  tx loses its event-derived value (shows a dash). The classic-op path
  (`has_soroban = 0`) is unaffected.
- **The guard already exists — this is a WIRING task, not a build** (deep review,
  2026-07-20). `crates/xdr-parser/src/sac.rs` already implements the full
  deterministic binding end-to-end:
  - `derive_sac_contract_id` — `SHA256(XDR(HashIdPreimage::ContractId{ network_id,
ContractIdPreimage::Asset(asset) }))` → the canonical SAC C-StrKey (CAP-46-3).
    Unit-proven against published constants: XLM SAC `CAS3J7GY…`, USDC SAC
    `CCW67TSZ…`.
  - `sac_override_from_event_topics` — parses the trailing SEP-11 asset string,
    derives the SAC id, and gates on `if derived != emitter { return None }`.
    Already used by the live NFT path (`nft.rs`).
  - stellar-xdr 27 exposes the full preimage; `network_id` is already threaded to
    the ingest path (`process.rs`). No dependency bump, no schema change.

## Implementation

### MUST FIX ALONGSIDE — the gate and the decoder read different topics

Wiring the gate is not sufficient on its own. The two sides disagree on where the
asset string lives:

- gate: `sac.rs` reads `topics.last()`
- decoder: `event_filters.rs` reads `arr.get(3)` (index 3)

For a 4-topic `transfer` these are the same element, so the gate appears to work.
For a **5-topic** event they are different elements: the decoder still yields
`Credit{USDC,…}` while the gate inspects some other topic, fails to derive a
matching SAC, and returns `None` — i.e. the forged identity slips through the very
check meant to stop it. Align both on one index (and add a test with a 5-topic
event asserting the gate rejects it).

### Primary — wire the existing guard into the PRESENCE path:

- Thread the emitter **C-StrKey** (already in hand at the caller as
  `ev.contract_id`, currently only hashed to a surrogate) and `net_id` into
  `token_events_net_settled` (`stage.rs`).
- Replace the blanket H2 branch (`stage.rs:~2292`, `!matches!(…Contract)` →
  `policy_null`). For a `Native`/`Credit` event call
  `sac_override_from_event_topics(emitter_strkey, topics, net_id)`: `Some` (emitter
  == derived SAC) → build the `Movement` on the SEP-11 asset_id (`event_asset_surrogate`
  already keys it identically to the op-path presence rows); `None` → keep in
  `policy_null` (dash). Bespoke `Contract` path unchanged.
- Result: genuine SAC classic transfers inside Soroban txs regain their value;
  forgeries still dash. No new keying, no schema change.

Complementary — **hybrid with classic ledger deltas** (0393 deep review):

- For `has_soroban = 1` txs the account-side legs of a SAC classic transfer are
  already present as authoritative, **unspoofable** `AccountEntry`/`TrustLineEntry`
  deltas (`ledger_balance_deltas`), which `stage.rs` currently **computes then
  discards**. Prefer these for account-side SAC value (no trust gate needed); the
  SAC-address guard still covers the contract-side (`C…`) legs and bespoke tokens
  that the delta path structurally cannot see (`ContractData` → `entry_balance`
  returns `None`). Guard is primary; deltas are a complementary/authoritative
  cross-check, not a replacement.

Safety:

- **Alarm on the SAC-derivation mismatch rate.** A wrong network passphrase makes
  every `derived != emitter` → all SAC-in-Soroban values silently dash. Fail-closed
  but invisible — emit a metric/`warn!` so a misconfig or a real spoofing spike is
  visible rather than a silent field of dashes.
- Residual (inherent, out of scope): a **bespoke** token's amount is still
  attacker-chosen — the guard certifies the _classic_ identity, not a contract's
  self-reported amount.
- Backfill/history: value history comes from the full S3 re-ingest (0393); the
  guard runs inside `stage.rs`, so a re-ingest recomputes SAC-in-Soroban values.

## Acceptance Criteria

- [ ] `sac_override_from_event_topics` is wired into `token_events_net_settled`
      (emitter C-StrKey + `net_id` threaded through).
- [ ] A genuine SAC transfer inside a Soroban tx surfaces its net-settled value
      (no longer forced to `NULL`). **DONE** — test `genuine_sac_transfer_surfaces_its_value`.
- [x] A forged `"CODE:ISSUER"` topic from a non-SAC emitter still yields `NULL`
      (spoof stays closed) — proven by a mutation test
      (`forged_classic_claim_from_a_non_sac_emitter_is_null`).
- [x] Value and presence paths resolve the same SAC identity (no drift) — the
      value attaches on `ids::credit_asset_id(code, issuer)` / `NATIVE_ASSET_ID`,
      the exact keys `event_asset_surrogate` / the op path already use.
- [ ] ~~Account-side SAC legs cross-checked against `ledger_balance_deltas`
      (hybrid)~~ — **DEFERRED / not needed:** the guard values both account-side and
      contract-side SAC legs from the (crypto-verified) events, so the delta hybrid
      is redundant for correctness. Kept as an optional authoritative cross-check.
- [x] Mismatch-rate visibility — `sac_rejected` counter + `tracing::debug!` when a
      guard-active reduction rejects classic claims (spoof / net_id misconfig).
      Test `wrong_network_id_rejects_a_genuine_sac_transfer` proves fail-closed.
- [x] `stage.rs` "interim for 0410" comments updated; guard-off tests reworded.
      0393 task H2 section + architecture docs still to reflect "guard wired" (ADR 0032) — folded into the same PR.

## Implementation status

**Implemented in the `feat/0393` branch (this PR), 2026-07-20.** Wiring:
`xdr_parser::sac::net_id()` accessor (env `STELLAR_NETWORK_PASSPHRASE`, `None` →
conservative) → threaded through `tx_token_net_settled` → `token_events_net_settled`,
which calls `sac_override_from_event_topics(emitter, topics, net_id)` per classic
claim. 4 mutation tests (`sac_gate_tests`). Move this file to `archive/` on merge
(via `/lore-framework-tasks`). Only the optional delta-hybrid cross-check remains.
