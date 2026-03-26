---
url: 'https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md'
title: 'CAP-0046-06: Stellar Asset Contract'
fetched_date: 2026-03-26
task: '0002'
---

# CAP-0046-06: Stellar Asset Contract

> Note: The fetched content covers the Stellar Asset Contract (CAP-0046-06), which defines the native smart contract implementation for classic Stellar assets. Despite the task context referencing Soroban events (CAP-67), this CAP is part of the CAP-0046 series covering Soroban-related proposals.

## Overview

This proposal introduces a native smart contract implementation for Stellar classic assets, enabling contracts to interoperate seamlessly with Native, AlphaNum4, and AlphaNum12 assets on the Stellar network.

## Key Components

### Data Structures

The contract uses several core data types:

- **AllowanceDataKey**: Tracks spender authorization with `from` and `spender` addresses
- **AllowanceValue**: Stores allowance amount (i128) and expiration ledger (u32)
- **BalanceValue**: Contains amount (i128), authorized flag, and clawback flag
- **DataKey**: Enum for allowance and balance persistence
- **InstanceDataKey**: Enum for admin and asset info storage

### Interfaces

**Descriptive Interface** provides token metadata:

- `decimals()` - Returns u32 decimal places
- `name()` - Returns String token name
- `symbol()` - Returns String token symbol

**Token Interface** implements ERC-20-like functionality:

- `allowance(from, spender)` - Returns approved amount
- `approve(from, spender, amount, expiration_ledger)` - Sets spending allowance
- `balance(id)` - Returns account balance
- `authorized(id)` - Checks authorization status
- `transfer(from, to, amount)` - Direct token transfer
- `transfer_from(spender, from, to, amount)` - Transfer using allowance
- `burn(from, amount)` - Destroy tokens
- `burn_from(spender, from, amount)` - Burn using allowance

**Admin Interface** manages supply and compliance:

- `set_admin(new_admin)` - Transfers administrator role
- `admin()` - Returns current administrator
- `set_authorized(id, authorize)` - Controls account authorization
- `mint(to, amount)` - Creates new tokens
- `clawback(from, amount)` - Recovers tokens from accounts

### Deployment Mechanism

Contracts deploy via `InvokeHostFunctionOp` using `HOST_FUNCTION_TYPE_CREATE_CONTRACT` with `CONTRACT_EXECUTABLE_TOKEN`. The `ContractIDPreimage` type `CONTRACT_ID_PREIMAGE_FROM_ASSET` ensures deterministic, unique contract addresses per asset.

The `create_asset_contract(asset)` host function enables programmatic deployment, while `init_asset(asset_bytes)` performs initialization — called automatically during deployment.

## Design Rationale

The native implementation reduces transaction fees compared to custom contracts while maintaining protocol knowledge of asset semantics. Native token contracts can potentially benefit from dedicated fee lanes or high-throughput exchange integration. The Native token lacks an admin to prevent unauthorized operations.

## Protocol Upgrade

This requires protocol version 20 and maintains backward compatibility while introducing new contract executable and preimage types.
