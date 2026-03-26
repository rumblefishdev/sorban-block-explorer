---
source_url: 'https://developers.stellar.org/docs/learn/fundamentals/contract-development/types/fully-typed-contracts'
title: 'Fully-Typed Contracts'
fetched_date: 2026-03-26
task_id: '0003'
---

# Fully-Typed Contracts

## Overview

When compiling contracts with [soroban-sdk](https://developers.stellar.org/docs/tools/sdks/contract-sdks#soroban-rust-sdk), the resulting Wasm file includes a custom section containing "a machine-readable description of your contract's interface types." This specification is formatted using XDR, a data format used throughout Stellar.

## Stellar CLI: `stellar contract invoke`

The Stellar CLI generates a unique command-line interface for each smart contract, constructed dynamically from on-chain interface types and including author documentation.

Example usage with the native asset contract on the Test network:

```
$ stellar contract invoke --network testnet --id CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC -- --help
Usage: CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC [COMMAND]

Commands:
  balance         Returns the balance of `id`.

                      # Arguments

                      * `id` - The address for which a balance is being queried. If the
                      address has no existing balance, returns 0.
  ...
  transfer        Transfer `amount` from `from` to `to`.

                      # Arguments

                      * `from` - The address holding the balance of tokens which will be
                      withdrawn from.
                      * `to` - The address which will receive the transferred tokens.
                      * `amount` - The amount of tokens to be transferred.

                      # Events

                      Emits an event with topics `["transfer", from: Address, to: Address],
                      data = amount: i128`
  ...

Options:
  -h, --help  Print help
```

> **Tip:** The double dash (`--`) is a common CLI pattern where everything after it gets passed to the child process, similar to `npm run` and `cargo run`.

## Stellar JS SDK: `contract.Client`

Create a contract client using this JavaScript:

```javascript
import { contract } from '@stellar/stellar-sdk';
const xlm = contract.Client.from({
  contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
  networkPassphrase: 'Test SDF Network ; September 2015',
  rpcUrl: 'https://soroban-testnet.stellar.org',
});
```

Call contract methods ergonomically:

```javascript
xlm.balance({ id: 'G123…' });
```

Generate TypeScript bindings for type-safe development:

```bash
stellar contract bindings typescript \
  --network testnet \
  --output-dir xlm --overwrite \
  --id CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC
```

This creates a fully-typed NPM module for the specified contract.

## Already the Best; Just Getting Started

The availability of all contract interface types on-chain from day one eliminates external API dependencies and enables reliable interactions without additional authentication overhead. Soroban tooling demonstrates how foundational platforms can deliver seamless developer experiences.

Future enhancements include:

- Adaptive GUIs that function as interactive documentation
- Auto-generated code for cross-contract calls
- Enhanced component interoperability through WebAssembly integration
