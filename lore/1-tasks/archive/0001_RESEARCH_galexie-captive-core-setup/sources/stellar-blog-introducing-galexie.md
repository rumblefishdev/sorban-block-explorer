---
url: 'https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data'
title: 'Introducing Galexie: Efficiently Extract and Store Stellar Data'
fetched_date: 2026-03-25
task_id: '0001'
---

# Introducing Galexie: Efficiently Extract and Store Stellar Data

## Overview

Galexie represents the inaugural component of Stellar's Composable Data Platform, functioning as a lightweight extraction tool for ledger data from the Stellar network. The platform addresses a fundamental challenge: developers previously faced "limited tools available to read and save data directly from the Stellar network."

## Core Functionality

Galexie performs several key operations:

- **Data Extraction**: Retrieves transaction metadata from Stellar's ledger
- **Compression**: Employs Zstandard compression for optimized storage
- **Storage Integration**: Connects to cloud-based data lakes (beginning with Google Cloud Storage)
- **Operation Modes**: Supports both fixed ledger range uploads and continuous streaming of new ledgers

## Architecture Principles

The design emphasizes:

- **Simplicity**: Focused on efficient ledger data export
- **Portability**: Flat data writing without complex indexing requirements
- **Decentralization**: Encourages individual data ownership
- **Extensibility**: Facilitates support for additional storage systems
- **Resiliency**: Enables interrupted operations to resume without conflicts

## Data Format and Storage Strategy

Galexie exports data in XDR format, maintaining compatibility with existing systems like Horizon and Hubble. The platform bundles multiple ledgers per file — experimentation showed that "bulk downloading data for the same ledger range is twice as fast when files contain a bundle of 64 ledgers."

## Performance Metrics

Key operational observations:

| Metric                                                    | Value        |
| --------------------------------------------------------- | ------------ |
| Single instance full history backfill                     | ~150 days    |
| Parallel processing (40+ instances) full history backfill | under 5 days |
| Total pubnet data lake size                               | ~3 TB        |
| Monthly operational cost (compute + storage)              | ~$160        |
| Cost for 10-year history backfill with parallel instances | ~$600        |
