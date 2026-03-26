---
source_url: 'https://developers.stellar.org/docs/build/guides/conventions/wasm-metadata'
fetched_date: 2026-03-26
title: 'Write metadata for your contract'
task_id: '0003'
---

# Write metadata for your contract

The `contractmeta!` macro in the Rust SDK enables developers to embed two strings—a `key` and `val`—within a serialized `SCMetaEntry::SCMetaV0` XDR object in the custom section of Wasm contracts. This metadata section is named `contractmetav0`, allowing tools to read and present the information to users.

## Example Usage

The liquidity pool example demonstrates implementation of the `contractmeta!` macro:

```rust
// Metadata that is added on to the Wasm custom section
contractmeta!(
    key = "Description",
    val = "Constant product AMM with a .3% swap fee"
);

pub trait LiquidityPoolTrait {...
```

## Related Guides

- [Making cross-contract calls](https://developers.stellar.org/docs/build/guides/conventions/cross-contract) — Invoke smart contracts from within another smart contract
- [Deploy a contract from installed Wasm bytecode using a deployer contract](https://developers.stellar.org/docs/build/guides/conventions/deploy-contract) — Deploy contracts from pre-installed Wasm bytecode
- [Deploy a SAC for a Stellar asset using code](https://developers.stellar.org/docs/build/guides/conventions/deploy-sac-with-code) — Deploy a SAC using Javascript SDK
- [Organize contract errors with an error enum type](https://developers.stellar.org/docs/build/guides/conventions/error-enum) — Manage contract error communication
- [Extend a deployed contract's TTL with code](https://developers.stellar.org/docs/build/guides/conventions/extending-wasm-ttl) — Extend Wasm code time-to-live
- [Upgrading Wasm bytecode for a deployed contract](https://developers.stellar.org/docs/build/guides/conventions/upgrading-contracts) — Upgrade Wasm bytecode
- [Workspaces](https://developers.stellar.org/docs/build/guides/conventions/workspace) — Organize contracts using Cargo workspaces
