---
prefix: G
title: Metadata JSONB Structure for Contract Interface
status: mature
spawned_from: null
spawns: []
---

# Metadata JSONB Structure for Contract Interface

## Recommended `soroban_contracts.metadata` JSONB Structure

```jsonc
{
  // Contract display name (indexed by search_vector)
  "name": "SoroSwap Router",

  // Version of the metadata schema for future migrations
  "schema_version": 1,

  // Extracted contract interface
  "interface": {
    // Public functions
    "functions": [
      {
        "name": "transfer",
        "doc": "Transfer tokens from one address to another",
        "inputs": [
          {
            "name": "from",
            "type": "address",
            "doc": "Source address"
          },
          {
            "name": "to",
            "type": "address",
            "doc": "Destination address"
          },
          {
            "name": "amount",
            "type": "i128",
            "doc": "Amount to transfer"
          }
        ],
        "outputs": ["void"]
      }
    ],

    // User-defined types referenced by functions
    "types": [
      {
        "kind": "struct",
        "name": "SwapParams",
        "doc": "Parameters for a swap operation",
        "fields": [
          { "name": "token_in", "type": "address", "doc": "" },
          { "name": "token_out", "type": "address", "doc": "" },
          { "name": "amount_in", "type": "i128", "doc": "" }
        ]
      },
      {
        "kind": "enum",
        "name": "SwapType",
        "doc": "",
        "cases": [
          { "name": "ExactIn", "value": 0 },
          { "name": "ExactOut", "value": 1 }
        ]
      },
      {
        "kind": "union",
        "name": "SwapResult",
        "doc": "",
        "cases": [
          { "name": "Success", "type": ["i128"] },
          { "name": "Error", "type": [] }
        ]
      }
    ],

    // Error types
    "errors": [
      {
        "name": "InsufficientBalance",
        "value": 1,
        "doc": "Not enough tokens"
      }
    ],

    // Events emitted by the contract
    "events": [
      {
        "name": "transfer",
        "doc": "Emitted on token transfer",
        "topics": [
          { "name": "from", "type": "address", "doc": "" },
          { "name": "to", "type": "address", "doc": "" }
        ],
        "data": [{ "name": "amount", "type": "i128", "doc": "" }]
      }
    ]
  },

  // Summary stats for quick display
  "stats": {
    "function_count": 12,
    "type_count": 3,
    "event_count": 2,
    "error_count": 4
  },

  // WASM hash for deduplication and known-contract matching
  "wasm_hash": "a1b2c3...",

  // Classification confidence (0.0-1.0)
  "classification_confidence": 0.85
}
```

## Type Representation

Types are stored as **human-readable strings**, not raw XDR type codes. This supports frontend rendering without needing XDR knowledge.

### Type String Format

| Soroban Type | String Representation                                                      |
| ------------ | -------------------------------------------------------------------------- |
| Primitives   | `bool`, `void`, `u32`, `i32`, `u64`, `i64`, `u128`, `i128`, `u256`, `i256` |
| Strings      | `string`, `symbol`, `bytes`                                                |
| Addresses    | `address`                                                                  |
| Time         | `timepoint`, `duration`                                                    |
| Optional     | `Option<inner_type>`                                                       |
| Result       | `Result<ok_type, error_type>`                                              |
| Vector       | `Vec<element_type>`                                                        |
| Map          | `Map<key_type, value_type>`                                                |
| Tuple        | `(type1, type2, ...)`                                                      |
| Fixed bytes  | `BytesN<32>`                                                               |
| User-defined | The type name (e.g., `SwapParams`) -- references the `types` array         |

## API Endpoint Response

For `GET /contracts/:contract_id/interface`:

```jsonc
{
  "contract_id": "CABC...XYZ",
  "contract_type": "token",
  "is_sac": false,
  "interface": {
    // Same structure as metadata.interface above
    "functions": [...],
    "types": [...],
    "errors": [...],
    "events": [...]
  }
}
```

## Contract Metadata from `contractmetav0`

The `metadata.name` field (used by `search_vector`) should be populated from the `contractmetav0` WASM custom section when available. This section contains developer-set key-value pairs like `Description`. See [R-wasm-spec-extraction](R-wasm-spec-extraction.md) for extraction details.

Mapping from `contractmetav0` keys to JSONB fields:

| `contractmetav0` key      | JSONB field              | Notes                                                                                                                  |
| ------------------------- | ------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `Description` (or `desc`) | `metadata.name`          | Primary name/description for search. Source: [stellar-docs-wasm-metadata.md](../sources/stellar-docs-wasm-metadata.md) |
| All other keys            | `metadata.contract_meta` | Store as `{ key: val }` object. Build tooling may inject additional keys automatically.                                |

If `contractmetav0` is absent (many contracts don't set it), `metadata.name` falls back to `null`. The `search_vector` will then have no content for that contract.

## Design Decisions

1. **Flat type strings over nested type objects**: Types like `Vec<address>` are stored as strings rather than `{ kind: "vec", element: { kind: "address" } }`. This simplifies storage, reduces JSONB size, and the frontend can parse the string format for rendering. The string format is also more readable in raw DB queries.

2. **Types array separate from functions**: UDT (user-defined type) definitions are stored in a separate `types` array rather than inlined into each function parameter. This avoids duplication when multiple functions reference the same type.

3. **Schema version**: Included for future migration support. If the metadata format changes, the Ledger Processor can handle both old and new formats during transition.

4. **Stats object**: Pre-computed counts for quick display on contract listing pages without needing to count array lengths client-side.

5. **Wasm hash stored in metadata**: Enables deduplication -- many contracts may share the same WASM binary. The hash also enables a future "known contracts" registry lookup.

## Extraction Pipeline in Ledger Processor

```
LedgerEntryChange (contract type)
  |-- Check ContractExecutable type
  |   |-- CONTRACT_EXECUTABLE_STELLAR_ASSET
  |   |   -> is_sac=true, contract_type='token', metadata={name, known SAC interface}
  |   |-- CONTRACT_EXECUTABLE_WASM
  |       -> Lookup WASM bytecode from ContractCodeEntry
  |         -> Spec.fromWasm(wasmBuffer)
  |         -> Extract functions, types, errors, events
  |         -> Classify contract_type from function signatures
  |         -> Store in soroban_contracts.metadata
  |-- Upsert soroban_contracts row
```

## Sources

- [SEP-0048: Contract Interface Specification](../sources/sep-0048-contract-interface-spec.md) -- defines `SCSpecEntry` XDR types that map to JSONB `interface`
- [SEP-0041: Soroban Token Interface](../sources/sep-0041-token-interface.md) -- defines the standard SAC interface stored for `is_sac=true`
- [Stellar Docs: Stellar Asset Contract](../sources/stellar-docs-sac.md) -- SAC detection via `ContractExecutable` type
- [Stellar Docs: WASM Metadata](../sources/stellar-docs-wasm-metadata.md) -- `contractmetav0` section for `metadata.name`
- [Stellar XDR: contract-meta.x](../sources/stellar-xdr-contract-meta.x) -- `SCMetaV0` key/val pair format
- [Stellar XDR: contract.x](../sources/stellar-xdr-contract.x) -- `ContractExecutable` union for SAC detection pipeline
