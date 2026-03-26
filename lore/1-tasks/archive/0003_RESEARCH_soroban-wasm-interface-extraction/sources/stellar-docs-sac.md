---
source_url: 'https://developers.stellar.org/docs/tokens/stellar-asset-contract'
title: 'Use Issued Assets in Smart Contracts with the Stellar Asset Contract (SAC)'
fetched_date: 2026-03-26
task_id: '0003'
---

# Use Issued Assets in Smart Contracts with the Stellar Asset Contract (SAC)

> **Note:** The term "custom token" has been deprecated in favor of "contract token."

## Overview

The Stellar Asset Contract (SAC) implements [CAP-46-6 Smart Contract Standardized Asset](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md) and [SEP-41 Token Interface](https://developers.stellar.org/docs/tokens/token-interface) for Stellar assets.

The SAC enables users and contracts to make payments with and interact with assets. It's a special built-in contract with direct access to Stellar network functionality, allowing it to use Stellar assets directly.

Key characteristics:

- Each Stellar asset has a reserved SAC instance on the network
- When the SAC transfers assets between accounts, the same debits and credits occur as Stellar payment operations
- Transfers between contracts use Contract Data ledger entries to store balances
- Stellar account balances for native assets are stored on the account itself
- Stellar contract balances for native assets are stored in contract data entries
- Account balances for issued assets are stored in trust lines
- Contract balances for issued assets are stored in contract data entries

The SAC implements the SEP-41 Token Interface, similar to the widely-used ERC-20 standard. Contracts depending only on SEP-41 are compatible with any SEP-41-compliant contract token.

> "Some functionality available on the Stellar network in transaction operations, such as the order book, do not have any functions exposed on the Stellar Asset Contract in the current protocol."

## Deployment

Anyone can deploy a Stellar Asset Contract to its reserved address—the asset issuer need not be involved. Deployment options include:

- The [Stellar CLI](https://developers.stellar.org/docs/tools/cli/stellar-cli)
- The [Stellar SDK](https://developers.stellar.org/docs/tools/sdks) via `InvokeHostFunctionOp` with `HOST_FUNCTION_TYPE_CREATE_CONTRACT` and `CONTRACT_ID_FROM_ASSET`

The resulting token has a deterministic identifier: the sha256 hash of the `HashIDPreimage::ENVELOPE_TYPE_CONTRACT_ID_FROM_ASSET` XDR.

Initialization happens automatically during deployment. The asset issuer receives administrative permissions after deployment. The native XLM asset has no administrator and cannot be burned.

## Interacting with Classic Stellar Assets

The SAC is the only way for contracts to interact with Stellar assets—the native XLM or those issued by accounts.

### Using `Address::Account`

- The balance must exist in a trust line (or account for native balance)
- Classic trust line semantics apply
- Transfers succeed only if trust lines have the `AUTHORIZED_FLAG` set
- Clawback requires the `TRUSTLINE_CLAWBACK_ENABLED_FLAG`
- Transfers to the issuer account burn the token; transfers from the issuer account mint
- Trust line balances are stored as 64-bit signed integers, despite the interface accepting 128-bit integers

### Using `Address::Contract`

- Balances and authorization state are stored in contract storage
- Balances use 128-bit signed integers
- Clawback only works if the issuer account had `AUTH_CLAWBACK_ENABLED_FLAG` set when the balance was created
- A balance is created when an `Address::Contract` receives a successful transfer or when the admin sets authorization

### Balance Authorization Required

If the issuer has `AUTH_REQUIRED_FLAG` set, an `Address::Contract` must be explicitly authorized with `set_auth` before receiving a balance. This mirrors how trust lines interact with the `AUTH_REQUIRED_FLAG` issuer flag.

### Revoking Authorization

Admins can only revoke authorization if the issuer has `AUTH_REVOCABLE_FLAG` set. When a trust line is deauthorized from Soroban, the `AUTHORIZED_FLAG` is cleared and `AUTHORIZED_TO_MAINTAIN_LIABILITIES_FLAG` is set.

## Authorization Semantics

Token contract operations fall into three categories:

- **Getters** (e.g., `balance`): Require no authorization; do not change contract state
- **Unprivileged Mutators** (e.g., `incr_allow`, `xfer`): Require authorization from the address spending or allowing spending; exceptions include `xfer_from` and `burn_from`, which require authorization from the spender with prior allowance
- **Privileged Mutators** (e.g., `clawback`, `set_admin`): Require authorization from the administrator

## Contract Interface

The Rust SDK's [token module](https://docs.rs/soroban-sdk/latest/soroban%5Fsdk/token/index.html) provides two traits and client structs:

1. **TokenInterface** and **TokenClient**: Implement the common SEP-41 Token Interface with functions like `transfer`, `burn`, and `allowance`
2. **StellarAssetInterface** and **StellarAssetClient**: Extend SEP-41 with administrative functions like `set_admin`, `clawback`, and `set_authorized`

```rust
pub trait StellarAssetInterface {
    // SEP-41 Token Interface functions
    fn allowance(env: Env, from: Address, spender: Address) -> i128;
    fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32);
    fn balance(env: Env, id: Address) -> i128;
    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128);
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128);
    fn burn(env: Env, from: Address, amount: i128);
    fn burn_from(env: Env, spender: Address, from: Address, amount: i128);
    fn decimals(env: Env) -> u32;
    fn name(env: Env) -> String;
    fn symbol(env: Env) -> String;

    // SAC-specific administrative functions

    /// Sets the administrator to the specified address `new_admin`.
    ///
    /// # Arguments
    ///
    /// * `new_admin` - The address which will henceforth be the administrator
    ///   of this token contract.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["set_admin", admin: Address], data =
    /// [new_admin: Address]`
    fn set_admin(env: Env, new_admin: Address);

    /// Returns the admin of the contract.
    ///
    /// # Panics
    ///
    /// If the admin is not set.
    fn admin(env: Env) -> Address;

    /// Sets whether the account is authorized to use its balance. If
    /// `authorized` is true, `id` should be able to use its balance.
    ///
    /// # Arguments
    ///
    /// * `id` - The address being (de-)authorized.
    /// * `authorize` - Whether or not `id` can use its balance.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["set_authorized", id: Address], data =
    /// [authorize: bool]`
    fn set_authorized(env: Env, id: Address, authorize: bool);

    /// Returns true if `id` is authorized to use its balance.
    ///
    /// # Arguments
    ///
    /// * `id` - The address for which token authorization is being checked.
    fn authorized(env: Env, id: Address) -> bool;

    /// Mints `amount` to `to`.
    ///
    /// # Arguments
    ///
    /// * `to` - The address which will receive the minted tokens.
    /// * `amount` - The amount of tokens to be minted.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["mint", to: Address], data
    /// = amount: i128`
    fn mint(env: Env, to: Address, amount: i128);

    /// Clawback `amount` from `from` account. `amount` is burned in the
    /// clawback process.
    ///
    /// # Arguments
    ///
    /// * `from` - The address holding the balance from which the clawback will
    ///   take tokens.
    /// * `amount` - The amount of tokens to be clawed back.
    ///
    /// # Events
    ///
    /// Emits an event with topics `["clawback", admin: Address, to: Address],
    /// data = amount: i128`
    fn clawback(env: Env, from: Address, amount: i128);
}
```

## Contract Errors

All built-in smart contracts on Stellar share the same error types:

```rust
#[derive(Debug, FromPrimitive, PartialEq, Eq)]
pub(crate) enum ContractError {
    // Protocol implementation error (rare in real networks)
    InternalError = 1,
    // Operation not supported (e.g., clawback without clawback enabled)
    OperationNotSupportedError = 2,
    // SAC already initialized
    AlreadyInitializedError = 3,
    // Account missing on network
    AccountMissingError = 6,
    // Negative transfer amount
    NegativeAmountError = 8,
    // Insufficient allowance or expiration issue
    AllowanceError = 9,
    // Balance too low/high or clawback on non-clawback-enabled trustline
    BalanceError = 10,
    // Address balance authorization revoked
    BalanceDeauthorizedError = 11,
    // Spender allowance would overflow
    OverflowError = 12,
    // Trust line entry missing
    TrustlineMissingError = 13,
}
```

Source: [rs-soroban-env contract_error.rs](https://github.com/stellar/rs-soroban-env/blob/main/soroban-env-host/src/builtin%5Fcontracts/contract%5Ferror.rs)
