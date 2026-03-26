---
url: 'https://stellar.org/blog/developers/stellar-x-ray-protocol-25-upgrade-guide'
title: 'Stellar X-Ray, Protocol 25 Upgrade Guide'
fetched_date: 2026-03-26
task: '0002'
---

# Stellar X-Ray, Protocol 25 Upgrade Guide

## Overview

This guide helps businesses and developers prepare for X-Ray, Protocol 25. Additional details appear in the "announcement blog" and the Stellar Developer Discord's #protocol-next channel.

## Key Dates

The upgrade timeline includes three completed milestones:

- December 15, 2025: Stable releases became available
- January 7, 2026 at 2100 UTC: Testnet upgrade completed
- January 22, 2026 at 1700 UTC: Mainnet upgrade vote completed

The guide notes "there is more time than usual between the availability of the stable releases and the Testnet upgrade" due to the holiday schedule.

## Preparation Requirements

**SDK Users:** Update to the latest Stellar SDK version before January 7, 2026 (Testnet) and January 22, 2026 (Mainnet).

**Infrastructure Operators:** Install Protocol 25 releases for Stellar Core, Horizon, RPC, and Galexie. Ubuntu 20.04 support ended; operators must upgrade to Ubuntu 22.04 or 24.04. The build system now requires llvm-20.

**Validators:** Arm validators with the command: `upgrades?mode=set&upgradetime=2026-01-22T17:00:00Z&protocolversion=25`

## Technical Changes

X-Ray introduces "new host functions for BN254, and Poseidon and Poseidon2 permutation primitives" with no backwards incompatibility.

Go SDK users must migrate from the old `github.com/stellar/go` module to the centralized `github.com/stellar/go-stellar-sdk` repository, updating all import statements accordingly.

## Available Resources

Release links are provided for Stellar Core, Horizon, RPC, Galexie, and SDKs across multiple languages (Rust, JavaScript, Go, Java, Python, iOS, PHP, C#, Flutter, and Elixir).
