---
url: 'https://developers.stellar.org/docs/data/indexers/build-your-own/galexie'
title: 'Galexie Overview | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
---

# Galexie Overview

## What is Galexie?

Galexie is a tool for extracting, processing, and exporting Stellar ledger metadata to external storage, creating a data lake of pre-processed ledger information. It represents the foundational layer of the Composable Data Pipeline (CDP), enabling access to raw Stellar ledger metadata. Additional context on CDP's applications is available in a [related blog post](https://stellar.org/blog/developers/composable-data-platform).

## Key Features

Galexie provides streamlined ledger metadata export through a user-friendly interface. Notable capabilities include:

- Exporting Stellar ledger metadata to cloud storage
- Configurable export ranges — either specified ledger ranges or continuous streaming of new ledgers
- Exporting metadata in XDR, Stellar Core's native format
- Data compression before export to optimize storage efficiency

## Why XDR Format?

XDR — Stellar Core's native format — "enables Galexie to preserve full transaction metadata, ensuring data integrity while keeping storage efficient." This format maintains compatibility with all Stellar components. See the [XDR documentation](https://developers.stellar.org/docs/learn/fundamentals/data-format/xdr) for additional details.

## Why Run Galexie?

Galexie allows you to create independently managed copies of Stellar ledger metadata. The tool continuously synchronizes your data lake with current ledger information, eliminating manual data ingestion tasks and allowing focus on building custom applications.

## Use Cases for the Data Lake

Cloud-stored data becomes accessible for integration with contemporary data processing and analytics platforms. Pre-processed ledger data supports multiple use cases:

- **Analytics Tools**: Examine trends across time periods
- **Audit Applications**: Access historical transaction records for compliance and verification
- **Monitoring Systems**: Build network metric tracking solutions
