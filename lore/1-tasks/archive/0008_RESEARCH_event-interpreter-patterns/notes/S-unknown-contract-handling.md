---
prefix: S
title: Unknown Contract Handling Strategy
status: mature
spawned_from: null
spawns: []
---

# Unknown Contract Handling Strategy

Synthesis: How should the Event Interpreter handle events from unknown contracts that match known topic patterns?

## Derived from

- [R-token-event-signatures.md](R-token-event-signatures.md) — SEP-41 standard format guarantees (topics[0] matching)
- [R-dex-swap-event-signatures.md](R-dex-swap-event-signatures.md) — DEX-specific event structures (Soroswap namespace prefix, Aqua "trade" topic, Phoenix per-field pattern)
- [SEP-41 Token Interface](../sources/sep-41-token-interface.md) — standard event definitions that enable unknown-contract interpretation

## The Problem

When the Event Interpreter sees an event with `topics[0] = Symbol("transfer")` from a contract that is not a known token, should it:

1. **Attempt interpretation** — treat any `"transfer"` event as a token transfer
2. **Skip unknown contracts** — only interpret events from known/registered contracts
3. **Interpret with confidence levels** — interpret but flag as low-confidence

## Decision: Interpret Standard Events, Skip DEX-Specific

### Standard token events (transfer, mint, burn): INTERPRET

- **Rationale**: SEP-41 defines a standard event format. Any compliant token contract emits events in the same structure. The `topics[0]` symbol is sufficient to identify and decode the event.
- **Risk**: A non-token contract could emit an event with `topics[0] = "transfer"` that has a different data structure. This is unlikely but possible.
- **Mitigation**: Wrap interpretation in try/catch. If data decoding fails, skip the event and log a warning. The pattern's `matches()` method should validate topic count and basic data shape before `interpret()` is called.

### DEX-specific events (swap, trade): REQUIRE KNOWN CONTRACT

- **Rationale**: DEX events have no universal standard. Each DEX has its own event structure. Matching on `topics[0] = "swap"` alone would match many unrelated contract events.
- **Implementation**: DEX pattern handlers include a set of known contract IDs (factory, router, pool contracts). The `matches()` method checks `contract_id` against this set.
- **Discovery**: New DEX contracts must be manually added to the known set. Future enhancement: auto-discover pool contracts by querying factory contracts.

### Summary Matrix

| Event Type        | Match Strategy                                      | Unknown Contracts                        |
| ----------------- | --------------------------------------------------- | ---------------------------------------- |
| `transfer`        | `topics[0] = "transfer"`                            | Attempt interpretation (standard format) |
| `mint`            | `topics[0] = "mint"`                                | Attempt interpretation (standard format) |
| `burn`            | `topics[0] = "burn"`                                | Attempt interpretation (standard format) |
| `swap` (Soroswap) | `topics[0] = "SoroswapPair"` + `topics[1] = "swap"` | Skip (namespace identifies protocol)     |
| `trade` (Aqua)    | `topics[0] = "trade"` + known `contract_id`         | Skip unknown contracts                   |
| `swap` (Phoenix)  | `topics[0] = "swap"` + known `contract_id`          | Skip unknown contracts                   |

## Error Handling

When interpretation fails for any event:

1. **Log the error** with event ID, contract ID, and failure reason
2. **Skip the event** — do not write a partial/broken interpretation
3. **Continue processing** — one failed event should not block the batch
4. **Advance the watermark** — the event was seen, just not interpretable

```typescript
for (const event of events) {
  try {
    const result = registry.interpret(event);
    if (result) interpretations.push(result);
  } catch (err) {
    logger.warn('Failed to interpret event', {
      eventId: event.id,
      contractId: event.contract_id,
      topics: event.topics,
      error: err.message,
    });
    // Continue — don't block the batch
  }
}
```

## Future Enhancement: Confidence Levels

If needed later, add a `confidence` field to `event_interpretations`:

- `high` — known contract + standard format (e.g., SAC transfer)
- `medium` — unknown contract but standard format (e.g., unknown SEP-41 token transfer)
- `low` — partial match or ambiguous

This allows the frontend to display or hide interpretations based on confidence. Not needed for Phase 1 but the schema supports adding this later.
