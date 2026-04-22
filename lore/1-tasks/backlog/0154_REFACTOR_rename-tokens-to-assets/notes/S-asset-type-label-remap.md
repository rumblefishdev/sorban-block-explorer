---
title: 'Decision: `asset_type` label remap scope — `classic_credit` only'
type: synthesis
status: developing
spawned_from: R-assets-vs-tokens-taxonomy.md
spawns: []
tags: [schema, naming, enum, decision]
links: []
history:
  - date: '2026-04-22'
    status: developing
    who: stkrolikiewicz
    note: >
      Extracted from task 0154 README during review cleanup. Captures
      the decision to rename `classic` → `classic_credit` only, and to
      drop the speculative `soroban` → `soroban_sep41` rename proposed
      in the research note §7.3. Kept as a separate S- note so the
      task README stays focused on scope + acceptance criteria.
---

# Decision: `asset_type` label remap scope — `classic_credit` only

## Conclusion

When task 0154 renames the `tokens` table to `assets`, the
accompanying `asset_type` label remap is limited to
**`classic` → `classic_credit`**. All other values (`native`, `sac`,
`soroban`) stay unchanged.

This overrides the initial research-note draft
([R-assets-vs-tokens-taxonomy.md](R-assets-vs-tokens-taxonomy.md)
§7.3), which also proposed renaming `soroban` → `soroban_sep41`.

## Reasoning

### `classic` → `classic_credit` — kept

- Stellar XDR calls these `CREDIT_ALPHANUM4` / `CREDIT_ALPHANUM12`
  (see the `AssetType` enum in stellar-xdr, referenced by ADR 0031).
  "Classic credit" mirrors the protocol term exactly.
- Without the rename the word "classic" is ambiguous: XLM is also
  "classic" in the sense of being a classic-ledger asset, yet we
  label it `native` because that is the XDR variant. The only reason
  `classic` works today is because `ck_tokens_identity` disambiguates
  at CHECK-constraint level — not at label level. A reader skimming
  the table sees `native` and `classic` side by side and has to read
  the constraint to understand that they partition correctly.
- `classic_credit` removes that cognitive step.

### `soroban` → `soroban_sep41` — dropped

- The only reason given in the research note (§7.3) is "leaves space
  for `soroban_trex`". But §4.2 of the same note says the T-REX
  ecosystem on Stellar is nascent — we would be adding precision for
  an ecosystem that does not exist on Stellar mainnet yet.
- If T-REX (SEP-57) ever lands, its on-chain identity is still a
  `C...` SEP-41 contract plus compliance extensions. The more
  natural modelling would be a boolean `is_compliance` or a separate
  `compliance_flavour` column — not splitting `asset_type` into
  sibling labels.
- Until then, T-REX tokens would classify as `soroban` with no loss
  of fidelity. Renaming pre-emptively costs a DB migration and a
  label-breaking change for no user today.
- Additive: if T-REX materialises, a follow-up ADR can add a
  `soroban_trex` (or equivalent) value without renaming the existing
  `soroban`. That is the symmetric cost at the point where the
  distinction is actually needed.

## Net effect

The rename stays mechanical and low-risk. One label change, all
other values preserved. The migration remains reversible (up: one
`UPDATE`, down: one `UPDATE`); the Rust `TokenAssetType` enum (post-
ADR 0031) gets one variant renamed — `Classic` → `ClassicCredit` —
with matching updates to the `token_asset_type_name` SQL helper and
the integration test that iterates every variant.

## Alternatives considered

### Rename all four labels for uniform protocol alignment

**Why not:** `native` is already the XDR term, `sac` is already
unambiguous, and `soroban_sep41` is speculative (see above). Extra
churn for no analytical gain.

### Leave labels alone — rename only the table

**Why not:** The table rename surfaces that `classic` sits next to
`native` without protocol-level disambiguation. Fixing the label in
the same migration costs little extra and avoids a follow-up churn
window later.

### Introduce `compliance_flavour` column now

**Why not:** Premature — no T-REX-shaped data on Stellar mainnet yet.
Add the column (with an ADR) the first time a T-REX token needs
distinctive storage; until then the schema carries no information
worth splitting.
