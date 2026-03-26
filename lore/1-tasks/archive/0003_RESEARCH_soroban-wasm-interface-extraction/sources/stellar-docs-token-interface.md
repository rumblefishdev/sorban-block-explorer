---
source_url: 'https://developers.stellar.org/docs/tokens/token-interface'
title: 'Token Interface'
fetched_date: 2026-03-26
task_id: '0003'
---

# Token Interface

> The phrase "custom token" has been retired in favor of "contract token".

Token contracts on Soroban, including the Stellar Asset Contract and example implementations, expose a common interface. While tokens can implement any interface, they should satisfy this standard interface to work with contracts supporting Soroban's built-in tokens.

The interface doesn't always require full implementation. For instance, a contract token may omit administrative functions compatible with the Stellar Asset Contract—it will still function in contracts performing standard user operations like transfers and balance checks.

## Compatibility Requirements

For each contract function, three requirements should align with the described interface:

- **Function interface** (name and arguments)—inconsistency prevents users from accessing the function entirely. This is mandatory.
- **Authorization**—users must authorize token function calls with all invocation arguments. Inconsistency may cause signature issues and confuse wallet software.
- **Events**—tokens must emit events in the specified format. Inconsistency can prevent proper handling by downstream systems like block explorers.

## Code

The interface below uses the Rust [soroban-sdk](https://developers.stellar.org/docs/tools/sdks/contract-sdks#soroban-rust-sdk) to declare a trait complying with the [SEP-41](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md) token interface.

```rust
pub trait TokenInterface {
    /// Returns the allowance for `spender` to transfer from `from`.
    ///
    /// The amount returned is the amount that spender is allowed to transfer
    /// out of from's balance. When the spender transfers amounts, the allowance
    /// will be reduced by the amount transferred.
    ///
    /// # Arguments
    ///
    /// * `from` - The address holding the balance of tokens to be drawn from.
    /// * `spender` - The address spending the tokens held by `from`.
    fn allowance(env: Env, from: Address, spender: Address) -> i128;

    /// Set the allowance by `amount` for `spender` to transfer/burn from
    /// `from`.
    ///
    /// The amount set is the amount that spender is approved to transfer out of
    /// from's balance. The spender will be allowed to transfer amounts, and
    /// when an amount is transferred the allowance will be reduced by the
    /// amount transferred.
    ///
    /// # Arguments
    ///
    /// * `from` - The address holding the balance of tokens to be drawn from.
    /// * `spender` - The address being authorized to spend the tokens held by
    ///   `from`.
    /// * `amount` - The tokens to be made available to `spender`.
    /// * `expiration_ledger` - The ledger number where this allowance expires. Cannot
    ///    be less than the current ledger number unless the amount is being set to 0.
    ///    An expired entry (where expiration_ledger < the current ledger number)
    ///    should be treated as a 0 amount allowance.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["approve", from: Address,
    /// spender: Address], data = [amount: i128, expiration_ledger: u32]`
    fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32);

    /// Returns the balance of `id`.
    ///
    /// # Arguments
    ///
    /// * `id` - The address for which a balance is being queried. If the
    ///   address has no existing balance, returns 0.
    fn balance(env: Env, id: Address) -> i128;

    /// Transfer `amount` from `from` to `to`.
    ///
    /// # Arguments
    ///
    /// * `from` - The address holding the balance of tokens which will be
    ///   withdrawn from.
    /// * `to` - The address which will receive the transferred tokens.
    /// * `amount` - The amount of tokens to be transferred.
    ///
    /// # Events
    ///
    /// Emits an event with:
    /// * topics `["transfer", from: Address, to: Address]`
    /// * data `{ to_muxed_id: Option<u64>, amount: i128 }: Map`
    ///
    /// Legacy implementations may emit an event with:
    /// * topics `["transfer", from: Address, to: Address]`
    /// * data `amount: i128`
    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128);

    /// Transfer `amount` from `from` to `to`, consuming the allowance that
    /// `spender` has on `from`'s balance. Authorized by spender
    /// (`spender.require_auth()`).
    ///
    /// The spender will be allowed to transfer the amount from from's balance
    /// if the amount is less than or equal to the allowance that the spender
    /// has on the from's balance. The spender's allowance on from's balance
    /// will be reduced by the amount.
    ///
    /// # Arguments
    ///
    /// * `spender` - The address authorizing the transfer, and having its
    ///   allowance consumed during the transfer.
    /// * `from` - The address holding the balance of tokens which will be
    ///   withdrawn from.
    /// * `to` - The address which will receive the transferred tokens.
    /// * `amount` - The amount of tokens to be transferred.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["transfer", from: Address, to: Address],
    /// data = amount: i128`
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128);

    /// Burn `amount` from `from`.
    ///
    /// Reduces from's balance by the amount, without transferring the balance
    /// to another holder's balance.
    ///
    /// # Arguments
    ///
    /// * `from` - The address holding the balance of tokens which will be
    ///   burned from.
    /// * `amount` - The amount of tokens to be burned.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["burn", from: Address], data = amount:
    /// i128`
    fn burn(env: Env, from: Address, amount: i128);

    /// Burn `amount` from `from`, consuming the allowance of `spender`.
    ///
    /// Reduces from's balance by the amount, without transferring the balance
    /// to another holder's balance.
    ///
    /// The spender will be allowed to burn the amount from from's balance, if
    /// the amount is less than or equal to the allowance that the spender has
    /// on the from's balance. The spender's allowance on from's balance will be
    /// reduced by the amount.
    ///
    /// # Arguments
    ///
    /// * `spender` - The address authorizing the burn, and having its allowance
    ///   consumed during the burn.
    /// * `from` - The address holding the balance of tokens which will be
    ///   burned from.
    /// * `amount` - The amount of tokens to be burned.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["burn", from: Address], data = amount:
    /// i128`
    fn burn_from(env: Env, spender: Address, from: Address, amount: i128);

    /// Returns the number of decimals used to represent amounts of this token.
    ///
    /// # Panics
    ///
    /// If the contract has not yet been initialized.
    fn decimals(env: Env) -> u32;

    /// Returns the name for this token.
    ///
    /// # Panics
    ///
    /// If the contract has not yet been initialized.
    fn name(env: Env) -> String;

    /// Returns the symbol for this token.
    ///
    /// # Panics
    ///
    /// If the contract has not yet been initialized.
    fn symbol(env: Env) -> String;
}
```

> **CAUTION WHEN MODIFYING ALLOWANCES:** The `approve` function overwrites the previous value with `amount`, so the previous allowance could be spent in an earlier transaction before the new `amount` is written in a later transaction. This allows the spender to spend more than intended. Avoid this by first setting the allowance to zero, verifying no spending occurred, then setting the new amount. See more details at <https://github.com/ethereum/EIPs/issues/20#issuecomment-263524729>.

## Metadata

Compliance with the token interface requires writing standard metadata (`decimal`, `name`, and `symbol`) in a specific format. This enables users to read constant data directly from the ledger without invoking a Wasm function. The [token example](https://github.com/stellar/soroban-examples/blob/main/token/src/metadata.rs) demonstrates using the Rust [soroban-token-sdk](https://github.com/stellar/rs-soroban-sdk/blob/main/soroban-token-sdk/src/lib.rs) to write metadata. Token implementations should follow this approach.

## Handling Failure Conditions

The token interface includes multiple scenarios where function calls can fail—insufficient authorization, inadequate allowance or balance, and others. Specifying expected behavior during such failures is important.

The interface incorporates authorization concepts matching Stellar Classic asset authorization and uses the Soroban authorization mechanism. When a token call fails, it may be due to either token authorization process. An `authorized` function returns true if an address has token authorization.

More details appear [here](https://developers.stellar.org/docs/learn/fundamentals/contract-development/authorization).

Functions in the token interface should use [trapping](https://doc.rust-lang.org/book/ch09-00-error-handling.html) as the standard failure-handling method, since the interface doesn't return error codes. When a function encounters an error, it halts execution and reverts any state changes from the function call.

## Failure Conditions

Expected behaviors for basic failure conditions in token interface functions:

### Admin functions

- If the admin didn't authorize the call, the function should trap.
- If the admin attempts an invalid action (like minting a negative amount), the function should trap.

### Token functions

- If the caller lacks authorization for the action (transferring tokens without proper authorization), the function should trap.
- If the action would create an invalid state (transferring more tokens than the available balance or allowance), the function should trap.

## Example: Handling Insufficient Allowance in `burn_from` function

The `burn_from` function should verify whether the spender has sufficient allowance to burn the specified tokens from the `from` address. With insufficient allowance, the function should trap, halting execution and reverting state changes.

Here's how the `burn_from` function can handle this failure condition:

```rust
fn burn_from(
    env: soroban_sdk::Env,
    spender: Address,
    from: Address,
    amount: i128,
) {
    // Check if the spender has enough allowance
    let current_allowance = allowance(env, from, spender);
    if current_allowance < amount {
        // Trap if the allowance is insufficient
        panic!("Insufficient allowance");
    }

    // Proceed with burning tokens
    // ...
}
```

By clearly outlining failure handling and incorporating appropriate error management in the token interface, token contracts become stronger and safer.
