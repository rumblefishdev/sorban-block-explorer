# NFT identifier interpreted as an amount (2026-09-07)

Confirmed on develop after PR #451: a per-op mint event from a bespoke
contract with `data = ScVal::U128(1_000_000_000)` produced a transfer with
`amount = Some(1_000_000_000)`. The regression test failed with that exact
value before the fix. A SEP-50 NFT mint identifies one token with that number;
the number is not a transferable quantity.

Sources:

- [SEP-41](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md): standalone amounts are i128.
- [SEP-50 (draft)](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md): TokenID is an unsigned integer without a fixed width; mint/transfer data carries that identifier.

The parser now accepts unsigned scalar IDs (u32/u64/u128/u256) as
non-fungible, preserving the movement with a NULL amount. It validates
token_id maps and rejects ambiguous maps containing both amount and token_id.
The measured map{amount: u128} extension stays supported, with checked
unsigned parsing and conversion into the signed storage range. Tests use
ScVal through the real typed-JSON encoder, including numeric JSON u32/u64.

Attack scenario (synthetic reproduction, not a claimed observed exploit):
an attacker mints an NFT with a large numeric ID to a victim address using
their own contract. Previously this polluted asset_transfers with that ID as
a quantity; an amount-summing reader could show a bogus balance change of
that contract's token. Now the same row has NULL amount. This does not move
ledger funds, bypass SAC identity validation, or impersonate genuine USDC.

## Remaining limitation: do not treat format as authentication

A custom NFT using i128 for its ID is indistinguishable from a SEP-41 amount
given only data. nft.rs already documents this ambiguity. This patch does not
claim universal NFT classification or protection against arbitrary false
events emitted by a bespoke contract. Existing contract classification is
not currently supplied consistently to live and backfill value-flow decoding;
blindly using today's database classification for historical events also
needs an upgrade/history policy. Resolve that in 0542 before promising NFT-safe
numeric aggregation for arbitrary contracts. A zero-contradiction oracle
sample is not evidence that all NFT event conventions are handled.
