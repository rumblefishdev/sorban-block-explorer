---
prefix: R
title: Token Event Signatures (transfer, mint, burn)
status: mature
spawned_from: null
spawns: []
---

# Token Event Signatures: transfer, mint, burn

Research into the exact Soroban event topic/data structures for standard token operations.

## Sources

- [SEP-41 Token Interface](../sources/sep-41-token-interface.md) — the authoritative standard
- [rs-soroban-sdk token events.rs](../sources/rs-soroban-sdk-token-events.md) — canonical Rust struct definitions
- [CAP-67: Classic Ops emit events](../sources/cap-67-classic-ops-events.md) — Protocol 23 changes
- [Stellar Docs: Events](../sources/stellar-docs-events.md) — general event structure

## General Event Structure

All Soroban contract events use the `ContractEvent` XDR structure:

```
ContractEvent {
    contractID: Address,     // the emitting contract
    type: "contract",
    body: {
        topics: ScVal[],     // up to 4 topics
        data: ScVal          // payload
    }
}
```

`topics[0]` is always a `ScVal::Symbol` identifying the event type. Fields marked `#[topic]` in the SDK become additional topic entries. Non-topic fields become the data payload.

## Transfer Events

Two variants exist depending on whether muxed account IDs are used.

### Simple variant (most common)

```rust
#[contractevent(topics = ["transfer"], data_format = "single-value")]
pub struct TransferWithAmountOnly {
    #[topic] pub from: Address,   // topics[1]
    #[topic] pub to: Address,     // topics[2]
    pub amount: i128,             // data
}
```

| Slot        | Type             | Value             |
| ----------- | ---------------- | ----------------- |
| `topics[0]` | `ScVal::Symbol`  | `"transfer"`      |
| `topics[1]` | `ScVal::Address` | sender address    |
| `topics[2]` | `ScVal::Address` | recipient address |
| `data`      | `ScVal::I128`    | amount            |

### Muxed variant

```rust
#[contractevent(topics = ["transfer"], data_format = "map")]
pub struct Transfer {
    #[topic] pub from: Address,
    #[topic] pub to: Address,
    pub to_muxed_id: Option<u64>,  // data field
    pub amount: i128,               // data field
}
```

| Slot        | Type             | Value                                      |
| ----------- | ---------------- | ------------------------------------------ |
| `topics[0]` | `ScVal::Symbol`  | `"transfer"`                               |
| `topics[1]` | `ScVal::Address` | sender                                     |
| `topics[2]` | `ScVal::Address` | recipient                                  |
| `data`      | `ScVal::Map`     | `{amount: i128, to_muxed_id: Option<u64>}` |

## Mint Events

### Simple variant

```rust
#[contractevent(topics = ["mint"], data_format = "single-value")]
pub struct MintWithAmountOnly {
    #[topic] pub to: Address,   // topics[1]
    pub amount: i128,           // data
}
```

| Slot        | Type             | Value     |
| ----------- | ---------------- | --------- |
| `topics[0]` | `ScVal::Symbol`  | `"mint"`  |
| `topics[1]` | `ScVal::Address` | recipient |
| `data`      | `ScVal::I128`    | amount    |

**Note:** No admin/issuer address in topics. SEP-41 deliberately excludes the admin from the event signature.

### Muxed variant

```rust
#[contractevent(topics = ["mint"], data_format = "map")]
pub struct Mint {
    #[topic] pub to: Address,
    pub to_muxed_id: Option<u64>,
    pub amount: i128,
}
```

Data is `ScVal::Map` with `{amount: i128, to_muxed_id: Option<u64>}`.

## Burn Events

Single variant only:

```rust
#[contractevent(topics = ["burn"], data_format = "single-value")]
pub struct Burn {
    #[topic] pub from: Address,   // topics[1]
    pub amount: i128,             // data
}
```

| Slot        | Type             | Value                          |
| ----------- | ---------------- | ------------------------------ |
| `topics[0]` | `ScVal::Symbol`  | `"burn"`                       |
| `topics[1]` | `ScVal::Address` | holder whose tokens are burned |
| `data`      | `ScVal::I128`    | amount                         |

## Clawback Events (bonus)

```rust
#[contractevent(topics = ["clawback"], data_format = "single-value")]
pub struct Clawback {
    #[topic] pub from: Address,
    pub amount: i128,
}
```

Same structure as burn but with `topics[0] = "clawback"`.

## SAC vs Custom Token Differences

### Same format for Soroban-invoked operations

When both SAC (Stellar Asset Contract) and custom SEP-41 tokens are invoked via Soroban, they emit **identical event structures**. The SAC uses the same `soroban_token_sdk::events` structs.

### Extra topic for classic operations (Protocol 23+ / CAP-67)

When classic Stellar operations (Payment, PathPayment, etc.) affect a wrapped asset, they emit SEP-41-compatible events with an **extra `sep0011_asset` topic**:

| Event    | SAC topics (classic op)                 |
| -------- | --------------------------------------- |
| transfer | `["transfer", from, to, sep0011_asset]` |
| mint     | `["mint", to, sep0011_asset]`           |
| burn     | `["burn", from, sep0011_asset]`         |
| clawback | `["clawback", from, sep0011_asset]`     |

The `sep0011_asset` is a `ScVal::String` like `"native"` or `"credit_alphanum4:USDC:GA5ZSE..."`.

### Pre vs Post Protocol 23 (September 2025)

| Aspect                | Pre-Protocol 23             | Post-Protocol 23 (current)          |
| --------------------- | --------------------------- | ----------------------------------- |
| Mint topics           | `["mint", admin, to]`       | `["mint", to, sep0011_asset]`       |
| Clawback topics       | `["clawback", admin, from]` | `["clawback", from, sep0011_asset]` |
| Classic ops           | No events emitted           | Emit SEP-41 events                  |
| `sep0011_asset` topic | Not present                 | Added as final topic (SAC only)     |

### Implications for Event Interpreter

1. **Match on `topics[0]`** being `Symbol("transfer")`, `Symbol("mint")`, or `Symbol("burn")`.
2. **Handle variable topic lengths** — SAC classic-op events have one more topic than custom tokens or Soroban-invoked SAC.
3. **Handle two data formats** for transfer/mint: simple `i128` or `Map{amount, to_muxed_id}`.
4. **Use `contract_id`** to identify which token emitted the event (for metadata lookup).

## SEP-41 Standard Compliance

SEP-41 is the token interface standard for Soroban:

- Defines required **events**: `transfer`, `burn`, `approve`, `mint`, `clawback`
- `mint()` and `clawback()` are **not required as interface functions** — contracts have naming flexibility — but when minting/clawback occurs, the standard event format **must** be emitted
- The `data_format` attribute controls serialization: `"single-value"` → `i128`, `"map"` → `ScVal::Map`, `"vec"` → `ScVal::Vec`
- Any token that wants interoperability with wallets, DEXes, and explorers should follow SEP-41
