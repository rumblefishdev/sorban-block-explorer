---
id: '0003'
title: 'Research: Soroban contract WASM interface extraction'
type: RESEARCH
status: completed
related_adr: []
related_tasks: ['0054']
tags: [priority-high, effort-medium, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-25
    status: active
    who: fmazur
    note: 'Moved to active for research work'
  - date: 2026-03-26
    status: completed
    who: fmazur
    note: >
      Research completed. 8/8 acceptance criteria met. 5 research notes,
      22 archived sources, all verified (md5sum + content match).
      Key findings: use @stellar/stellar-sdk Spec.fromWasm(), SAC detection
      via CONTRACT_EXECUTABLE_STELLAR_ASSET, heuristic classification,
      contractmetav0 for search_vector. No blockers for task 0054.
---

# Research: Soroban contract WASM interface extraction

## Summary

Investigate how to extract public function signatures (names, parameter types, return types) from Soroban contract WASM bytecode at deployment time, including SAC detection, contract type classification, and the tools/libraries available for this extraction. This research must determine the feasibility and performance impact of performing WASM analysis during the Ledger Processor ingestion path.

## Status: Completed

## Context

The block explorer exposes contract interface data as a first-class feature. When a Soroban contract is deployed, the Ledger Processor must extract its public API so users can understand the contract without reading source code. This extraction is part of the broader XDR/protocol decode pipeline because it turns deployment-related protocol artifacts into stable explorer-facing contract metadata.

### Extraction Trigger and Source

Extraction happens at deployment time within the Ledger Processor Lambda. The source data comes from LedgerEntryChanges of contract type -- when a new contract entry appears in the ledger close, the associated WASM bytecode contains the contract specification. The Ledger Processor must identify contract deployment entries, retrieve the WASM bytecode, and parse out the public interface.

### Storage Target

Extracted interface data is stored in the `soroban_contracts.metadata` JSONB column. This column holds explorer metadata including optional extracted interface signatures. The data is served via `GET /contracts/:contract_id/interface`, which returns public function signatures with parameter names and types.

### SAC Detection

Stellar Asset Contracts (SACs) are a special category of Soroban contracts that wrap classic Stellar assets. The schema includes an `is_sac` boolean on `soroban_contracts`. SACs have a known standard interface, so detection may be possible by comparing the extracted interface against the known SAC function set, or by examining the deployment mechanism itself.

### Contract Type Classification

The schema includes a `contract_type` column with allowed values: 'token', 'dex', 'lending', 'nft', 'other'. The research must determine how to classify contracts into these categories from WASM analysis alone. This may involve matching against known interface patterns (e.g., a token contract implements mint/transfer/balance functions), examining event emission patterns, or combining deployment-time WASM analysis with runtime event observation.

### Search Integration

The `soroban_contracts` table includes a `search_vector` column (tsvector GENERATED ALWAYS AS) that indexes the `metadata->>'name'` field. This means contract metadata quality directly affects search discoverability.

## Research Questions

- What tools exist for parsing Soroban contract WASM to extract the contract specification? Evaluate: `wasmparser` (Rust/WASM), `@aspect-build/wasm-parser` or similar JS WASM parsers, and `@stellar/stellar-sdk` contract spec utilities.
- Where exactly in the WASM binary is the Soroban contract spec stored? Is it a custom section, and what is its format?
- What does the extracted interface look like structurally -- function name, parameter names, parameter types, return type? What type system does Soroban use for its contract spec?
- How can SAC contracts be reliably detected? Is it through the deployment mechanism (specific deployer, specific WASM hash), or through interface matching against the known SAC standard?
- How should `contract_type` classification work? Can 'token', 'dex', 'lending', 'nft' be determined from the WASM interface alone, or does it require additional heuristics from event patterns or known contract registries?
- What is the performance impact of WASM parsing during the Ledger Processor Lambda execution? Contract deployments are relatively infrequent compared to invocations, but the Lambda has a ~10 second budget per ledger.
- Are there known Soroban contracts on mainnet that can serve as test cases for each contract type?
- How should the metadata JSONB structure represent the extracted interface for the `GET /contracts/:contract_id/interface` endpoint?

## Acceptance Criteria

- [x] Documented method for extracting contract spec from WASM bytecode, with recommended library/tool
- [x] TypeScript-compatible approach confirmed (must run in Node.js Lambda environment)
- [x] Interface data structure defined: function signatures with names, parameter types, return types
- [x] SAC detection strategy documented with reliability assessment
- [x] Contract type classification approach documented with heuristics for each type
- [x] Performance impact assessment for WASM parsing during ingestion
- [x] Recommended `metadata` JSONB structure for storing extracted interface
- [x] Test cases identified: at least one known mainnet contract per category if available

## Research Notes

| Note                                                                              | Title                                                                         |
| --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| [R-wasm-spec-extraction](notes/R-wasm-spec-extraction.md)                         | WASM parsing tools, SDK API, extraction pipeline, performance, contractmetav0 |
| [R-sac-detection-and-classification](notes/R-sac-detection-and-classification.md) | SAC detection, SEP-41 interface, contract type heuristics                     |
| [R-mainnet-test-cases](notes/R-mainnet-test-cases.md)                             | Known mainnet contracts per category with verified IDs                        |
| [G-metadata-jsonb-structure](notes/G-metadata-jsonb-structure.md)                 | Recommended JSONB schema for `soroban_contracts.metadata`                     |
| [S-research-synthesis](notes/S-research-synthesis.md)                             | Key findings, recommendations, risks, open questions                          |

## Implementation Notes

- 5 research notes produced (3x R-, 1x G-, 1x S-)
- 22 sources archived in `sources/` -- 12 raw GitHub files (10 byte-identical, 2 cosmetic diff), 10 web pages (all key data points verified)
- Sources include: SEP-0048, SEP-0041, 4 Stellar XDR definitions, 3 SDK source files, 7 Stellar docs pages, Blend/Soroswap/Aquarius/FxDAO/Litemint/Oracle references

## Design Decisions

### From Plan

1. **Use `@stellar/stellar-sdk` for extraction**: Task spec asked to evaluate multiple tools. SDK's `contract.Spec.fromWasm()` handles the full pipeline with no additional dependencies.

2. **SAC detection via `ContractExecutable` type**: Task asked whether to use deployer, WASM hash, or interface matching. `CONTRACT_EXECUTABLE_STELLAR_ASSET` is protocol-level, 100% reliable.

### Emerged

3. **Human-readable type strings over nested type objects**: Types like `Vec<address>` stored as strings, not deeply nested JSON. Simpler storage, smaller JSONB, frontend can parse if needed.

4. **Heuristic classification with confidence scores**: Contract type classification is inherently fuzzy. Added `classification_confidence` field to signal reliability per type.

5. **Two-path extraction logic**: SACs and WASM contracts require completely different extraction paths. SACs have no WASM bytecode -- interface is known from the protocol.

6. **`contractmetav0` for `metadata.name`**: Not in original task scope. Discovered during research that this WASM section provides contract descriptions for `search_vector`. Added extraction approach.

7. **`@aspect-build/wasm-parser` does not exist**: Task spec listed it as candidate -- verified it's not a real npm package.

## Issues Encountered

- **`parseWasmCustomSections` not in public API**: Exported from internal `utils.ts` but not re-exported from `@stellar/stellar-sdk/contract`. Implementation will need to either vendor it (~45 lines) or use internal import path.

- **No NFT contract ID on mainnet**: Litemint has auction contracts but no publicly documented contract ID. NFT is the weakest test category. Recommended testnet deployment for testing.

## Future Work

- Implement extraction in Ledger Processor (task 0054)
- Known contract registry for exact classification of popular contracts
- Event-based classification to strengthen heuristic confidence at runtime

## Notes

- Contract deployments are infrequent relative to invocations and events, so the performance budget for WASM analysis is more generous than for per-transaction parsing.
- The frontend contract page renders the interface as a readable function list, not a raw ABI dump. The extracted data structure must support human-friendly rendering.
- The `soroban_contracts` table uses `contract_id` (VARCHAR 56) as the primary key -- this is the public stable identifier for all contract lookups.
