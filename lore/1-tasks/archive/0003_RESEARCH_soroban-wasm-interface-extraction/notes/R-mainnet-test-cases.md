---
prefix: R
title: Mainnet Test Cases for Contract Classification
status: mature
spawned_from: null
spawns: []
---

# Mainnet Test Cases for Contract Classification

## Token Contracts (SACs)

SACs are created for every classic Stellar asset that interacts with Soroban. These have deterministic contract IDs derived from the asset code + issuer.

| Asset         | Contract ID                                                | Notes                                                                                                 |
| ------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| XLM (native)  | `CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA` | Native asset SAC. No issuer.                                                                          |
| USDC (Circle) | `CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75` | Major stablecoin SAC. Source: [blend-mainnet-contracts.json](../sources/blend-mainnet-contracts.json) |

**SAC contract IDs are deterministic** -- can be computed from the asset:

```typescript
import { Asset, Networks } from '@stellar/stellar-sdk';
const contractId = Asset.native().contractId(Networks.PUBLIC);
```

## Token Contracts (Custom WASM, non-SAC)

| Name       | Contract ID                                                | Notes                  |
| ---------- | ---------------------------------------------------------- | ---------------------- |
| BLND Token | `CD25MNVTZDL4Y3XBCPCJXGXATV5WUHHOWMYFF4YBEGU5FCPGMYTVG5JY` | Blend governance token |

## DEX Contracts

| Name            | Contract ID                                                | Notes                        |
| --------------- | ---------------------------------------------------------- | ---------------------------- |
| SoroswapFactory | `CA4HEQTL2WPEUYKYKCDOHCDNIV4QHNJ7EL4J4NQ6VADP7SYHVRYZ7AW2` | Uniswap V2-style AMM factory |
| SoroswapRouter  | `CAG5LRYQ5JVEUI5TEID72EYOVX44TTUJT5BQR2J6J77FH65PCCFAJDDH` | Routes swaps through pairs   |
| Aquarius AMM    | `CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK` | Aquarius AMM protocol        |
| Comet DEX Pool  | `CAS3FL6TLZKDGGSISDBWGGPXT3NRR4DYTZD7YOD3HMYO6LTJUVGRVEAM` | Used by Blend                |

## Lending Contracts

| Name                  | Contract ID                                                | Notes                |
| --------------------- | ---------------------------------------------------------- | -------------------- |
| Blend Pool Factory V2 | `CDSYOAVXFY7SM5S64IZPPPYB4GVGGLMQVFREPSQQEZVIWXX5R23G4QSU` | Lending pool factory |
| Blend Backstop V2     | `CAQQR5SWBXKIGZKPBZDH3KM5GQ5GUTPKB7JAFCINLZBC5WXPJKRG3IM7` | Insurance module     |

## NFT Contracts

NFT activity on Soroban is limited. No established standard exists yet (SEP-50 is in discussion). OpenZeppelin has released Stellar NFT contracts (Base, Consecutive, Enumerable variants) but these are primarily for testnet/development use.

| Name             | Contract ID               | Notes                                                                                                                                       |
| ---------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Litemint Auction | (not publicly documented) | Auction/royalty contracts. Source: [github.com/litemint/litemint-soroban-contracts](https://github.com/litemint/litemint-soroban-contracts) |

**Gap**: No publicly documented NFT contract ID on mainnet. This is the weakest test category. The acceptance criterion says "at least one known mainnet contract per category **if available**" -- for NFT, it is not reliably available. For implementation testing, consider deploying an OpenZeppelin NFT contract on testnet.

## Other Notable Contracts

| Name                   | Contract ID                                                | Category        |
| ---------------------- | ---------------------------------------------------------- | --------------- |
| Reflector Oracle (DEX) | `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M` | Oracle (SEP-40) |
| FxDAO Vaults           | `CCUN4RXU5VNDHSF4S4RKV4ZJYMX2YWKOH6L4AKEKVNVDQ7HY5QIAO4UB` | CDP/Stablecoin  |

## Verification Approach

For each test case:

1. Fetch the WASM bytecode via Soroban RPC (`getLedgerEntries` with `ContractCode` key)
2. Run `Spec.fromWasm(wasmBuffer)` to extract the spec
3. Verify the classification heuristic produces the expected `contract_type`
4. Validate the metadata JSONB structure is well-formed

**Note**: SACs don't have WASM bytecode -- they need to be tested via the deployment mechanism detection path, not the WASM extraction path.

## Sources

- [Blend mainnet.contracts.json](../sources/blend-mainnet-contracts.json) -- Blend contract IDs: BLND, XLM, USDC, Comet, Pool Factory, Backstop (all verified)
- [Soroswap Core README](../sources/soroswap-core-readme.md) -- SoroswapFactory `CA4HEQTL2...` and SoroswapRouter `CAG5LRYQ5...` contract IDs
- [Aquarius Docs: Prerequisites](../sources/aquarius-docs-prerequisites.md) -- Aquarius AMM contract ID `CBQDHNBFBZYE...`
- [FxDAO Addresses](../sources/fxdao-addresses.md) -- FxDAO Vaults `CCUN4RXU5VND...` and all FxDAO asset contracts
- [Stellar Docs: Oracle Providers](../sources/stellar-docs-oracle-providers.md) -- Reflector Oracle contract IDs `CALI2BYU2JE6...`
- [Stellar Docs: Stellar Asset Contract](../sources/stellar-docs-sac.md) -- SAC deployment and deterministic contract ID derivation
- [Litemint Soroban Contracts README](../sources/litemint-soroban-contracts-readme.md) -- Litemint auction/royalty NFT contracts
- [Stellar Expert](https://stellar.expert/explorer/public) -- Soroban contract browser for verification (not archived, live tool)
