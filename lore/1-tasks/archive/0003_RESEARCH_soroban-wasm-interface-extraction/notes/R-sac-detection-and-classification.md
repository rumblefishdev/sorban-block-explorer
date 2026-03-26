---
prefix: R
title: SAC Detection and Contract Type Classification
status: mature
spawned_from: null
spawns: []
---

# SAC Detection and Contract Type Classification

## SAC (Stellar Asset Contract) Detection

### What are SACs?

SACs are built-in Soroban contracts that wrap classic Stellar assets (XLM, USDC, etc.) to make them accessible from Soroban smart contracts. They are **not deployed as WASM** -- they are created by the Stellar host environment itself via a special host function.

### Detection Strategy: Deployment Mechanism (Recommended, Primary)

SACs are created via `InvokeHostFunction` with `HostFunction` type `HOST_FUNCTION_TYPE_CREATE_CONTRACT`, where the `ContractIdPreimage` uses `CONTRACT_ID_PREIMAGE_FROM_ASSET` (not `CONTRACT_ID_PREIMAGE_FROM_ADDRESS`).

When processing `LedgerEntryChanges`:

1. Look for `InvokeHostFunctionOp` in the transaction
2. Check if the host function is `createContract`
3. Check if the contract ID preimage is `fromAsset` -- this definitively means it's a SAC

**Reliability: 100%.** This is the canonical mechanism -- there is no other way to create a SAC.

### Detection Strategy: ContractExecutable Type (Simplest for Ledger Processing)

SACs do not have associated WASM bytecode in the ledger. When a contract entry appears in LedgerEntryChanges:

- Regular contracts: `ContractDataEntry` references a WASM hash, and the WASM code exists as a separate `ContractCodeEntry`
- SACs: The contract executable type is `CONTRACT_EXECUTABLE_STELLAR_ASSET` (see [stellar-xdr-contract.x](../sources/stellar-xdr-contract.x):168)

In the XDR:

```
ContractExecutable union:
  CONTRACT_EXECUTABLE_WASM -> has wasmHash
  CONTRACT_EXECUTABLE_STELLAR_ASSET -> no WASM, is a SAC
```

**This is the simplest detection method during ledger processing**: just check the `ContractExecutable` type on the contract's `LedgerEntry`.

### Detection Strategy: Interface Matching (Not Recommended as Primary)

SACs implement the SEP-41 token interface. Matching against known function names works but is fragile -- any custom token contract could implement the same interface. Use only as a secondary validation.

### Recommended Approach

```
if contractExecutable.type === CONTRACT_EXECUTABLE_STELLAR_ASSET:
  is_sac = true
  contract_type = 'token'
else:
  is_sac = false
  contract_type = classifyFromInterface(wasmSpec)
```

## SEP-41 Token Interface (Standard)

The Stellar Ecosystem Proposal 41 defines the standard token interface for Soroban. All SACs implement this, and well-behaved custom tokens should too.

### Required Functions (10)

| Function        | Inputs                                                                    | Output   | Category |
| --------------- | ------------------------------------------------------------------------- | -------- | -------- |
| `allowance`     | `(from: address, spender: address)`                                       | `i128`   | Query    |
| `approve`       | `(from: address, spender: address, amount: i128, expiration_ledger: u32)` | `void`   | Mutation |
| `balance`       | `(id: address)`                                                           | `i128`   | Query    |
| `transfer`      | `(from: address, to: address, amount: i128)`                              | `void`   | Mutation |
| `transfer_from` | `(spender: address, from: address, to: address, amount: i128)`            | `void`   | Mutation |
| `burn`          | `(from: address, amount: i128)`                                           | `void`   | Mutation |
| `burn_from`     | `(spender: address, from: address, amount: i128)`                         | `void`   | Mutation |
| `decimals`      | `()`                                                                      | `u32`    | Query    |
| `name`          | `()`                                                                      | `string` | Query    |
| `symbol`        | `()`                                                                      | `string` | Query    |

### Admin Functions (SAC-specific, also common in custom tokens)

| Function         | Inputs                           | Output    |
| ---------------- | -------------------------------- | --------- |
| `set_admin`      | `(new_admin: address)`           | `void`    |
| `admin`          | `()`                             | `address` |
| `set_authorized` | `(id: address, authorize: bool)` | `void`    |
| `authorized`     | `(id: address)`                  | `bool`    |
| `mint`           | `(to: address, amount: i128)`    | `void`    |
| `clawback`       | `(from: address, amount: i128)`  | `void`    |

## Contract Type Classification Heuristics

### Strategy: Interface Pattern Matching

Classification should work by matching extracted function names against known interface patterns. This is a heuristic approach -- not all contracts will match cleanly.

### `token` Classification

**Required functions** (must have ALL of these): `transfer`, `balance`, `decimals`, `name`, `symbol`

**Strong indicators**: `approve`, `allowance`, `burn`, `mint`, `transfer_from`

**Confidence**: High if 8+ of the 10 SEP-41 functions are present.

### `dex` Classification

**Key function patterns**:

- `swap`, `add_liquidity`, `remove_liquidity` (AMM-style)
- `deposit`, `withdraw` combined with `get_reserves` or `get_price`
- Presence of `pool` or `pair` in type/struct names
- Functions like `get_amount_out`, `get_amount_in`

**Heuristic**: Has swap/trade functions AND liquidity management functions, but does NOT look like a simple token.

### `lending` Classification

**Key function patterns**:

- `supply`, `borrow`, `repay`, `liquidate`, `withdraw`
- `get_collateral_factor`, `get_borrow_rate`, `get_supply_rate`
- Presence of `reserve`, `pool`, `position` in type names

**Heuristic**: Has supply/borrow/repay function set.

### `nft` Classification

**Key function patterns**:

- `mint` with a `token_id` or similar unique identifier parameter
- `transfer` with a `token_id` parameter (not `amount: i128`)
- `owner_of`, `token_uri`, `metadata`
- Functions operating on individual token IDs rather than fungible amounts

**Heuristic**: Has transfer/mint with token_id parameter rather than i128 amounts.

### `other` Classification

Default when no strong pattern match is found. This will be the most common classification initially.

### Classification Algorithm

```typescript
function classifyContract(functions: FunctionSpec[]): ContractType {
  const funcNames = new Set(functions.map((f) => f.name));

  // Token: must have core SEP-41 functions
  const tokenCoreFuncs = ['transfer', 'balance', 'decimals', 'name', 'symbol'];
  const tokenScore = tokenCoreFuncs.filter((f) => funcNames.has(f)).length;
  if (tokenScore >= 4) return 'token';

  // DEX: swap + liquidity functions
  const dexIndicators = [
    'swap',
    'add_liquidity',
    'remove_liquidity',
    'get_reserves',
    'deposit',
    'withdraw',
  ];
  const dexScore = dexIndicators.filter((f) => funcNames.has(f)).length;
  if (dexScore >= 2 && funcNames.has('swap')) return 'dex';

  // Lending: supply + borrow + repay
  const lendingIndicators = [
    'supply',
    'borrow',
    'repay',
    'liquidate',
    'withdraw',
    'get_collateral_factor',
  ];
  const lendingScore = lendingIndicators.filter((f) => funcNames.has(f)).length;
  if (lendingScore >= 3) return 'lending';

  // NFT: token_id based operations
  const hasTokenId = functions.some((f) =>
    f.inputs.some((i) => i.name === 'token_id' || i.name === 'nft_id')
  );
  if (hasTokenId && funcNames.has('mint')) return 'nft';

  return 'other';
}
```

### Limitations and Future Improvements

1. **Heuristic fragility**: Contracts may use non-standard function names. The classification should be treated as best-effort.
2. **Event-based classification**: Runtime events could strengthen classification. For example, a DEX emitting `swap` events with price data is a strong signal. This is a future enhancement beyond deployment-time analysis.
3. **Known contract registry**: Maintaining a mapping of known WASM hashes to contract types would provide exact classification for popular contracts. This could be a separate data source updated independently.
4. **Token check precedence**: Token classification should run first because many DEX/lending contracts also include token-like functions for their LP/receipt tokens.

## Sources

- [SEP-0041: Soroban Token Interface](../sources/sep-0041-token-interface.md) -- defines the 10 required token functions
- [Stellar Docs: Stellar Asset Contract](../sources/stellar-docs-sac.md) -- SAC deployment, authorization, admin functions, error codes
- [Stellar Docs: Token Interface](../sources/stellar-docs-token-interface.md) -- full `TokenInterface` Rust trait with doc comments
- [Stellar Docs: Deploy SAC with Code](../sources/stellar-docs-deploy-sac.md) -- JavaScript guide for SAC deployment via `createStellarAssetContract`
- [Stellar XDR: Stellar-contract.x](../sources/stellar-xdr-contract.x) -- defines `ContractExecutableType` enum (`CONTRACT_EXECUTABLE_WASM`, `CONTRACT_EXECUTABLE_STELLAR_ASSET`) and `ContractExecutable` union
- [Stellar XDR: Stellar-transaction.x](../sources/stellar-xdr-transaction.x) -- defines `CreateContractArgs` with `ContractIDPreimage` + `ContractExecutable`
