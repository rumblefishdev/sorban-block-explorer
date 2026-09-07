# NFT interpretation policy (2026-09-07)

Scope: thread 104 step 1 / thread 99. Research and semantic decision, not an
implemented decoder or permission to deploy. Code baseline: 30753600.
Thread 103: the user selected a joint develop-to-master release; audit the
whole release range and combine migration prerequisites before tagging.

## Decision

An event carries a declared quantity, an individual token identity, or a
payload whose meaning we have not resolved. These are different outcomes.
An integer's signedness alone does not prove the business meaning of an
arbitrary contract's event. Preserve the source event and its identity; do
not turn unresolved values into amounts, zeroes, or confirmed NFTs.

For Soroban NFTs the identity is (network, emitting collection contract,
typed token ID), not just the collection and not just the number. Preserve
the ID's original type/value. Amount is absent; a separate count of token
movements, if requested later, must never sum token IDs. A transfer from an
account to itself has zero net ownership-count effect. Approvals are not
movements. Batch ranges must retain all identities, not use the range's
upper bound as either an amount or a single token ID.

Classic assets used as NFTs are different: their asset identity is already
the classic asset, and their protocol-defined transferred quantity remains
an amount. Do not null SAC amounts merely because someone markets the asset
as an NFT. No pricing or financial valuation is involved in this policy.

## Evidence and precedence

1. Protocol-owned SAC events: authenticate the asset through the existing
   network-dependent SAC derivation and decode the protocol's amount shape.
2. Bespoke contracts: match an event-specific schema from the executing WASM
   version, or an explicitly supported legacy decoder tied to evidence for
   that implementation/version. Use the decoded parameter's semantic role,
   not a contract-wide name-only label. A declared amount is an event amount,
   not a independently proven balance delta.
3. Missing or conflicting semantic context: unresolved. Scalar i128 can be
   a quantity or a legacy NFT ID; unsigned scalars are NFT-compatible, not
   universal proof of NFT semantics. Explicit amount/token_id map keys are
   useful format evidence but are still authored by the contract. With no
   disambiguating schema a map carrying both remains unresolved.

Do not apply today's contract_type or WASM hash backwards through history.
Live and replay must use identical immutable evidence for an event, or both
return unresolved. If the executing version cannot be established (including
an upgrade during execution), do not guess from the final ledger state.
This policy does not require building a universal historical resolver before
ingestion: unresolved is a legitimate outcome, with visible coverage limits.

## Primary sources checked

- [SEP-41](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md),
  Events: standalone transfer quantities are i128; map format uses amount.
  Additional map fields are allowed. Therefore amount + token_id is not
  intrinsically malformed or necessarily an NFT. This corrects the earlier
  regression note's overly broad description of contradictory maps.
- [SEP-50 draft](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md):
  TokenID is an unsigned integer without a fixed width; NFT transfers identify
  a token, and approvals are separate operations. This is not a universal
  retrospective requirement for all deployed legacy contracts.
- [OpenZeppelin implementation](https://github.com/OpenZeppelin/stellar-contracts/blob/main/packages/tokens/src/non_fungible/mod.rs):
  Transfer and Mint event structs declare token_id: u32. Decoding this as a
  quantity would contradict the implementation's declared meaning.
- [SEP-48 event specifications](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0048.md#sc_spec_entry_event_v0):
  Event specs describe prefix topics, named typed parameters, their locations,
  and single-value/vec/map encoding. Reuse official ScSpecEntry XDR types to
  read them; do not invent a parallel ABI. Specs describe what the contract
  declares, not proof that its storage changed honestly.

No production counts or new live-chain observations were made for this note.
The legacy mainnet i128 example is previously recorded evidence in nft.rs,
not freshly verified here. The impossibility of universally distinguishing
identical payloads does not depend on its prevalence.

## What the current code actually does

- asset_transfers.rs::token_event_amount treats every scalar i128 as fungible
  and every valid unsigned scalar as non-fungible. It lacks semantic context.
- classification.rs::classify_contract_from_wasm_spec checks function NAMES
  only. Adding owner_of to a fungible implementation changes its verdict to
  Nft; adding decimals can label an otherwise unknown contract Fungible.
  This is classification evidence, not an authority for individual events.
- contract.rs::parse_spec_entries uses official ScSpecEntry but currently
  retains only FunctionV0. Event specifications are not yet consumed.
- nft.rs supports packed arguments, token_id maps and consecutive_mint;
  asset_transfers.rs does not cover that same set of NFT shapes.
- AssetTransferRow has neither a token ID nor an explicit unresolved state.
  Its current contract says NULL means NFT only. Do not silently overload it
  to also mean unknown without changing that contract and its consumers.
- The existing NFT writer uses classification/quarantine; targeted backfill
  does not load prior_contract_verdicts. Plugging that path directly into the
  new extractor would not establish historical/live equivalence.

## Examples (synthetic)

| Event context                            | Payload                   | Interpretation                            |
| ---------------------------------------- | ------------------------- | ----------------------------------------- |
| Proven SAC, matching token event         | i128(42)                  | quantity 42 base units                    |
| Matching NFT schema                      | token_id: u32(42)         | collection NFT #42, no amount             |
| Evidence-backed legacy NFT decoder       | i128(42)                  | collection NFT #42, no amount             |
| Unknown bespoke emitter                  | i128(42)                  | unresolved, never summed                  |
| Recognised FT schema allowing extra keys | {amount: 7, token_id: 42} | declared quantity 7; preserve extra field |
| No disambiguating schema                 | {amount: 7, token_id: 42} | unresolved, not automatically malformed   |

A malicious bespoke contract may emit the exact same schema and payload as
an honest contract without changing ownership. The index records that
contract's declaration, not a verified balance. It cannot acquire a genuine
classic asset's identity through a forged topic because the SAC gate remains.

## Consequences before full value-flow backfill

The interpretation is settled; implementation is not. A minimal follow-up
must represent unresolved semantics explicitly and exclude them from amount
aggregation without losing the source event. A small interpretation-state
field on the existing projection is preferable to a new table; final DDL and
storage measurement belong to implementation, not this research step.
Token identity may initially be recovered from the existing source-event
reference; no extra token-ID column is required merely to display transfers.
Do not advertise indexed per-NFT history until its lookup path is implemented.

Reuse the existing NFT shape readers after checking their operand rules;
do not blindly reuse their name-only classifier or duplicate the entire
decoder. Unsupported batch shapes remain explicitly uncovered until tested;
one-event/one-row projection must not expand a batch into colliding keys.

Required tests: same i128 with FT/NFT/missing context; unsigned NFT IDs;
amount + extra token_id under known FT and unknown schemas; packed and batch
NFT shapes; same ID in two collections; approvals; self-transfer; executable
upgrade or missing historical version; identical live/replay inputs including
missing evidence. These tests are not claimed run or implemented here.

Keep the full asset_transfers backfill on hold until this narrow semantic
contract and tests land. This does not gate unrelated memo/event-position
tables, nor require completing all of task 0542. No production action or
commit is authorised by completion of this research step alone.

## Measured exposure and decision (task owner, 2026-09-07, thread 114 A)

The one payload no parser can disambiguate — a bespoke NFT whose token id is
an `i128` — was measured on production before deciding whether it gates the
backfill. Unlabelled (bespoke) token events with a scalar `i128` payload,
joined to the emitter's classifier verdict in `soroban_contracts`:

| Window                | events  | emitters | verdict of every emitter |
| --------------------- | ------- | -------- | ------------------------ |
| 56 000 000–56 499 999 | 6 877   | 118      | fungible (3)             |
| 64 000 000–64 499 999 | 137 820 | 145      | fungible (3)             |

Zero from NFT-classified (2), other (1) or unclassified contracts. The one
collection `nft.rs` cites as using `i128` ids emitted a single `Mint` with a
`vec` payload over its whole life, which the decoder rejects and counts. The
exposure is also bounded by construction: a bespoke token's `asset_id` is its
own contract, so a misread id can only inflate that collection's own row on
an account page — never XLM or a classic asset.

**Decision: the ambiguity does not gate the backfill.** No interpretation-state
column on `asset_transfers`: an unresolved payload is already preserved in
`soroban_events` and counted per cause; the table need not carry it. The
semantic resolution (event specs per SEP-48, or an evidence-backed legacy
decoder, applied identically to live and replay) is task 0542 step 6; marking
rows of NFT-classified emitters on the account page is task 0543 step 2.
