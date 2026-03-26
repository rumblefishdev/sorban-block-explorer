---
url: 'https://developers.stellar.org/docs/networks/software-versions'
title: 'Stellar Software Versions'
fetched_date: 2026-03-26
task: '0002'
---

# Stellar Software Versions

## Overview

This page documents Soroban software releases and their corresponding changelogs across different protocol versions and network deployments (Mainnet, Testnet, Futurenet).

## Current Production Releases

**Protocol 25 (Mainnet, January 22, 2026)** represents the latest stable version. Key components include:

- Stellar Core: 25.0.0
- Smart Contract Rust SDK: 25.0.0
- Stellar CLI: 25.0.0
- Stellar RPC: v25.0.0
- Stellar Horizon: v25.0.0

New features in Protocol 25 include: "BN254 Elliptic Curve Operations" and "Poseidon/Poseidon2 Hash Functions."

## Recent Protocol Updates

**Protocol 24 (Mainnet, October 22, 2025)** was characterized as "a stability upgrade following Whisk (Protocol 23)."

**Protocol 23 "Whisk" (Mainnet, September 3, 2025)** introduced major features:

- Unified Events (CAP-67)
- State Archival (CAP-62, CAP-66)

**Protocol 22 (Mainnet, December 5, 2024)** added:

- Constructor support in Soroban
- BLS12-381 host functions

## Key SDK Versions

| Component | Protocol 25 | Protocol 24 | Protocol 23 |
| --------- | ----------- | ----------- | ----------- |
| Rust SDK  | 25.0.0      | 23.0.3      | 23.0.2      |
| JS SDK    | v14.4.3     | v14.3.0     | v14.1.1     |
| CLI       | 25.0.0      | 23.1.4      | v23.1.1     |

## Testing Networks

Both Testnet and Futurenet maintain synchronized versions with production releases. The documentation emphasizes: "Release candidates are software releases that are also released to the Testnet test network."

All networks share consistent passphrases established in 2015, with Futurenet using a separate passphrase from October 2022.
