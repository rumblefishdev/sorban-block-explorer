---
url: 'https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md'
title: 'Soroban Token Interface (SEP-0041)'
fetched_date: 2026-04-14
task_id: '0118'
image_count: 0
---

# Soroban Token Interface (SEP-0041)

## Preamble

```
SEP: 0041
Title: Soroban Token Interface
Authors: Jonathan Jove <@jonjove>, Siddharth Suresh <@sisuresh>, Simon Chow <@chowbao>, Leigh McCulloch <@leighmcculloch>
Status: Draft
Created: 2023-09-22
Updated: 2025-08-28
Version 0.4.1
Discussion: https://discord.com/channels/897514728459468821/1159937045322547250, https://github.com/stellar/stellar-protocol/discussions/1584
```

## Simple Summary

This proposal establishes a standardized contract interface for tokens, representing "a subset of the Stellar Asset contract, and compatible with its descriptive and token interfaces defined in CAP-46-6."

## Motivation

Fungible assets represent core blockchain functionality. The ecosystem requires "an interface that is less opinionated than the interface of the Stellar Asset contract" to enable token implementations supporting standard functionality without requiring specialized Stellar asset behaviors. Additionally, developers need compatibility ensuring that tokens implementing this interface "would be largely indistinguishable" from Stellar Asset contracts.

## Specification

### Interface

```rust
pub trait TokenInterface {
    /// Returns the allowance for `spender` to transfer from `from`.
    ///
    /// # Arguments
    ///
    /// - `from` - The address holding the balance of tokens to be drawn from.
    /// - `spender` - The address spending the tokens held by `from`.
    fn allowance(env: Env, from: Address, spender: Address) -> i128;

    /// Set the allowance by `amount` for `spender` to transfer/burn from
    /// `from`.
    ///
    /// # Arguments
    ///
    /// - `from` - The address holding the balance of tokens to be drawn from.
    /// - `spender` - The address being authorized to spend the tokens held by
    /// `from`.
    /// - `amount` - The tokens to be made available to `spender`.
    /// - `live_until_ledger` - The ledger number where this allowance expires.
    /// Cannot be less than the current ledger number unless the amount is being
    /// set to 0.  An expired entry (where live_until_ledger < the current
    /// ledger number) should be treated as a 0 amount allowance.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["approve", from: Address,
    /// spender: Address], data = [amount: i128, live_until_ledger: u32]`
    ///
    /// Emits an event with:
    /// - topics - `["approve", from: Address, spender: Address]`
    /// - data - `[amount: i128, live_until_ledger: u32]`
    fn approve(env: Env, from: Address, spender: Address, amount: i128, live_until_ledger: u32);

    /// Returns the balance of `id`.
    ///
    /// # Arguments
    ///
    /// - `id` - The address for which a balance is being queried. If the
    /// address has no existing balance, returns 0.
    fn balance(env: Env, id: Address) -> i128;

    /// Transfer `amount` from `from` to `to`.
    ///
    /// # Arguments
    ///
    /// - `from` - The address holding the balance of tokens which will be
    /// withdrawn from.
    /// - `to` - The address which will receive the transferred tokens. A MuxedAddress or Address.
    /// - `amount` - The amount of tokens to be transferred.
    ///
    /// # Events
    ///
    /// Emits an event with:
    /// - topics - `["transfer", from: Address, to: Address]`
    /// - data - `amount: i128` or `{ amount: i128, to_muxed_id: Option<u64 | String | BytesN<32>> }`
    /// If the transfer involves a muxed address the address and muxed details
    /// are separated in the event.
    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128);

    /// Transfer `amount` from `from` to `to`, consuming the allowance of
    /// `spender`. Authorized by spender (`spender.require_auth()`).
    ///
    /// # Arguments
    ///
    /// - `spender` - The address authorizing the transfer, and having its
    /// allowance consumed during the transfer.
    /// - `from` - The address holding the balance of tokens which will be
    /// withdrawn from.
    /// - `to` - The address which will receive the transferred tokens.
    /// - `amount` - The amount of tokens to be transferred.
    ///
    /// # Events
    ///
    /// Emits an event with:
    /// - topics - `["transfer", from: Address, to: Address]`
    /// - data - `amount: i128`
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128);

    /// Burn `amount` from `from`.
    ///
    /// # Arguments
    ///
    /// - `from` - The address holding the balance of tokens which will be
    /// burned from.
    /// - `amount` - The amount of tokens to be burned.
    ///
    /// # Events
    ///
    /// Emits an event with:
    /// - topics - `["burn", from: Address]`
    /// - data - `amount: i128`
    fn burn(env: Env, from: Address, amount: i128);

    /// Burn `amount` from `from`, consuming the allowance of `spender`.
    ///
    /// # Arguments
    ///
    /// - `spender` - The address authorizing the burn, and having its allowance
    /// consumed during the burn.
    /// - `from` - The address holding the balance of tokens which will be
    /// burned from.
    /// - `amount` - The amount of tokens to be burned.
    ///
    /// # Events
    ///
    /// Emits an event with:
    /// - topics - `["burn", from: Address]`
    /// - data - `amount: i128`
    fn burn_from(env: Env, spender: Address, from: Address, amount: i128);

    /// Returns the number of decimals used to represent amounts of this token.
    fn decimals(env: Env) -> u32;

    /// Returns the name for this token.
    fn name(env: Env) -> String;

    /// Returns the symbol for this token.
    fn symbol(env: Env) -> String;
}
```

### Events

#### Approve Event

The `approve` event activates when allowance authorization occurs.

**Topics:**

- `Symbol` with value `"approve"`
- `Address` the source account holding transferable tokens
- `Address` the authorized spender

**Data:**

- `i128` permitted spending amount
- `u32` expiration ledger number

#### Transfer Event

The `transfer` event activates during token transfers between addresses.

**Topics:**

- `Symbol` with value `"transfer"`
- `Address` source of withdrawn tokens
- `Address` recipient of transferred tokens

**Data:**

- `i128` transferred amount
- or `map` structure containing:
  - `amount: i128` transferred quantity
  - `to_muxed_id: Option<u64 | String | BytesN<32>>` muxed identifier (absent if none)
  - Additional implementation-defined entries permitted

#### Burn Event

The `burn` event activates when tokens undergo destruction from an address.

**Topics:**

- `Symbol` with value `"burn"`
- `Address` source account losing tokens

**Data:**

- `i128` destroyed amount

#### Mint Event

The `mint` event activates when token creation increases supply and recipient balances.

**Topics:**

- `Symbol` with value `"mint"`
- `Address` recipient account receiving newly created tokens

**Data:**

- `i128` created amount
- or `map` structure containing:
  - `amount: i128` created quantity
  - `to_muxed_id: Option<u64 | String | BytesN<32>>` muxed identifier (absent if none)
  - Additional implementation-defined entries permitted

#### Clawback Event

The `clawback` event activates during forcible token recovery reducing both holder balances and total supply.

**Topics:**

- `Symbol` with value `"clawback"`
- `Address` account losing recovered tokens

**Data:**

- `i128` recovered amount

### Mint and Clawback Event Flexibility

The specification deliberately excludes `mint()`, `init_asset()`, and `clawback()` functions "to give contracts more flexibility to implement and name these functions as they see fit." Implementations retain freedom defining mint and clawback initiation mechanisms, requiring only that "a mint action that emits a mint event must increase total supply, and a clawback action that emits a clawback event must reduce total supply."

## Changelog

- `v0.1.0` - Initial draft based on CAP-46-6
- `v0.2.0` - Remove `spendable_balance`
- `v0.3.0` - Add `mint` and `clawback` events
- `v0.4.0` - Add muxed support to transfer and mint
- `v0.4.1` - Clarified clawback behavior and event emission rules

## Implementations

- The Rust soroban-sdk provides interface definitions and generated client functionality for contract development
- The Soroban Env offers native interface implementation as a presentation layer for Stellar Assets
