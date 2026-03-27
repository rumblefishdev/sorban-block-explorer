---
prefix: G
title: structured_data JSONB Schemas
status: mature
spawned_from: null
spawns: []
---

# structured_data JSONB Schemas

Generated artifact: the shape of the `structured_data` JSONB column in `event_interpretations` for each pattern type.

## Derived from

- [R-dex-swap-event-signatures.md](R-dex-swap-event-signatures.md) — Soroswap fields (path, amounts, pair_contract), Aqua fields (fee_amount, pool_contract), DEX identifiers
- [R-token-event-signatures.md](R-token-event-signatures.md) — transfer/mint/burn field structures, muxed account support (to_muxed_id)
- [Soroswap Mainnet Contracts](../sources/soroswap-mainnet-contracts.md) — Router address `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH`
- Task README — `event_interpretations` table schema (interpretation_type, human_readable, structured_data)

## Design Principles

1. **Queryable**: Fields at top level for efficient JSONB queries (`structured_data->>'token_symbol'`)
2. **Consistent**: Common fields (`addresses`, `amounts`) use the same naming across types
3. **Complete**: Include all values needed to reconstruct the human-readable summary
4. **Typed**: Amounts as strings (to preserve i128 precision), addresses as full strings

## Transfer Schema

```jsonc
{
  "type": "transfer",
  "from": "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
  "to": "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5",
  "amount": "1000000000", // raw i128 as string
  "amount_display": "100.00", // formatted with decimals
  "token_contract": "CCW67...", // emitting contract ID
  "token_symbol": "USDC", // from metadata (nullable)
  "token_decimals": 7, // from metadata (nullable)
  "to_muxed_id": null // u64 or null (for muxed accounts)
}
```

## Swap Schema (Soroswap)

```jsonc
{
  "type": "swap",
  "dex": "soroswap",
  "dex_contract": "CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH",
  "pair_contract": "CABC...", // Soroswap pair contract (for pair-level events)
  "user": "GA5Z...",
  "token_in": {
    "contract": "CCW67...",
    "symbol": "USDC",
    "decimals": 7
  },
  "token_out": {
    "contract": "CAS3J...",
    "symbol": "XLM",
    "decimals": 7
  },
  "amount_in": "1000000000",
  "amount_in_display": "100.00",
  "amount_out": "952000000",
  "amount_out_display": "95.20",
  "path": ["CCW67...", "CAS3J..."], // token path (router events)
  "amounts": ["1000000000", "952000000"] // amounts at each step
}
```

## Swap Schema (Aqua)

```jsonc
{
  "type": "swap",
  "dex": "aquarius",
  "pool_contract": "CABC...",
  "user": "GA5Z...",
  "token_in": {
    "contract": "CCW67...",
    "symbol": "USDC",
    "decimals": 7
  },
  "token_out": {
    "contract": "CAS3J...",
    "symbol": "XLM",
    "decimals": 7
  },
  "amount_in": "2000000000",
  "amount_in_display": "200.00",
  "amount_out": "1905000000",
  "amount_out_display": "190.50",
  "fee_amount": "6000000",
  "fee_display": "0.60"
}
```

## Mint Schema

```jsonc
{
  "type": "mint",
  "to": "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
  "amount": "10000000000000",
  "amount_display": "1000000.00",
  "token_contract": "CCW67...",
  "token_symbol": "USDC",
  "token_decimals": 7,
  "to_muxed_id": null
}
```

## Burn Schema

```jsonc
{
  "type": "burn",
  "from": "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
  "amount": "5000000000",
  "amount_display": "500.00",
  "token_contract": "CCW67...",
  "token_symbol": "USDC",
  "token_decimals": 7
}
```

## Common Fields Across Types

| Field            | Type         | Present in           | Description                                              |
| ---------------- | ------------ | -------------------- | -------------------------------------------------------- |
| `type`           | string       | all                  | interpretation type (`transfer`, `swap`, `mint`, `burn`) |
| `amount`         | string       | transfer, mint, burn | raw i128 amount as string                                |
| `amount_display` | string       | transfer, mint, burn | formatted amount with decimals                           |
| `token_contract` | string       | transfer, mint, burn | contract ID of the token                                 |
| `token_symbol`   | string\|null | all                  | token symbol from metadata                               |
| `token_decimals` | number\|null | all                  | token decimal places                                     |
| `dex`            | string       | swap                 | DEX identifier (`soroswap`, `aquarius`, `phoenix`)       |

## Query Examples

```sql
-- Find all USDC transfers over 1000
SELECT * FROM event_interpretations
WHERE interpretation_type = 'transfer'
  AND structured_data->>'token_symbol' = 'USDC'
  AND (structured_data->>'amount')::numeric > 10000000000;  -- 1000 * 10^7

-- Find all swaps on Soroswap
SELECT * FROM event_interpretations
WHERE interpretation_type = 'swap'
  AND structured_data->>'dex' = 'soroswap';

-- Find all mints for a specific token
SELECT * FROM event_interpretations
WHERE interpretation_type = 'mint'
  AND structured_data->>'token_contract' = 'CCW67...';
```

## Amounts as Strings

All `amount` fields are stored as strings (not numbers) to preserve i128 precision. JavaScript `Number` cannot represent values above 2^53. JSONB `numeric` could work but string is safer for cross-language compatibility. The `amount_display` field provides the human-formatted version.
