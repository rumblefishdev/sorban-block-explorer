---
prefix: R
title: DEX Swap Event Signatures (Soroswap, Phoenix, Aqua)
status: mature
spawned_from: null
spawns: []
---

# DEX Swap Event Signatures

Research into swap event topic/data structures emitted by Soroban DEXes.

## Sources

- [Soroswap Events](../sources/soroswap-events.md) — pair and router event definitions
- [Soroswap Mainnet Contracts](../sources/soroswap-mainnet-contracts.md) — deployed addresses
- [Soroswap Aggregator Mainnet](../sources/soroswap-aggregator-mainnet.md) — adapter addresses (Phoenix, Aqua)
- [Phoenix Pool Contract](../sources/phoenix-pool-contract.md) — XYK pool swap event emissions
- [Phoenix Stable Pool Contract](../sources/phoenix-stable-pool-contract.md) — stable pool swap events
- [Phoenix Multihop Contract](../sources/phoenix-multihop-contract.md) — confirms no swap events emitted
- [Aqua AMM Events](../sources/aqua-amm-events.md) — pool and router event definitions
- [Aqua Docs: Soroban Functions](../sources/aqua-docs-soroban-functions.md) — Router mainnet address

## 1. Soroswap

Soroswap emits swap events at **two levels**: the Pair contract (pool-level) and the Router contract (user-facing).

### SoroswapPair Swap Event

Source: `soroswap/core/contracts/pair/src/event.rs`

```rust
#[contracttype]
pub struct SwapEvent {
    pub to: Address,
    pub amount_0_in: i128,
    pub amount_1_in: i128,
    pub amount_0_out: i128,
    pub amount_1_out: i128,
}

e.events().publish(("SoroswapPair", symbol_short!("swap")), event);
```

| Slot        | Type            | Value                                                        |
| ----------- | --------------- | ------------------------------------------------------------ |
| `topics[0]` | `ScVal::Symbol` | `"SoroswapPair"`                                             |
| `topics[1]` | `ScVal::Symbol` | `"swap"`                                                     |
| `data`      | Struct          | `{to, amount_0_in, amount_1_in, amount_0_out, amount_1_out}` |

The emitting `contract_id` is the pair contract itself. Token 0 and token 1 must be resolved by querying the pair contract.

### SoroswapRouter Swap Event

Source: `soroswap/core/contracts/router/src/event.rs`

```rust
#[contracttype]
pub struct SwapEvent {
    pub path: Vec<Address>,
    pub amounts: Vec<i128>,
    pub to: Address,
}

e.events().publish(("SoroswapRouter", symbol_short!("swap")), event);
```

| Slot        | Type            | Value                                                   |
| ----------- | --------------- | ------------------------------------------------------- |
| `topics[0]` | `ScVal::Symbol` | `"SoroswapRouter"`                                      |
| `topics[1]` | `ScVal::Symbol` | `"swap"`                                                |
| `data`      | Struct          | `{path: Vec<Address>, amounts: Vec<i128>, to: Address}` |

The `path` is the ordered list of token addresses (first = input, last = output). The `amounts` array contains amounts at each step. This is the most useful event for multi-hop swaps.

### Soroswap Mainnet Addresses

| Contract | Address                                                    |
| -------- | ---------------------------------------------------------- |
| Factory  | `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` |
| Router   | `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` |

Pair contracts are deployed per token pair by the Factory — no single address.

## 2. Phoenix Protocol

Phoenix uses a fundamentally different approach: **one event per field** rather than a single structured event.

Source: `Phoenix-Protocol-Group/phoenix-contracts/contracts/pool/src/contract.rs`

### XYK Pool Swap Events (8 events per swap)

```rust
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events().publish(("swap", "actual received amount"), actual_received_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events().publish(("swap", "return_amount"), compute_swap.return_amount);
env.events().publish(("swap", "spread_amount"), compute_swap.spread_amount);
env.events().publish(("swap", "referral_fee_amount"), compute_swap.referral_fee_amount);
```

Each individual event:

| Slot        | Type            | Value                                         |
| ----------- | --------------- | --------------------------------------------- |
| `topics[0]` | `ScVal::Symbol` | `"swap"`                                      |
| `topics[1]` | `ScVal::Symbol` | field name (e.g., `"sender"`, `"sell_token"`) |
| `data`      | varies          | single value (`Address` or `i128`)            |

### Stable Pool — same pattern, fewer fields

```rust
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events().publish(("swap", "return_amount"), return_amount);
env.events().publish(("swap", "spread_amount"), spread_amount);
```

### Phoenix Multihop

The multihop contract does **not** emit its own swap events — it delegates to individual pool contracts.

### Phoenix Mainnet Addresses

Not found in Phoenix public repositories. The Soroswap aggregator (see [soroswap-aggregator-mainnet.md](../sources/soroswap-aggregator-mainnet.md)) references a Phoenix Adapter at `CCEBUGFV3D73OMV7MUXXA43AREY53MUHVD5SMUM7YZODNGY4NZBA2TSC`, but this is the Soroswap aggregator's adapter, not Phoenix's own pool contracts.

## 3. Aqua DEX (Aquarius AMM)

Aqua emits events at pool level (`trade`) and router level (`swap`).

### Pool-Level Trade Event

Source: `AquaToken/soroban-amm/liquidity_pool_events/src/lib.rs`

```rust
fn trade(&self, user: Address, token_in: Address, token_out: Address,
         in_amount: u128, out_amount: u128, fee_amount: u128) {
    e.events().publish(
        (Symbol::new(e, "trade"), token_in, token_out, user),
        (in_amount as i128, out_amount as i128, fee_amount as i128),
    );
}
```

| Slot        | Type             | Value                                                      |
| ----------- | ---------------- | ---------------------------------------------------------- |
| `topics[0]` | `ScVal::Symbol`  | `"trade"`                                                  |
| `topics[1]` | `ScVal::Address` | token_in                                                   |
| `topics[2]` | `ScVal::Address` | token_out                                                  |
| `topics[3]` | `ScVal::Address` | user                                                       |
| `data`      | Tuple            | `(i128, i128, i128)` — (in_amount, out_amount, fee_amount) |

Note: Amounts are cast from `u128` to `i128` before publishing.

### Router-Level Swap Event

Source: `AquaToken/soroban-amm/liquidity_pool_router/src/events.rs`

```rust
fn swap(&self, tokens: Vec<Address>, user: Address, pool_id: Address,
        token_in: Address, token_out: Address, in_amount: u128, out_amt: u128) {
    self.env().events().publish(
        (Symbol::new(self.env(), "swap"), tokens, user),
        (pool_id, token_in, token_out, in_amount, out_amt),
    );
}
```

| Slot        | Type                  | Value                                                                                             |
| ----------- | --------------------- | ------------------------------------------------------------------------------------------------- |
| `topics[0]` | `ScVal::Symbol`       | `"swap"`                                                                                          |
| `topics[1]` | `ScVal::Vec<Address>` | pool tokens                                                                                       |
| `topics[2]` | `ScVal::Address`      | user                                                                                              |
| `data`      | Tuple                 | `(Address, Address, Address, u128, u128)` — (pool_id, token_in, token_out, in_amount, out_amount) |

### Aqua Mainnet Addresses

| Contract | Address                                                    |
| -------- | ---------------------------------------------------------- |
| Router   | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` |

## Comparative Analysis

### Event Naming

| DEX             | Event Symbol                  | Convention                     |
| --------------- | ----------------------------- | ------------------------------ |
| Soroswap Pair   | `"SoroswapPair"` + `"swap"`   | Namespaced two-topic prefix    |
| Soroswap Router | `"SoroswapRouter"` + `"swap"` | Namespaced two-topic prefix    |
| Phoenix Pool    | `"swap"` + `"<field>"`        | One event per field            |
| Aqua Pool       | `"trade"`                     | Single event, tokens in topics |
| Aqua Router     | `"swap"`                      | Single event, tokens in data   |

### Key Differences

1. **Soroswap** uses namespace prefixes (`"SoroswapPair"` / `"SoroswapRouter"`) as `topics[0]`, making protocol filtering easy. All data packed in one struct.

2. **Phoenix** emits **8 separate events per swap**. To reconstruct a full swap, you must group consecutive events from the same contract invocation. This is significantly harder to parse.

3. **Aqua** puts token addresses in topics (indexable via GIN), amounts in data body. Pool events use `"trade"` (not `"swap"`).

4. **Amount types**: Soroswap uses `i128`, Aqua casts `u128` → `i128`, Phoenix uses `i128` directly.

5. **Fee visibility**: Only Aqua includes fee in swap data. Phoenix emits `spread_amount` as separate event. Soroswap pair events omit fees.

6. **Path/routing**: Only Soroswap Router and Aqua Router include multi-hop path information.

### Implications for Event Interpreter

- **Soroswap** is easiest to interpret: match `topics[0]="SoroswapPair"` and `topics[1]="swap"`, decode single struct.
- **Aqua** pool events: match `topics[0]="trade"`, tokens already in topics.
- **Phoenix** requires correlating 8 separate events from the same transaction — more complex, may be deferred to a later phase.
- For Phase 1, recommend supporting **Soroswap** and **Aqua**, deferring Phoenix.
