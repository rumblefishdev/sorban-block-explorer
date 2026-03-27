# Phoenix Protocol: Pool Contract (Swap Events)

**Source:** https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool/src/contract.rs
**Fetched:** 2026-03-27

---

## Swap Function Signature

```rust
fn do_swap(
    env: Env,
    sender: Address,
    offer_asset: Address,
    offer_amount: i128,
    ask_asset_min_amount: Option<i128>,
    max_spread: Option<i64>,
    max_allowed_fee_bps: Option<i64>,
) -> i128
```

## Swap Event Emissions

Phoenix emits **one event per field** (8 separate events per swap):

```rust
env.events().publish(("swap", "sender"), sender);
env.events().publish(("swap", "sell_token"), sell_token);
env.events().publish(("swap", "offer_amount"), offer_amount);
env.events()
    .publish(("swap", "actual received amount"), actual_received_amount);
env.events().publish(("swap", "buy_token"), buy_token);
env.events()
    .publish(("swap", "return_amount"), compute_swap.return_amount);
env.events()
    .publish(("swap", "spread_amount"), compute_swap.spread_amount);
env.events().publish(
    ("swap", "referral_fee_amount"),
    compute_swap.referral_fee_amount,
);
```

## Event Structure (per event)

Each individual event has:

- `topics[0]` = `Symbol("swap")`
- `topics[1]` = `Symbol("<field_name>")` — one of: `"sender"`, `"sell_token"`, `"offer_amount"`, `"actual received amount"`, `"buy_token"`, `"return_amount"`, `"spread_amount"`, `"referral_fee_amount"`
- `data` = single value (`Address` for sender/tokens, `i128` for amounts)

## Note

To reconstruct a full swap from Phoenix events, you must group 8 consecutive events from the same contract invocation. This is significantly more complex than Soroswap or Aqua which emit a single structured event per swap.
