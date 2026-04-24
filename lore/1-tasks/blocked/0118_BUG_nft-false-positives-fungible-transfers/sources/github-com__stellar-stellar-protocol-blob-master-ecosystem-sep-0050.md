---
url: 'https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0050.md'
title: 'SEP-0050: Non-Fungible Tokens'
fetched_date: 2026-04-14
task_id: '0118'
image_count: 0
---

# SEP-0050: Non-Fungible Tokens

## Preamble

```
SEP: 0050
Title: Non-Fungible Tokens
Author: OpenZeppelin, Boyan Barakov <@brozorec>, Özgün Özerk <@ozgunozerk>
Status: Draft
Created: 2025-03-10
Updated: 2025-03-10
Version: 0.1.0
Discussion: https://github.com/stellar/stellar-protocol/discussions/1674
```

## Summary

This proposal defines a standard contract interface for non-fungible tokens. The interface is similar to ERC721, but factors in the differences between Ethereum and Stellar.

## Motivation

A non-fungible asset (NFT) is a fundamental concept on blockchains, representing unique and indivisible digital assets. While most blockchain ecosystems have established standards for NFTs, such as ERC-721 and ERC-1155 in Ethereum, the Stellar ecosystem lacks a standardized interface for NFTs. This absence may lead to fragmentation, making it difficult to ensure interoperability between different NFT contracts and applications.

Currently, while it is **technically possible** to create NFTs on Stellar by issuing non-divisible assets with unique metadata as outlined in **SEP-39** —and one can deploy the SAC for it afterwards—this approach is unintuitive for developers, due to SAC being mainly designed for fungible tokens, and comes with the following limitations:

- **Non Fungibility** - There are two alternatives to issue NFTs via Classic Stellar Assets: one Asset per NFT, or one Asset per collection. One Asset per NFT requires the creation of a new Classic Stellar Asset for each NFT, which can be cumbersome and inefficient. One Asset per collection resolves this problem, but then the NFTs inside the collection become fungible instead of non-fungible (one can exchange an NFT inside the collection for another NFT inside the same collection).
- **Metadata handling** – SAC NFTs store metadata off-chain, whereas if wanted, smart contract NFTs can store more on-chain metadata with further customizations.
- **Permission controls** – SACs follow the standard asset authorization model, while smart contract NFTs enable fine-grained access controls and custom rules.
- **Transfers & ownership** – SAC NFTs rely on trust-lines and asset balances, whereas smart contract NFTs can implement more explicit and flexible ownership structures.
- **Customizability** – SACs are limited to predefined asset behaviors, whereas smart contract NFTs allow advanced logic like royalties, leasing, or dynamic attributes.

By defining NFTs as smart contracts, developers gain a more intuitive and straightforward way to issue NFTs, along with greater **flexibility, programmability, and interoperability**, enabling use cases beyond simple asset issuance.

This proposal defines a non-fungible token interface that provides core NFT functionality, including ownership management, transfers, and approvals, without enforcing opinionated behaviors beyond the standard expectations for NFTs. By establishing this interface, NFT contracts can interact seamlessly with other contracts and applications that support the standard, ensuring broader usability and compatibility within the Stellar network.

## Interface

NFTs can have diverse use cases, and there is no universal `token_id` format that fits all scenarios. Some implementations may use sequential numbers, while others may opt for UUIDs or hashes. To accommodate this variability, this interface remains agnostic to the specific `token_id` format, defining it as a generic `TokenID` type, which should be an unsigned integer.

Additionally, since a single account cannot hold more tokens than the total supply of `TokenIDs`, we introduce a `Balance` type. In most cases, `Balance` should use the same type as `TokenID` for consistency.

```rust
/// The `NonFungibleToken` trait defines the core functionality for non-fungible
/// tokens. It provides a standard interface for managing
/// transfers and approvals associated with non-fungible tokens.
pub trait NonFungibleToken {
    /// Returns the number of tokens in `owner`'s account.
    fn balance(e: &Env, owner: Address) -> Balance;

    /// Returns the address of the owner of the given `token_id`.
    fn owner_of(e: &Env, token_id: TokenID) -> Address;

    /// Transfers `token_id` token from `from` to `to`.
    ///
    /// # Events
    ///
    /// * topics - `["transfer", from: Address, to: Address]`
    /// * data - `[token_id: TokenID]`
    fn transfer(e: &Env, from: Address, to: Address, token_id: TokenID);

    /// Transfers `token_id` token from `from` to `to` by using `spender`s approval.
    ///
    /// # Events
    ///
    /// * topics - `["transfer", from: Address, to: Address]`
    /// * data - `[token_id: TokenID]`
    fn transfer_from(e: &Env, spender: Address, from: Address, to: Address, token_id: TokenID);

    /// Gives permission to `approved` to transfer `token_id` token to another account.
    ///
    /// # Events
    ///
    /// * topics - `["approve", from: Address, to: Address]`
    /// * data - `[token_id: TokenID, live_until_ledger: u32]`
    fn approve(e: &Env, approver: Address, approved: Address, token_id: TokenID, live_until_ledger: u32);

    /// Approve or remove `operator` as an operator for the owner.
    ///
    /// # Events
    ///
    /// * topics - `["approve_for_all", from: Address]`
    /// * data - `[operator: Address, live_until_ledger: u32]`
    fn approve_for_all(e: &Env, owner: Address, operator: Address, live_until_ledger: u32);

    fn get_approved(e: &Env, token_id: TokenID) -> Option<Address>;
    fn is_approved_for_all(e: &Env, owner: Address, operator: Address) -> bool;
    fn name(e: &Env) -> String;
    fn symbol(e: &Env) -> String;
    fn token_uri(e: &Env, token_id: TokenID) -> String;
}
```

## Events

### Transfer Event

The transfer event is emitted when an NFT is transferred from one address to another.

**Topics:**

- `Symbol` with value `"transfer"`
- `Address`: the address holding the token that was transferred.
- `Address`: the address that received the token.

**Data:**

- `TokenID`: the identifier of the transferred token.

### Approve Event

**Topics:**

- `Symbol` with value `"approve"`
- `Address`: the owner of the token.
- `TokenID`: the identifier of the token.

**Data:**

- `Address`: the approved address.
- `u32`: the expiration ledger.

### Approve for All Event

**Topics:**

- `Symbol` with value `"approve_for_all"`
- `Address`: the owner of the tokens.

**Data:**

- `Address`: the operator receiving or losing permission.
- `u32`: the expiration ledger. If `0`, the approval is revoked.

### Mint Event

The event has topics:

- `Symbol` with value `"mint"`
- `Address`: the address to hold the newly minted token.

The event has data:

- `TokenID` the identifier of the minted token.

## Notes on `name()`, `symbol()` and `token_uri()`

Those methods are not part of ERC721, but including them in the standard, because exposing metadata through this API proved itself as a common practice for marketplaces to display NFT details. Furthermore, it's important to be consistent within the Stellar ecosystem as SEP-41 also defines metadata as part of the core interface for fungible tokens.

### Non-Fungible Metadata JSON Schema

```json
{
  "title": "Non-Fungible Metadata",
  "type": "object",
  "properties": {
    "name": { "type": "string" },
    "description": { "type": "string" },
    "image": { "type": "string" },
    "external_url": { "type": "string" },
    "attributes": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "display_type": { "type": "string" },
          "trait_type": { "type": "string" },
          "value": { "anyOf": [{ "type": "string" }, { "type": "number" }] },
          "max_value": { "type": "number" }
        }
      }
    }
  }
}
```

## Changelog

- `0.1.0`: Initial draft
