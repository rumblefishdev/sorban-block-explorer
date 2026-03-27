---
prefix: G
title: Human-Readable Text Templates
status: mature
spawned_from: null
spawns: []
---

# Human-Readable Text Templates

Generated artifact: template approach and concrete examples for the `human_readable` column in `event_interpretations`.

## Derived from

- [R-dex-swap-event-signatures.md](R-dex-swap-event-signatures.md) — DEX names (Soroswap, Aquarius, Phoenix), event naming (Aqua uses "trade"), data structures
- [R-token-event-signatures.md](R-token-event-signatures.md) — transfer/mint/burn field structures (from, to, amount)
- Task README context — frontend shows display-ready summaries like "Swapped 100 USDC for 95.2 XLM on Soroswap"

## Approach: Template Strings with Extracted Values

Use simple template strings with placeholder substitution. No sophisticated formatting system needed — the templates are few, well-defined, and display-ready.

Each pattern handler produces a `human_readable` string by filling a template with values extracted from the event's topics and data.

### Why not alternatives?

| Approach           | Verdict         | Reason                                            |
| ------------------ | --------------- | ------------------------------------------------- |
| Template strings   | **Selected**    | Simple, predictable, display-ready                |
| i18n framework     | Over-engineered | English-only for now, 4 templates total           |
| Markdown/rich text | Wrong scope     | Frontend handles formatting; text should be plain |

## Templates by Pattern Type

### Transfer

```
Transferred {amount} {token_symbol} from {from_short} to {to_short}
```

Examples:

- `Transferred 100.00 USDC from GA5Z...KZVN to GBBD...7HFG`
- `Transferred 1,500.0000000 XLM from GCEX...ABCD to GDEF...1234`

### Swap (Soroswap)

```
Swapped {amount_in} {token_in_symbol} for {amount_out} {token_out_symbol} on {dex_name}
```

Examples:

- `Swapped 100.00 USDC for 95.20 XLM on Soroswap`
- `Swapped 500.0000000 XLM for 12.50 EURC on Soroswap`

### Swap (Aqua)

```
Traded {amount_in} {token_in_symbol} for {amount_out} {token_out_symbol} on Aquarius
```

Examples:

- `Traded 200.00 USDC for 190.50 XLM on Aquarius`

### Mint

```
Minted {amount} {token_symbol} to {to_short}
```

Examples:

- `Minted 1,000,000.00 USDC to GA5Z...KZVN`
- `Minted 50.0000000 XLM to GBBD...7HFG`

### Burn

```
Burned {amount} {token_symbol} from {from_short}
```

Examples:

- `Burned 500.00 USDC from GA5Z...KZVN`
- `Burned 100.0000000 XLM from GCEX...ABCD`

## Formatting Rules

### Addresses

Truncate to first 4 + last 4 characters: `GA5Z...KZVN`. The frontend transaction detail page links to full addresses, so the summary only needs to be scannable.

### Amounts

- Format with the token's decimal places (from token metadata)
- Use commas for thousands separator: `1,000,000.00`
- Strip trailing zeros after decimal only if all zeros: `100` not `100.00000000` but keep `100.50`

### Token symbols

- Use the token's `symbol()` from contract metadata (e.g., `USDC`, `XLM`, `EURC`)
- If symbol is unknown, use truncated contract ID: `CA5Z...KZVN`

### DEX names

Map known contract IDs to human names:

| Contract ID            | Display Name          |
| ---------------------- | --------------------- |
| Soroswap Router / Pair | `Soroswap`            |
| Aqua Router / Pool     | `Aquarius`            |
| Phoenix Pool           | `Phoenix`             |
| Unknown                | contract ID truncated |

## Implementation

```typescript
function formatTransfer(
  from: string,
  to: string,
  amount: bigint,
  token: TokenMeta
): string {
  return `Transferred ${formatAmount(amount, token.decimals)} ${
    token.symbol
  } from ${shortenAddr(from)} to ${shortenAddr(to)}`;
}

function formatSwap(
  amountIn: bigint,
  tokenIn: TokenMeta,
  amountOut: bigint,
  tokenOut: TokenMeta,
  dex: string
): string {
  return `Swapped ${formatAmount(amountIn, tokenIn.decimals)} ${
    tokenIn.symbol
  } for ${formatAmount(amountOut, tokenOut.decimals)} ${
    tokenOut.symbol
  } on ${dex}`;
}

function formatMint(to: string, amount: bigint, token: TokenMeta): string {
  return `Minted ${formatAmount(amount, token.decimals)} ${
    token.symbol
  } to ${shortenAddr(to)}`;
}

function formatBurn(from: string, amount: bigint, token: TokenMeta): string {
  return `Burned ${formatAmount(amount, token.decimals)} ${
    token.symbol
  } from ${shortenAddr(from)}`;
}

function shortenAddr(addr: string): string {
  return `${addr.slice(0, 4)}...${addr.slice(-4)}`;
}
```

## Token Metadata Dependency

The templates require token metadata (symbol, decimals). The Event Interpreter must either:

1. Query token contracts on-chain for metadata (expensive, but accurate)
2. Maintain a local cache/table of known token metadata (cheaper, needs population)
3. Use a fallback (truncated contract ID, raw amount) when metadata is unavailable

**Recommendation:** Local cache populated by the Ledger Processor or a separate metadata worker. Fallback to contract ID when metadata is missing.
