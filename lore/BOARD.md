# Backlog Board

> **Auto-generated** — do not edit manually.
> Run `node tools/scripts/generate-lore-board.mjs` to regenerate.
> Last updated: 2026-03-25

## Overview

| Total  | 📋 Backlog | 🚧 Active | 🚫 Blocked | ✅ Done |
| :----: | :--------: | :-------: | :--------: | :-----: |
| **81** |     77     |     2     |     0      |    2    |

**Progress:** 2% complete · 2% in progress

## By Layer

| Layer             | Total | Backlog | Active | Blocked | Done |
| :---------------- | :---: | :-----: | :----: | :-----: | :--: |
| 🔬 Research       |   8   |    7    |   0    |    0    |  1   |
| 📦 Domain         |   6   |    6    |   0    |    0    |  0   |
| 🗄️ Database       |   8   |    8    |   0    |    0    |  0   |
| ⚙️ Backend API    |  16   |   16    |   0    |    0    |  0   |
| 🔄 Indexing       |   8   |    8    |   0    |    0    |  0   |
| 🖥️ Frontend       |  22   |   22    |   0    |    0    |  0   |
| ☁️ Infrastructure |  10   |   10    |   0    |    0    |  0   |
| 🔧 Tooling        |   3   |    0    |   2    |    0    |  1   |

## Tasks

### 🔬 Research

| ID                                                                                | Title                                                                       |    Status    | Priority  | Assignee |   Type   |
| :-------------------------------------------------------------------------------- | :-------------------------------------------------------------------------- | :----------: | :-------: | :------: | :------: |
| [0001](1-tasks/archive/0001_RESEARCH_galexie-captive-core-setup/README.md)        | Research: Galexie configuration, Captive Core setup, and output format      | ✅ completed |  🔴 high  | `filip`  | RESEARCH |
| [0002](1-tasks/backlog/0002_RESEARCH_ledgerclosemeta-xdr-parsing/README.md)       | Research: LedgerCloseMeta structure and @stellar/stellar-sdk XDR parsing    |  📋 backlog  |  🔴 high  |    —     | RESEARCH |
| [0003](1-tasks/backlog/0003_RESEARCH_soroban-wasm-interface-extraction/README.md) | Research: Soroban contract WASM interface extraction                        |  📋 backlog  |  🔴 high  |    —     | RESEARCH |
| [0004](1-tasks/backlog/0004_RESEARCH_nestjs-lambda-adapter/README.md)             | Research: NestJS on AWS Lambda (adapter, cold starts, connection lifecycle) |  📋 backlog  |  🔴 high  |    —     | RESEARCH |
| [0005](1-tasks/backlog/0005_RESEARCH_soroban-nft-patterns/README.md)              | Research: Soroban NFT ecosystem patterns and detection heuristics           |  📋 backlog  | 🟡 medium |    —     | RESEARCH |
| [0006](1-tasks/backlog/0006_RESEARCH_aws-cdk-nx-monorepo/README.md)               | Research: AWS CDK with Nx monorepo organization                             |  📋 backlog  | 🟡 medium |    —     | RESEARCH |
| [0007](1-tasks/backlog/0007_RESEARCH_drizzle-orm-postgres-partitioning/README.md) | Research: Drizzle ORM with PostgreSQL partitioning and advanced features    |  📋 backlog  |  🔴 high  |    —     | RESEARCH |
| [0008](1-tasks/backlog/0008_RESEARCH_event-interpreter-patterns/README.md)        | Research: Event Interpreter pattern matching and enrichment approach        |  📋 backlog  | 🟡 medium |    —     | RESEARCH |

### 📦 Domain

| ID                                                                          | Title                                                                  |   Status   | Priority  | Assignee |  Type   |
| :-------------------------------------------------------------------------- | :--------------------------------------------------------------------- | :--------: | :-------: | :------: | :-----: |
| [0009](1-tasks/backlog/0009_FEATURE_domain-types-ledger-transaction.md)     | Domain types: ledger and transaction models                            | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0010](1-tasks/backlog/0010_FEATURE_domain-types-soroban-models.md)         | Domain types: Soroban models (contract, invocation, event)             | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0011](1-tasks/backlog/0011_FEATURE_domain-types-token-account-nft.md)      | Domain types: token, account, NFT models                               | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0012](1-tasks/backlog/0012_FEATURE_domain-types-pool-search-pagination.md) | Domain types: liquidity pool, search, pagination, network stats models | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0013](1-tasks/backlog/0013_FEATURE_shared-xdr-scval-parsing-lib.md)        | Shared XDR/ScVal parsing utilities library                             | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0014](1-tasks/backlog/0014_FEATURE_shared-error-types-parse-error.md)      | Shared error types and parse_error handling                            | 📋 backlog | 🟡 medium |    —     | FEATURE |

### 🗄️ Database

| ID                                                                      | Title                                                                       |   Status   | Priority  | Assignee |  Type   |
| :---------------------------------------------------------------------- | :-------------------------------------------------------------------------- | :--------: | :-------: | :------: | :-----: |
| [0015](1-tasks/backlog/0015_FEATURE_drizzle-orm-config-connection.md)   | Drizzle ORM configuration and connection setup                              | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0016](1-tasks/backlog/0016_FEATURE_db-schema-ledgers-transactions.md)  | DB schema: ledgers and transactions tables                                  | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0017](1-tasks/backlog/0017_FEATURE_db-schema-operations.md)            | DB schema: operations table with transaction_id partitioning                | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0018](1-tasks/backlog/0018_FEATURE_db-schema-soroban-tables.md)        | DB schema: Soroban tables (contracts, invocations, events, interpretations) | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0019](1-tasks/backlog/0019_FEATURE_db-schema-tokens-accounts.md)       | DB schema: tokens and accounts tables                                       | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0020](1-tasks/backlog/0020_FEATURE_db-schema-nfts-pools-snapshots.md)  | DB schema: NFTs, liquidity pools, and pool snapshots tables                 | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0021](1-tasks/backlog/0021_FEATURE_db-migration-framework.md)          | Database migration framework                                                | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0022](1-tasks/backlog/0022_FEATURE_partition-management-automation.md) | Partition management automation                                             | 📋 backlog | 🟡 medium |    —     | FEATURE |

### ⚙️ Backend API

| ID                                                                              | Title                                                                  |   Status   | Priority  | Assignee |  Type   |
| :------------------------------------------------------------------------------ | :--------------------------------------------------------------------- | :--------: | :-------: | :------: | :-----: |
| [0023](1-tasks/backlog/0023_FEATURE_nestjs-api-bootstrap.md)                    | NestJS API bootstrap: Lambda adapter, app.module, env config           | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0024](1-tasks/backlog/0024_FEATURE_backend-pagination-query-parsing.md)        | Backend: cursor-based pagination helpers and query parsing             | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0025](1-tasks/backlog/0025_FEATURE_backend-validation-serialization-errors.md) | Backend: request validation, response serialization, error mapping     | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0026](1-tasks/backlog/0026_FEATURE_backend-network-module.md)                  | Backend: Network module (GET /network/stats)                           | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0027](1-tasks/backlog/0027_FEATURE_backend-transactions-module.md)             | Backend: Transactions module (list + detail + filters)                 | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0028](1-tasks/backlog/0028_FEATURE_backend-ledgers-module.md)                  | Backend: Ledgers module (list + detail + linked transactions)          | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0029](1-tasks/backlog/0029_FEATURE_backend-accounts-module.md)                 | Backend: Accounts module (detail + balances + transactions)            | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0030](1-tasks/backlog/0030_FEATURE_backend-tokens-module.md)                   | Backend: Tokens module (list + detail + transactions)                  | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0031](1-tasks/backlog/0031_FEATURE_backend-contracts-module.md)                | Backend: Contracts module (detail, interface, invocations, events)     | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0032](1-tasks/backlog/0032_FEATURE_backend-nfts-module.md)                     | Backend: NFTs module (list + detail + transfers)                       | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0033](1-tasks/backlog/0033_FEATURE_backend-liquidity-pools-module.md)          | Backend: Liquidity Pools module (list + detail + transactions + chart) | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0034](1-tasks/backlog/0034_FEATURE_backend-search-module.md)                   | Backend: Search module (unified search with query classification)      | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0035](1-tasks/backlog/0035_FEATURE_backend-xdr-decode-helpers.md)              | Backend: API-time XDR decode helpers for advanced transaction view     | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0036](1-tasks/backlog/0036_FEATURE_backend-inmemory-cache.md)                  | Backend: in-memory caching in Lambda execution environment             | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0037](1-tasks/backlog/0037_FEATURE_backend-api-gateway-caching.md)             | Backend: API Gateway response caching and cache-control headers        | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0038](1-tasks/backlog/0038_FEATURE_backend-openapi-docs-portal.md)             | Backend: OpenAPI spec generation and docs portal                       | 📋 backlog | 🟡 medium |    —     | FEATURE |

### 🔄 Indexing

| ID                                                                             | Title                                                                           |   Status   | Priority  | Assignee |  Type   |
| :----------------------------------------------------------------------------- | :------------------------------------------------------------------------------ | :--------: | :-------: | :------: | :-----: |
| [0060](1-tasks/backlog/0060_FEATURE_xdr-parsing-ledgerclosemeta.md)            | XDR parsing: LedgerCloseMeta deserialization, ledger and transaction extraction | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0061](1-tasks/backlog/0061_FEATURE_xdr-parsing-operations.md)                 | XDR parsing: operation extraction and INVOKE_HOST_FUNCTION details              | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0062](1-tasks/backlog/0062_FEATURE_xdr-parsing-soroban-events-invocations.md) | XDR parsing: Soroban events, invocation tree, contract interface extraction     | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0063](1-tasks/backlog/0063_FEATURE_xdr-parsing-ledger-entry-changes.md)       | XDR parsing: LedgerEntryChanges extraction                                      | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0064](1-tasks/backlog/0064_FEATURE_indexer-ledger-processor-handler.md)       | Indexer: Ledger Processor Lambda handler                                        | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0065](1-tasks/backlog/0065_FEATURE_indexer-idempotent-writes.md)              | Indexer: idempotent write logic and ledger-sequence watermarks                  | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0066](1-tasks/backlog/0066_FEATURE_indexer-historical-backfill.md)            | Indexer: historical backfill Fargate task                                       | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0067](1-tasks/backlog/0067_FEATURE_workers-event-interpreter.md)              | Workers: Event Interpreter Lambda                                               | 📋 backlog | 🟡 medium |    —     | FEATURE |

### 🖥️ Frontend

| ID                                                                           | Title                                                                  |   Status   | Priority  | Assignee |  Type   |
| :--------------------------------------------------------------------------- | :--------------------------------------------------------------------- | :--------: | :-------: | :------: | :-----: |
| [0039](1-tasks/backlog/0039_FEATURE_ui-layout-shell-header-nav.md)           | UI lib: layout shell, header, navigation, network indicator            | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0040](1-tasks/backlog/0040_FEATURE_ui-global-search-bar.md)                 | UI lib: global search bar component                                    | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0041](1-tasks/backlog/0041_FEATURE_ui-explorer-table-pagination.md)         | UI lib: explorer table, pagination controls, cursor pagination adapter | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0042](1-tasks/backlog/0042_FEATURE_ui-identifier-display-copy.md)           | UI lib: identifier display, copy button, linked identifiers            | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0043](1-tasks/backlog/0043_FEATURE_ui-badges-timestamps-polling.md)         | UI lib: badges, relative timestamps, polling indicator                 | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0044](1-tasks/backlog/0044_FEATURE_ui-loading-error-empty-states.md)        | UI lib: loading skeletons, error states, empty states                  | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0045](1-tasks/backlog/0045_FEATURE_ui-tabs-charts-tree-viz.md)              | UI lib: tabs, charts, and graph/tree visualization primitives          | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0046](1-tasks/backlog/0046_FEATURE_frontend-tanstack-query-api-client.md)   | Frontend: TanStack Query setup, API client, polling, env config        | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0047](1-tasks/backlog/0047_FEATURE_frontend-router-routes.md)               | Frontend: router setup, route definitions, param validation            | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0048](1-tasks/backlog/0048_FEATURE_frontend-home-page.md)                   | Frontend: Home page                                                    | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0049](1-tasks/backlog/0049_FEATURE_frontend-transactions-list.md)           | Frontend: Transactions list page                                       | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0050](1-tasks/backlog/0050_FEATURE_frontend-transaction-detail-normal.md)   | Frontend: Transaction detail -- normal mode                            | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0051](1-tasks/backlog/0051_FEATURE_frontend-transaction-detail-advanced.md) | Frontend: Transaction detail -- advanced mode                          | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0052](1-tasks/backlog/0052_FEATURE_frontend-ledgers-list-detail.md)         | Frontend: Ledgers list and detail pages                                | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0053](1-tasks/backlog/0053_FEATURE_frontend-account-detail.md)              | Frontend: Account detail page                                          | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0054](1-tasks/backlog/0054_FEATURE_frontend-tokens-list-detail.md)          | Frontend: Tokens list and detail pages                                 | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0055](1-tasks/backlog/0055_FEATURE_frontend-contract-detail.md)             | Frontend: Contract detail page                                         | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0056](1-tasks/backlog/0056_FEATURE_frontend-nfts-list-detail.md)            | Frontend: NFTs list and detail pages                                   | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0057](1-tasks/backlog/0057_FEATURE_frontend-liquidity-pools-list-detail.md) | Frontend: Liquidity Pools list and detail pages                        | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0058](1-tasks/backlog/0058_FEATURE_frontend-search-results.md)              | Frontend: Search results page                                          | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0059](1-tasks/backlog/0059_FEATURE_frontend-observability-accessibility.md) | Frontend: observability and accessibility baseline                     | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0077](1-tasks/backlog/0077_FEATURE_ui-mui-theme.md)                         | UI lib: MUI theme configuration and explorer-specific styling          | 📋 backlog |  🔴 high  |    —     | FEATURE |

### ☁️ Infrastructure

| ID                                                                 | Title                                                       |   Status   | Priority  | Assignee |  Type   |
| :----------------------------------------------------------------- | :---------------------------------------------------------- | :--------: | :-------: | :------: | :-----: |
| [0068](1-tasks/backlog/0068_FEATURE_cdk-vpc-networking.md)         | CDK: VPC, subnets, security groups, VPC endpoints           | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0069](1-tasks/backlog/0069_FEATURE_cdk-rds-s3-secrets.md)         | CDK: RDS PostgreSQL, RDS Proxy, S3 buckets, Secrets Manager | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0070](1-tasks/backlog/0070_FEATURE_cdk-lambda-api-gateway.md)     | CDK: Lambda functions + API Gateway                         | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0071](1-tasks/backlog/0071_FEATURE_cdk-ecs-fargate-galexie.md)    | CDK: ECS Fargate for Galexie live + backfill                | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0072](1-tasks/backlog/0072_FEATURE_cdk-cloudfront-waf-route53.md) | CDK: CloudFront, WAF, Route 53, S3 static hosting           | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0073](1-tasks/backlog/0073_FEATURE_cdk-cloudwatch-alarms.md)      | CDK: CloudWatch dashboards and alarms                       | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0074](1-tasks/backlog/0074_FEATURE_cdk-eventbridge-xray.md)       | CDK: EventBridge rules and X-Ray tracing                    | 📋 backlog | 🟡 medium |    —     | FEATURE |
| [0075](1-tasks/backlog/0075_FEATURE_cdk-environment-config.md)     | CDK: environment-specific configuration (dev/staging/prod)  | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0076](1-tasks/backlog/0076_FEATURE_cicd-github-actions.md)        | CI/CD pipeline: GitHub Actions workflows                    | 📋 backlog |  🔴 high  |    —     | FEATURE |
| [0078](1-tasks/backlog/0078_FEATURE_cdk-iam-ecr-nat.md)            | CDK: IAM roles, ECR repository, NAT Gateway                 | 📋 backlog |  🔴 high  |    —     | FEATURE |

### 🔧 Tooling

| ID                                                                          | Title                                                                       |    Status    | Priority |     Assignee     |  Type   |
| :-------------------------------------------------------------------------- | :-------------------------------------------------------------------------- | :----------: | :------: | :--------------: | :-----: |
| [0079](1-tasks/archive/0079_FEATURE_pr-and-branch-skills.md)                | Create /branch and /pr Claude Code skills for lore-aware git workflow       | ✅ completed | 🔴 high  | `stkrolikiewicz` | FEATURE |
| [0080](1-tasks/active/0080_BUG_deploy-board-duplicate-artifacts.md)         | Fix GitHub Pages deploy failing with duplicate artifacts                    |  🚧 active   | 🔴 high  | `stkrolikiewicz` |   BUG   |
| [0081](1-tasks/active/0081_BUG_fix-skills-structure-and-deploy-workflow.md) | Fix skill directory structure and deploy-board workflow duplicate artifacts |  🚧 active   | 🔴 high  | `stkrolikiewicz` |   BUG   |

## Dependency Graph

```mermaid
graph LR
  classDef research fill:#e1f5fe,stroke:#0288d1
  classDef domain fill:#f3e5f5,stroke:#7b1fa2
  classDef database fill:#fff3e0,stroke:#f57c00
  classDef backend fill:#e8f5e9,stroke:#388e3c
  classDef indexing fill:#fce4ec,stroke:#c62828
  classDef frontend fill:#e0f2f1,stroke:#00695c
  classDef infra fill:#efebe9,stroke:#4e342e
  T0001["0001: Research: Galexie configuration,..."]
  class T0001 research
  T0002["0002: Research: LedgerCloseMeta struct..."]
  class T0002 research
  T0003["0003: Research: Soroban contract WASM ..."]
  class T0003 research
  T0004["0004: Research: NestJS on AWS Lambda (..."]
  class T0004 research
  T0005["0005: Research: Soroban NFT ecosystem ..."]
  class T0005 research
  T0006["0006: Research: AWS CDK with Nx monore..."]
  class T0006 research
  T0007["0007: Research: Drizzle ORM with Postg..."]
  class T0007 research
  T0008["0008: Research: Event Interpreter patt..."]
  class T0008 research
  T0009["0009: Domain types: ledger and transac..."]
  class T0009 domain
  T0010["0010: Domain types: Soroban models (co..."]
  class T0010 domain
  T0011["0011: Domain types: token, account, NF..."]
  class T0011 domain
  T0012["0012: Domain types: liquidity pool, se..."]
  class T0012 domain
  T0013["0013: Shared XDR/ScVal parsing utiliti..."]
  class T0013 domain
  T0014["0014: Shared error types and parse_err..."]
  class T0014 domain
  T0015["0015: Drizzle ORM configuration and co..."]
  class T0015 database
  T0016["0016: DB schema: ledgers and transacti..."]
  class T0016 database
  T0017["0017: DB schema: operations table with..."]
  class T0017 database
  T0018["0018: DB schema: Soroban tables (contr..."]
  class T0018 database
  T0019["0019: DB schema: tokens and accounts t..."]
  class T0019 database
  T0020["0020: DB schema: NFTs, liquidity pools..."]
  class T0020 database
  T0021["0021: Database migration framework"]
  class T0021 database
  T0022["0022: Partition management automation"]
  class T0022 database
  T0023["0023: NestJS API bootstrap: Lambda ada..."]
  class T0023 backend
  T0024["0024: Backend: cursor-based pagination..."]
  class T0024 backend
  T0025["0025: Backend: request validation, res..."]
  class T0025 backend
  T0026["0026: Backend: Network module (GET /ne..."]
  class T0026 backend
  T0027["0027: Backend: Transactions module (li..."]
  class T0027 backend
  T0028["0028: Backend: Ledgers module (list + ..."]
  class T0028 backend
  T0029["0029: Backend: Accounts module (detail..."]
  class T0029 backend
  T0030["0030: Backend: Tokens module (list + d..."]
  class T0030 backend
  T0031["0031: Backend: Contracts module (detai..."]
  class T0031 backend
  T0032["0032: Backend: NFTs module (list + det..."]
  class T0032 backend
  T0033["0033: Backend: Liquidity Pools module ..."]
  class T0033 backend
  T0034["0034: Backend: Search module (unified ..."]
  class T0034 backend
  T0035["0035: Backend: API-time XDR decode hel..."]
  class T0035 backend
  T0036["0036: Backend: in-memory caching in La..."]
  class T0036 backend
  T0037["0037: Backend: API Gateway response ca..."]
  class T0037 backend
  T0038["0038: Backend: OpenAPI spec generation..."]
  class T0038 backend
  T0039["0039: UI lib: layout shell, header, na..."]
  class T0039 frontend
  T0050["0050: Frontend: Transaction detail -- ..."]
  class T0050 frontend
  T0051["0051: Frontend: Transaction detail -- ..."]
  class T0051 frontend
  T0052["0052: Frontend: Ledgers list and detai..."]
  class T0052 frontend
  T0053["0053: Frontend: Account detail page"]
  class T0053 frontend
  T0054["0054: Frontend: Tokens list and detail..."]
  class T0054 frontend
  T0055["0055: Frontend: Contract detail page"]
  class T0055 frontend
  T0056["0056: Frontend: NFTs list and detail p..."]
  class T0056 frontend
  T0058["0058: Frontend: Search results page"]
  class T0058 frontend
  T0059["0059: Frontend: observability and acce..."]
  class T0059 frontend
  T0060["0060: XDR parsing: LedgerCloseMeta des..."]
  class T0060 indexing
  T0061["0061: XDR parsing: operation extractio..."]
  class T0061 indexing
  T0062["0062: XDR parsing: Soroban events, inv..."]
  class T0062 indexing
  T0063["0063: XDR parsing: LedgerEntryChanges ..."]
  class T0063 indexing
  T0064["0064: Indexer: Ledger Processor Lambda..."]
  class T0064 indexing
  T0065["0065: Indexer: idempotent write logic ..."]
  class T0065 indexing
  T0066["0066: Indexer: historical backfill Far..."]
  class T0066 indexing
  T0067["0067: Workers: Event Interpreter Lambda"]
  class T0067 indexing
  T0068["0068: CDK: VPC, subnets, security grou..."]
  class T0068 infra
  T0069["0069: CDK: RDS PostgreSQL, RDS Proxy, ..."]
  class T0069 infra
  T0070["0070: CDK: Lambda functions + API Gateway"]
  class T0070 infra
  T0071["0071: CDK: ECS Fargate for Galexie liv..."]
  class T0071 infra
  T0076["0076: CI/CD pipeline: GitHub Actions w..."]
  class T0076 infra
  T0077["0077: UI lib: MUI theme configuration ..."]
  class T0077 frontend
  T0078["0078: CDK: IAM roles, ECR repository, ..."]
  class T0078 infra
  T0079["0079: Create /branch and /pr Claude Co..."]
  T0080["0080: Fix GitHub Pages deploy failing ..."]
  T0081["0081: Fix skill directory structure an..."]
  T0058 --> T0001
  T0063 --> T0001
  T0005 --> T0002
  T0052 --> T0002
  T0053 --> T0002
  T0054 --> T0002
  T0055 --> T0002
  T0054 --> T0003
  T0015 --> T0004
  T0007 --> T0004
  T0055 --> T0005
  T0012 --> T0005
  T0008 --> T0007
  T0009 --> T0007
  T0010 --> T0007
  T0011 --> T0007
  T0012 --> T0007
  T0059 --> T0008
  T0008 --> T0009
  T0010 --> T0010
  T0011 --> T0011
  T0012 --> T0011
  T0012 --> T0012
  T0002 --> T0013
  T0052 --> T0013
  T0053 --> T0013
  T0054 --> T0013
  T0055 --> T0013
  T0027 --> T0013
  T0013 --> T0014
  T0056 --> T0014
  T0017 --> T0014
  T0004 --> T0015
  T0007 --> T0015
  T0015 --> T0016
  T0009 --> T0016
  T0016 --> T0017
  T0009 --> T0017
  T0016 --> T0018
  T0010 --> T0018
  T0011 --> T0019
  T0012 --> T0020
  T0015 --> T0021
  T0068 --> T0021
  T0017 --> T0022
  T0018 --> T0022
  T0020 --> T0022
  T0004 --> T0023
  T0015 --> T0023
  T0023 --> T0024
  T0023 --> T0025
  T0014 --> T0025
  T0023 --> T0026
  T0023 --> T0027
  T0024 --> T0027
  T0025 --> T0027
  T0023 --> T0028
  T0024 --> T0028
  T0023 --> T0029
  T0024 --> T0029
  T0023 --> T0030
  T0024 --> T0030
  T0023 --> T0031
  T0024 --> T0031
  T0023 --> T0032
  T0024 --> T0032
  T0023 --> T0033
  T0024 --> T0033
  T0023 --> T0034
  T0024 --> T0034
  T0013 --> T0035
  T0023 --> T0035
  T0023 --> T0036
  T0023 --> T0037
  T0062 --> T0037
  T0023 --> T0038
  T0064 --> T0038
  T0077 --> T0039
  T0050 --> T0051
  T0002 --> T0060
  T0013 --> T0060
  T0016 --> T0060
  T0060 --> T0061
  T0017 --> T0061
  T0060 --> T0062
  T0018 --> T0062
  T0003 --> T0062
  T0060 --> T0063
  T0019 --> T0063
  T0020 --> T0063
  T0060 --> T0064
  T0061 --> T0064
  T0062 --> T0064
  T0063 --> T0064
  T0065 --> T0064
  T0064 --> T0065
  T0001 --> T0066
  T0064 --> T0066
  T0065 --> T0066
  T0008 --> T0067
  T0018 --> T0067
  T0006 --> T0068
  T0068 --> T0069
  T0068 --> T0070
  T0069 --> T0070
  T0068 --> T0071
  T0001 --> T0071
  T0021 --> T0076
  T0068 --> T0078
  T0079 --> T0080
  T0079 --> T0081
  T0080 --> T0081
```

**Legend:** 🔬 Research · 📦 Domain · 🗄️ Database · ⚙️ Backend API · 🔄 Indexing · 🖥️ Frontend · ☁️ Infrastructure · 🔧 Tooling | 🔴 High · 🟡 Medium · ⚪ Low
