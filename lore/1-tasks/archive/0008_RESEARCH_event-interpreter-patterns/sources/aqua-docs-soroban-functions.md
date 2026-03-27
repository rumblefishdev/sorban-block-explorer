# Aquarius: Soroban Functions (AMM Router)

**Source:** https://docs.aqua.network/developers/aquarius-soroban-functions
**Fetched:** 2026-03-27

---

## Contract Address

**CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK**

This smart contract serves as the entry point for all AMM functionality in the Aquarius system.

## Function Signatures

### Deposit

```rust
fn deposit(
  e: Env,
  user: Address,
  tokens: Vec<Address>,
  pool_index: BytesN<32>,
  desired_amounts: Vec<u128>,
  min_shares: u128,
) -> (Vec<u128>, u128);
```

### Withdraw

```rust
fn withdraw(
  e: Env,
  user: Address,
  tokens: Vec<Address>,
  pool_index: BytesN<32>,
  share_amount: u128,
  min_amounts: Vec<u128>,
) -> Vec<u128>;
```

### Swap Chained

```rust
fn swap_chained(
    e: Env,
    user: Address,
    swaps_chain: Vec<(Vec<Address>, BytesN<32>, Address)>,
    token_in: Address,
    in_amount: u128,
    out_min: u128,
) -> u128
```

Maximum 4 pools allowed in chain due to Soroban constraints.
