---
prefix: S
title: Pattern Registry Design Decision
status: mature
spawned_from: null
spawns: []
---

# Pattern Registry Design

Synthesis of research into how to structure the pattern matching registry for the Event Interpreter.

## Sources

- [Blockscout: Event Log Decoding](../sources/blockscout-event-decoding.md) — `Explorer.Chain.Log` schema and decoding logic
- Ethereum 4byte.directory for event signature databases: [4byte.directory](https://www.4byte.directory/) (external, live registry)
- Etherscan event log decoder approach: [docs.etherscan.io/api-endpoints/logs](https://docs.etherscan.io/api-endpoints/logs) (external API docs)
- [Soroban event model](../sources/stellar-docs-events.md) — ContractEvent XDR structure

## Decision: Strategy Pattern with Static Registry

### Why not alternatives?

| Approach                                   | Verdict         | Reason                                                                       |
| ------------------------------------------ | --------------- | ---------------------------------------------------------------------------- |
| Static map (object literal)                | Too simple      | No match/interpret separation, no versioning                                 |
| Configuration-driven (JSON/YAML)           | Too limited     | Complex patterns (multi-event Phoenix) can't be expressed declaratively      |
| Plugin system (dynamic loading)            | Over-engineered | Only 4 patterns initially, all known at build time                           |
| **Strategy pattern + static registration** | **Selected**    | Clean interface, independently testable, extensible without over-engineering |

### How blockchain explorers do it

**Etherscan**: ABI-based approach. Users submit verified contract ABIs. Event signatures (Keccak-256 hash) are matched against a signature database. Interpretation is mechanical (ABI decoding), not semantic.

**Blockscout**: Combination of known ABI signatures from verified contracts, static list of common ERC-20/ERC-721 signatures, and user-submitted ABIs. Decodes parameters but does **not** produce human-readable semantic summaries.

**Key insight**: Both are signature-first — identify event type by signature, then decode parameters. Semantic interpretation ("Alice swapped 100 USDC for 0.05 ETH") is a layer on top that neither fully implements. This is where our Event Interpreter adds value.

### Soroban-specific considerations

Soroban differs from EVM:

- No ABI in the Ethereum sense
- First topic is conventionally the event name (`Symbol("transfer")`)
- Emerging standards (SEP-41) but no universal registry
- Match strategy: `(contract_id, topics[0])` or just `topics[0]` for standard events

### Recommended Design

```typescript
interface EventPattern {
  readonly name: string; // e.g. "transfer", "soroswap_swap"
  readonly version: number; // for tracking pattern evolution

  // Fast check: can this pattern handle the event?
  matches(event: SorobanEvent): boolean;

  // Full interpretation — only called if matches() returned true
  interpret(event: SorobanEvent): EventInterpretation;
}

interface EventInterpretation {
  interpretationType: string; // 'swap', 'transfer', 'mint', 'burn'
  humanReadable: string; // display-ready text
  structuredData: Record<string, unknown>; // queryable JSONB
}

class PatternRegistry {
  private patterns: EventPattern[] = [];

  register(pattern: EventPattern): void {
    this.patterns.push(pattern);
  }

  interpret(event: SorobanEvent): {
    pattern: string;
    version: number;
    interpretation: EventInterpretation;
  } | null {
    for (const p of this.patterns) {
      if (p.matches(event)) {
        return {
          pattern: p.name,
          version: p.version,
          interpretation: p.interpret(event),
        };
      }
    }
    return null;
  }
}
```

### Registration (static, at module load)

```typescript
const registry = new PatternRegistry();
registry.register(new TransferPattern()); // SEP-41 transfer
registry.register(new MintPattern()); // SEP-41 mint
registry.register(new BurnPattern()); // SEP-41 burn
registry.register(new SoroswapSwapPattern()); // Soroswap pair swap
registry.register(new AquaTradePattern()); // Aqua pool trade
```

### Pattern ordering

Patterns are evaluated in registration order. More specific patterns should be registered first (e.g., `SoroswapSwapPattern` before a generic `SwapPattern`). Token patterns (transfer/mint/burn) match on `topics[0]` only; DEX patterns match on `topics[0]` + optionally `contract_id`.

### Extensibility

Adding a new pattern:

1. Implement `EventPattern` interface
2. Register in the static list
3. Bump `version` for existing patterns when logic changes
4. Deploy — no config changes, no database updates

### Future evolution

If dynamic patterns are needed later (e.g., user-submitted contract ABIs), add a `DynamicAbiPattern` that reads from the database. The registry interface does not change.
