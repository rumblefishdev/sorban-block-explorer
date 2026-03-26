---
prefix: S
title: Research Synthesis - WASM Interface Extraction
status: mature
spawned_from: null
spawns: []
---

# Research Synthesis: Soroban WASM Interface Extraction

## Key Findings

### 1. Extraction is straightforward with `@stellar/stellar-sdk`

The SDK provides `contract.Spec.fromWasm(buffer)` which handles the entire pipeline: WASM parsing, custom section extraction (`contractspecv0`), and XDR deserialization. No additional dependencies needed. Pure JavaScript, Lambda-compatible.

### 2. Contract spec is in a WASM custom section

The spec is stored in a custom section named `contractspecv0` as a continuous XDR stream of `ScSpecEntry` values. Each entry is either a function, struct, union, enum, error enum, or event definition. Formally defined in SEP-0048.

### 3. SAC detection is definitive, not heuristic

SACs use `CONTRACT_EXECUTABLE_STELLAR_ASSET` as their executable type -- this is set at the protocol level and cannot be faked. Check the `ContractExecutable` type in the `LedgerEntry`, not the WASM interface. SACs have no WASM bytecode to parse.

### 4. Contract type classification is heuristic-based

Classification into 'token', 'dex', 'lending', 'nft', 'other' relies on matching extracted function names against known interface patterns. Token detection is most reliable (SEP-41 is well-defined). DEX/lending/NFT detection is best-effort. Default to 'other'.

### 5. `contractmetav0` provides contract name for search

A separate WASM custom section `contractmetav0` contains developer-set key-value metadata (description, version, author). This maps directly to `metadata.name` and thus `search_vector`. The SDK does not expose a high-level API for this section -- extraction requires using the internal `parseWasmCustomSections` utility + `ScMetaEntry` XDR deserialization.

### 6. Performance impact is negligible

WASM custom section parsing is O(n) with section skipping. XDR deserialization is linear. Typical contracts parse in < 10ms. Contract deployments are rare events (few per ledger). No performance concern for the Lambda.

## Recommendations for Implementation (Task 0054)

1. **Add `@stellar/stellar-sdk` dependency** to the Ledger Processor package.

2. **Two-path extraction logic**:

   - SAC path: Check `ContractExecutable` type. If `STELLAR_ASSET`, set `is_sac=true`, `contract_type='token'`, store known SAC interface in metadata.
   - WASM path: Extract WASM bytes, call `Spec.fromWasm()`, serialize to metadata JSONB, classify contract type.

3. **Store the WASM hash** in metadata for deduplication. Many contract instances share the same WASM code.

4. **Use the proposed JSONB structure** from `G-metadata-jsonb-structure.md`. Key features: human-readable type strings, separate types array, schema version for migrations.

5. **Classification confidence score**: Store alongside `contract_type` to signal how reliable the classification is. Token=high, DEX/lending=medium, NFT=low.

## Risks and Mitigations

| Risk                           | Likelihood | Mitigation                                                                                                             |
| ------------------------------ | ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| SDK API changes                | Low        | Pin `@stellar/stellar-sdk` version. The `Spec` class is stable.                                                        |
| Contracts without spec section | Low        | Older or non-standard contracts might lack the section. Handle gracefully -- store metadata without interface.         |
| Classification false positives | Medium     | Default to 'other'. Use confidence scores. Plan for manual override/known-contract registry.                           |
| Large WASM binaries            | Low        | Soroban enforces WASM size limits via network config. Parsing is O(n) and fast even for the largest allowed contracts. |

## Open Questions for Implementation

1. **WASM bytecode source**: During ledger processing, the WASM bytecode comes from `ContractCodeEntry` in `LedgerEntryChanges`. Need to confirm the exact XDR path to extract the raw bytes from the ledger close meta.

2. **Contract updates**: When a contract's WASM is updated (via `restore` or `update`), the metadata should be re-extracted. Need to detect WASM hash changes in `LedgerEntryChanges`.

3. **Known SAC interface**: Should the SAC interface metadata be hardcoded (since it's defined by the protocol) or extracted from a reference SAC?

4. **`parseWasmCustomSections` access**: This utility is internal to `@stellar/stellar-sdk` (not exported). Need to either vendor it (~45 lines pure JS) or import from internal path. Required for `contractmetav0` extraction.

## Sources

All findings are backed by sources archived in `../sources/`:

| Source                                                                                  | Backs                                                           |
| --------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| [SEP-0048](../sources/sep-0048-contract-interface-spec.md)                              | WASM section format, XDR types, stream encoding                 |
| [SEP-0041](../sources/sep-0041-token-interface.md)                                      | Token interface standard, SAC function set                      |
| [Stellar XDR: contract.x](../sources/stellar-xdr-contract.x)                            | `ContractExecutable` union, `CONTRACT_EXECUTABLE_STELLAR_ASSET` |
| [Stellar XDR: contract-meta.x](../sources/stellar-xdr-contract-meta.x)                  | `SCMetaV0`, `SCMetaEntry` format                                |
| [Stellar XDR: contract-spec.x](../sources/stellar-xdr-contract-spec.x)                  | `SCSpecEntry`, `SCSpecFunctionV0` XDR definitions               |
| [Stellar XDR: transaction.x](../sources/stellar-xdr-transaction.x)                      | `CreateContractArgs`, `ContractIDPreimage`                      |
| [Stellar Docs: SAC](../sources/stellar-docs-sac.md)                                     | SAC detection, admin functions                                  |
| [Stellar Docs: Token Interface](../sources/stellar-docs-token-interface.md)             | Full `TokenInterface` trait                                     |
| [Stellar Docs: Deploy SAC](../sources/stellar-docs-deploy-sac.md)                       | SAC deployment mechanism                                        |
| [Stellar Docs: WASM Metadata](../sources/stellar-docs-wasm-metadata.md)                 | `contractmetav0` section, `contractmeta!` macro                 |
| [Stellar Docs: Fully Typed Contracts](../sources/stellar-docs-fully-typed-contracts.md) | How WASM encodes interface types                                |
| [Stellar Docs: Build Your Own SDK](../sources/stellar-docs-build-your-own-sdk.md)       | Contract spec format for SDK builders                           |
| [SDK source: utils.ts](../sources/sdk-source-contract-utils.ts)                         | `parseWasmCustomSections` implementation                        |
| [SDK source: spec.ts](../sources/sdk-source-spec.ts)                                    | `Spec` class API                                                |
| [@stellar/stellar-sdk](../sources/npm-stellar-sdk.md)                                   | SDK package info                                                |
| [Blend mainnet contracts](../sources/blend-mainnet-contracts.json)                      | Lending + token contract IDs                                    |
| [Soroswap Core](../sources/soroswap-core-readme.md)                                     | DEX contract IDs                                                |
| [Aquarius Docs](../sources/aquarius-docs-prerequisites.md)                              | Aquarius AMM contract ID                                        |
| [FxDAO Addresses](../sources/fxdao-addresses.md)                                        | FxDAO Vaults contract ID                                        |
| [Stellar Docs: Oracle Providers](../sources/stellar-docs-oracle-providers.md)           | Reflector Oracle contract IDs                                   |
| [Litemint Contracts](../sources/litemint-soroban-contracts-readme.md)                   | NFT auction contracts                                           |
