# Database Schema Audit

Full audit of all 12 tables in the Soroban Block Explorer database.
For each table: column descriptions, all write paths (INSERT/UPDATE/UPSERT),
and post-insert mutability.

Generated: 2026-04-15

---

## Table of Contents

1. [ledgers](#ledgers)
2. [transactions](#transactions)
3. [operations](#operations)
4. [soroban_contracts](#soroban_contracts)
5. [soroban_events](#soroban_events)
6. [soroban_invocations](#soroban_invocations)
7. [accounts](#accounts)
8. [tokens](#tokens)
9. [nfts](#nfts)
10. [liquidity_pools](#liquidity_pools)
11. [liquidity_pool_snapshots](#liquidity_pool_snapshots)
12. [wasm_interface_metadata](#wasm_interface_metadata)

---

## `ledgers`

### Description

Stores one row per Stellar ledger (block) that has been indexed. Serves as the parent table for `transactions` (FK on `ledgers.sequence`) and is referenced by other tables (`soroban_contracts`, `accounts`, `liquidity_pools`) for temporal anchoring.

### Columns

| Column              | Type                          | Description                                                           |
| ------------------- | ----------------------------- | --------------------------------------------------------------------- |
| `sequence`          | `BIGINT PRIMARY KEY`          | Ledger sequence number. Unique monotonically-increasing identifier.   |
| `hash`              | `VARCHAR(64) NOT NULL UNIQUE` | SHA-256 hash of the `LedgerHeaderHistoryEntry` XDR, hex-encoded.      |
| `closed_at`         | `TIMESTAMPTZ NOT NULL`        | Timestamp when the ledger was closed by the network (consensus time). |
| `protocol_version`  | `INTEGER NOT NULL`            | Stellar protocol version in effect at this ledger.                    |
| `transaction_count` | `INTEGER NOT NULL`            | Number of transactions included in this ledger.                       |
| `base_fee`          | `BIGINT NOT NULL`             | Network base fee in stroops (1 stroop = 0.0000001 XLM).               |

### Indexes

| Index           | Columns          |
| --------------- | ---------------- |
| Primary key     | `sequence`       |
| Unique          | `hash`           |
| `idx_closed_at` | `closed_at DESC` |

### Write Paths

| #   | Function        | File:Line                            | SQL                                            | Trigger                                                          | Columns       |
| --- | --------------- | ------------------------------------ | ---------------------------------------------- | ---------------------------------------------------------------- | ------------- |
| 1   | `insert_ledger` | `crates/db/src/persistence.rs:22-42` | `INSERT ... ON CONFLICT (sequence) DO NOTHING` | `persist_ledger()` step 1, called per ledger from Lambda handler | All 6 columns |

### Post-Insert Mutability

**Fully immutable.** `ON CONFLICT DO NOTHING` — no columns are ever updated. No UPDATE statements exist for this table.

---

## `transactions`

### Description

Stores one row per Stellar transaction. Captures the full transaction envelope, result, and metadata in both structured columns and raw XDR blobs. Keyed by surrogate `BIGSERIAL` id and uniquely constrained on the transaction hash.

### Columns

| Column            | Type                          | Description                                                                                                             |
| ----------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `id`              | `BIGSERIAL PRIMARY KEY`       | Auto-incrementing surrogate key. FK target for `operations`, `soroban_events`, `soroban_invocations`.                   |
| `hash`            | `VARCHAR(64) NOT NULL UNIQUE` | SHA-256 hash of the TransactionEnvelope, hex-encoded. Dedup key.                                                        |
| `ledger_sequence` | `BIGINT NOT NULL`             | Parent ledger sequence. FK to `ledgers(sequence)` (no CASCADE — deleting a ledger is blocked while transactions exist). |
| `source_account`  | `VARCHAR(56) NOT NULL`        | Transaction source account (G... or M... address).                                                                      |
| `fee_charged`     | `BIGINT NOT NULL`             | Actual fee charged in stroops.                                                                                          |
| `successful`      | `BOOLEAN NOT NULL`            | Whether the transaction succeeded.                                                                                      |
| `result_code`     | `VARCHAR(50)`                 | Transaction result code string (e.g. `txSUCCESS`). Nullable.                                                            |
| `envelope_xdr`    | `TEXT NOT NULL`               | Full transaction envelope, base64-encoded XDR.                                                                          |
| `result_xdr`      | `TEXT NOT NULL`               | Transaction result, base64-encoded XDR.                                                                                 |
| `result_meta_xdr` | `TEXT`                        | Transaction result metadata, base64-encoded XDR. Nullable.                                                              |
| `memo_type`       | `VARCHAR(20)`                 | Memo type: `"text"`, `"id"`, `"hash"`, `"return"`, or NULL.                                                             |
| `memo`            | `TEXT`                        | Memo value. Nullable.                                                                                                   |
| `created_at`      | `TIMESTAMPTZ NOT NULL`        | Timestamp from parent ledger's close time.                                                                              |
| `parse_error`     | `BOOLEAN`                     | True if XDR parsing failed for this transaction. Nullable.                                                              |
| `operation_tree`  | `JSONB`                       | Pre-computed Soroban invocation call tree. Populated asynchronously. NULL for non-Soroban.                              |

### Indexes

| Index        | Columns                             |
| ------------ | ----------------------------------- |
| Primary key  | `id`                                |
| Unique       | `hash`                              |
| `idx_source` | `(source_account, created_at DESC)` |
| `idx_ledger` | `ledger_sequence`                   |

### Write Paths

| #   | Function                       | File:Line                             | SQL                                                                                   | Trigger                                                  | Columns Affected                                                                    |
| --- | ------------------------------ | ------------------------------------- | ------------------------------------------------------------------------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| 1   | `insert_transactions_batch`    | `crates/db/src/persistence.rs:56-129` | `INSERT ... ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash RETURNING hash, id` | `persist_ledger()` step 2                                | All 15 columns on INSERT. On conflict: no-op self-assignment (to get RETURNING id). |
| 2   | `update_operation_trees_batch` | `crates/db/src/soroban.rs:44-65`      | `UPDATE transactions SET operation_tree = ... WHERE id = ...`                         | `persist_ledger()` step 6, only for Soroban transactions | `operation_tree` only                                                               |

### Post-Insert Mutability

Only **`operation_tree`** is updated after initial insert. All other columns are write-once. The ON CONFLICT clause is a no-op used solely for RETURNING.

### Child Tables

- `operations.transaction_id` → `ON DELETE CASCADE`
- `soroban_events.transaction_id` → `ON DELETE CASCADE`
- `soroban_invocations.transaction_id` → `ON DELETE CASCADE`

---

## `operations`

### Description

Stores individual Stellar operations extracted from transactions. Each
transaction contains one or more operations. Partitioned by `RANGE (created_at)`
monthly (per ADR 0027).

### Columns

| Column              | Type                            | Description                                                                                   |
| ------------------- | ------------------------------- | --------------------------------------------------------------------------------------------- |
| `id`                | `BIGSERIAL NOT NULL`            | Auto-generated surrogate. Part of composite PK `(id, created_at)`.                            |
| `transaction_id`    | `BIGINT NOT NULL`               | Parent transaction. FK `(transaction_id, created_at) → transactions(id, created_at)` CASCADE. |
| `application_order` | `SMALLINT NOT NULL`             | Zero-based index of this operation within its parent transaction.                             |
| `type`              | `SMALLINT NOT NULL`             | Operation type (ADR 0031 enum; label via `op_type_name`). CK `BETWEEN 0 AND 127`.             |
| `source_id`         | `BIGINT` FK `accounts`          | Operation source account (nullable — inherited from transaction if not overridden).           |
| `destination_id`    | `BIGINT` FK `accounts`          | Destination account for payment-like ops. Nullable.                                           |
| `contract_id`       | `BIGINT` FK `soroban_contracts` | Contract touched by the op (ADR 0030 surrogate). Nullable.                                    |
| `asset_code`        | `VARCHAR(12)`                   | Asset code for asset-denominated ops. Nullable.                                               |
| `asset_issuer_id`   | `BIGINT` FK `accounts`          | Asset issuer account. Nullable.                                                               |
| `pool_id`           | `BYTEA`                         | Liquidity pool 32-byte id (CK `octet_length = 32`). FK attached in migration 0006.            |
| `transfer_amount`   | `NUMERIC(28,7)`                 | Amount for transfer-shaped ops. Nullable.                                                     |
| `ledger_sequence`   | `BIGINT NOT NULL`               | Parent ledger sequence.                                                                       |
| `created_at`        | `TIMESTAMPTZ NOT NULL`          | Partition key. Inherited from parent transaction.                                             |

### Partitioning

- **Method:** `PARTITION BY RANGE (created_at)`, monthly (per ADR 0027).
- **Naming:** `operations_y{YYYY}m{MM}` (e.g. `operations_y2026m04`).
- **Dynamic:** `db-partition-mgmt` Lambda creates future monthly partitions
  daily (covers current month + 3 months ahead).

### Indexes & Constraints

| Name                  | Type        | Columns                                                                         |
| --------------------- | ----------- | ------------------------------------------------------------------------------- |
| PK                    | Primary key | `(id, created_at)`                                                              |
| FK                    | Foreign key | `(transaction_id, created_at) → transactions(id, created_at)` CASCADE           |
| `idx_ops_tx`          | B-tree      | `transaction_id`                                                                |
| `idx_ops_type`        | B-tree      | `(type, created_at DESC)`                                                       |
| `idx_ops_contract`    | B-tree      | `(contract_id, created_at DESC)` WHERE `contract_id IS NOT NULL`                |
| `idx_ops_asset`       | B-tree      | `(asset_code, asset_issuer_id, created_at DESC)` WHERE `asset_code IS NOT NULL` |
| `idx_ops_pool`        | B-tree      | `(pool_id, created_at DESC)` WHERE `pool_id IS NOT NULL`                        |
| `idx_ops_destination` | B-tree      | `(destination_id, created_at DESC)` WHERE `destination_id IS NOT NULL`          |
| `ck_ops_pool_id_len`  | Check       | `pool_id IS NULL OR octet_length(pool_id) = 32`                                 |
| `ck_ops_type_range`   | Check       | `type BETWEEN 0 AND 127`                                                        |

### Write Paths

| #   | Function                  | File:Line                              | SQL                                                                      | Trigger                   | Columns                                                                    |
| --- | ------------------------- | -------------------------------------- | ------------------------------------------------------------------------ | ------------------------- | -------------------------------------------------------------------------- |
| 1   | `insert_operations_batch` | `crates/db/src/persistence.rs:137-178` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_operations_tx_order DO NOTHING` | `persist_ledger()` step 3 | `transaction_id`, `application_order`, `source_account`, `type`, `details` |

### Post-Insert Mutability

**Fully immutable.** `ON CONFLICT DO NOTHING`. No UPDATE statements exist.

---

## `soroban_contracts`

### Description

Stores one row per deployed Soroban smart contract. Records contract identity, WASM hash, deployer, classification (`contract_type`), SAC flag, and accumulating JSONB metadata with function signatures.

### Columns

| Column               | Type                                  | Description                                                                                       |
| -------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `contract_id`        | `VARCHAR(56) PRIMARY KEY`             | Soroban contract address (C-prefixed).                                                            |
| `wasm_hash`          | `VARCHAR(64)`                         | SHA-256 hex hash of the WASM bytecode. Nullable (stub rows may exist before deployment). Indexed. |
| `deployer_account`   | `VARCHAR(56)`                         | Account that deployed the contract. Nullable.                                                     |
| `deployed_at_ledger` | `BIGINT REFERENCES ledgers(sequence)` | Ledger at which the contract was deployed. Nullable.                                              |
| `contract_type`      | `VARCHAR(50)`                         | Classification: `"token"`, `"dex"`, `"lending"`, `"nft"`, `"other"`. Indexed. Nullable.           |
| `is_sac`             | `BOOLEAN NOT NULL DEFAULT FALSE`      | Whether this is a Stellar Asset Contract. Sticky TRUE (once true, never reverts).                 |
| `metadata`           | `JSONB`                               | Accumulating JSON: function signatures, WASM byte length, etc. Merged with `\|\|` on each upsert. |
| `search_vector`      | `TSVECTOR GENERATED`                  | Full-text search over `metadata->>'name'`. GIN-indexed. DB-only.                                  |

### Write Paths

| #   | Function                                  | File:Line                          | SQL                                                                               | Trigger                                                                                     | Columns Affected                                                                                                                                                                                                                                                                                  |
| --- | ----------------------------------------- | ---------------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `ensure_contracts_exist_batch`            | `crates/db/src/soroban.rs:22-40`   | `INSERT (contract_id) ON CONFLICT DO NOTHING`                                     | `persist_ledger()` step 3b — before inserting into child tables (events, invocations, nfts) | `contract_id` only (stub row)                                                                                                                                                                                                                                                                     |
| 2   | `upsert_contract_deployments_batch`       | `crates/db/src/soroban.rs:69-142`  | `INSERT ... ON CONFLICT (contract_id) DO UPDATE SET ...`                          | `persist_ledger()` step 7                                                                   | All columns on INSERT. On conflict: `wasm_hash`, `deployer_account`, `deployed_at_ledger`, `contract_type` (COALESCE — first write wins); `is_sac` (OR — sticky TRUE); `metadata` (merge `\|\|`). Also runs embedded UPDATE joining `wasm_interface_metadata` to apply staged interface metadata. |
| 3   | `update_contract_interfaces_by_wasm_hash` | `crates/db/src/soroban.rs:175-192` | `UPDATE ... SET metadata = COALESCE(metadata, '{}') \|\| $1 WHERE wasm_hash = $2` | `persist_ledger()` step 8, per WASM upload                                                  | `metadata` only                                                                                                                                                                                                                                                                                   |

### Post-Insert Mutability

| Column               | Mutable?               | Mechanism                                                  |
| -------------------- | ---------------------- | ---------------------------------------------------------- |
| `contract_id`        | No                     | Primary key                                                |
| `wasm_hash`          | NULL → value only      | `COALESCE(existing, new)` — first write wins               |
| `deployer_account`   | NULL → value only      | `COALESCE(existing, new)`                                  |
| `deployed_at_ledger` | NULL → value only      | `COALESCE(existing, new)`                                  |
| `contract_type`      | NULL → value only      | `COALESCE(existing, new)` — **never overwritten once set** |
| `is_sac`             | FALSE → TRUE only      | `OR` logic — sticky true                                   |
| `metadata`           | Yes, always appendable | `existing \|\| new` JSON merge                             |

---

## `soroban_events`

### Description

Stores Soroban smart contract events emitted during transaction execution. Each row is a single event. **Partitioned by range on `created_at`** (monthly).

### Columns

| Column            | Type                          | Description                                                            |
| ----------------- | ----------------------------- | ---------------------------------------------------------------------- |
| `id`              | `BIGSERIAL`                   | Surrogate key. Part of composite PK `(id, created_at)`.                |
| `transaction_id`  | `BIGINT NOT NULL`             | FK to `transactions(id)` with CASCADE.                                 |
| `contract_id`     | `VARCHAR(56)`                 | FK to `soroban_contracts` (no CASCADE). NULL for system events.        |
| `event_type`      | `VARCHAR(20) NOT NULL`        | `"contract"`, `"system"`, or `"diagnostic"`.                           |
| `topics`          | `JSONB NOT NULL`              | ScVal-decoded topic values as JSON array (indexed/filterable portion). |
| `data`            | `JSONB NOT NULL`              | ScVal-decoded event data payload as JSON.                              |
| `event_index`     | `SMALLINT NOT NULL DEFAULT 0` | Zero-based index within parent transaction. Dedup key.                 |
| `ledger_sequence` | `BIGINT NOT NULL`             | Ledger sequence of parent transaction.                                 |
| `created_at`      | `TIMESTAMPTZ NOT NULL`        | Timestamp from ledger close time. Partition key.                       |

### Partitioning

Monthly: `soroban_events_y{YYYY}m{MM}`, plus default. Auto-managed by `db-partition-mgmt`.

### Indexes & Constraints

| Name                  | Columns                                            |
| --------------------- | -------------------------------------------------- |
| `idx_events_contract` | `(contract_id, created_at DESC)`                   |
| `idx_events_topics`   | GIN `(topics)`                                     |
| `idx_events_tx`       | `(transaction_id)`                                 |
| `uq_events_tx_index`  | Unique `(transaction_id, event_index, created_at)` |

### Write Paths

| #   | Function              | File:Line                          | SQL                                                                  | Trigger                   | Columns            |
| --- | --------------------- | ---------------------------------- | -------------------------------------------------------------------- | ------------------------- | ------------------ |
| 1   | `insert_events_batch` | `crates/db/src/persistence.rs:186` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_events_tx_index DO NOTHING` | `persist_ledger()` step 4 | All non-id columns |

### Post-Insert Mutability

**Fully immutable.** No UPDATE statements exist.

---

## `soroban_invocations`

### Description

Stores flattened records of Soroban contract function calls (both root and sub-invocations). **Partitioned by range on `created_at`** (monthly).

### Columns

| Column             | Type                          | Description                                                                |
| ------------------ | ----------------------------- | -------------------------------------------------------------------------- |
| `id`               | `BIGSERIAL`                   | Surrogate key. Part of composite PK `(id, created_at)`.                    |
| `transaction_id`   | `BIGINT NOT NULL`             | FK to `transactions(id)` with CASCADE.                                     |
| `contract_id`      | `VARCHAR(56)`                 | FK to `soroban_contracts` (no CASCADE). NULL for non-contract invocations. |
| `caller_account`   | `VARCHAR(56)`                 | Account or contract that initiated the call. Nullable.                     |
| `function_name`    | `VARCHAR(100) NOT NULL`       | Function name invoked. Empty string for contract creation.                 |
| `function_args`    | `JSONB`                       | ScVal-decoded function arguments. Nullable.                                |
| `return_value`     | `JSONB`                       | ScVal-decoded return value. NULL for sub-invocations.                      |
| `successful`       | `BOOLEAN NOT NULL`            | Whether invocation succeeded.                                              |
| `invocation_index` | `SMALLINT NOT NULL DEFAULT 0` | Depth-first index in the invocation tree. Dedup key.                       |
| `ledger_sequence`  | `BIGINT NOT NULL`             | Ledger sequence number.                                                    |
| `created_at`       | `TIMESTAMPTZ NOT NULL`        | Timestamp from ledger close time. Partition key.                           |

### Partitioning

Monthly: `soroban_invocations_y{YYYY}m{MM}`, plus default. Auto-managed by `db-partition-mgmt`.

### Indexes & Constraints

| Name                       | Columns                                                 |
| -------------------------- | ------------------------------------------------------- |
| `idx_invocations_contract` | `(contract_id, created_at DESC)`                        |
| `idx_invocations_function` | `(contract_id, function_name)`                          |
| `idx_invocations_tx`       | `(transaction_id)`                                      |
| `uq_invocations_tx_index`  | Unique `(transaction_id, invocation_index, created_at)` |

### Write Paths

| #   | Function                   | File:Line                          | SQL                                                                       | Trigger                   | Columns            |
| --- | -------------------------- | ---------------------------------- | ------------------------------------------------------------------------- | ------------------------- | ------------------ |
| 1   | `insert_invocations_batch` | `crates/db/src/persistence.rs:246` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_invocations_tx_index DO NOTHING` | `persist_ledger()` step 5 | All non-id columns |

### Post-Insert Mutability

**Fully immutable.** No UPDATE statements exist.

---

## `accounts`

### Description

Stores the latest observed state of Stellar accounts. A derived-state entity with a `last_seen_ledger` watermark that prevents older data from overwriting newer state. Populated from `LedgerEntryChanges` (created/updated/restored account entries).

### Columns

| Column              | Type                          | Description                                                                                                    |
| ------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `account_id`        | `VARCHAR(56) PRIMARY KEY`     | Stellar account address (G... or M...).                                                                        |
| `first_seen_ledger` | `BIGINT NOT NULL`             | Ledger at which account was first observed. Set on insert, never updated.                                      |
| `last_seen_ledger`  | `BIGINT NOT NULL`             | Most recent ledger with activity. Watermark — updates only apply when incoming >= existing. Indexed DESC.      |
| `sequence_number`   | `BIGINT NOT NULL`             | Account transaction sequence number.                                                                           |
| `balances`          | `JSONB NOT NULL DEFAULT '[]'` | Account balances as JSON array. Currently only native XLM: `[{"asset_type": "native", "balance": <stroops>}]`. |
| `home_domain`       | `VARCHAR(256)`                | Account home domain. Nullable.                                                                                 |

### Write Paths

| #   | Function                      | File:Line                          | SQL                                                                                                         | Trigger                   | Columns Affected                                                                                                                                |
| --- | ----------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_account_states_batch` | `crates/db/src/soroban.rs:196-243` | `INSERT ... ON CONFLICT (account_id) DO UPDATE SET ... WHERE last_seen_ledger <= EXCLUDED.last_seen_ledger` | `persist_ledger()` step 9 | All on INSERT. On conflict: `last_seen_ledger`, `sequence_number`, `balances` (overwritten); `home_domain` (COALESCE — only if new is non-NULL) |

### Post-Insert Mutability

| Column              | Updated?          | Mechanism                                           |
| ------------------- | ----------------- | --------------------------------------------------- |
| `first_seen_ledger` | **No**            | Preserved from first insert forever                 |
| `last_seen_ledger`  | **Yes**           | Overwritten (watermark gate)                        |
| `sequence_number`   | **Yes**           | Overwritten                                         |
| `balances`          | **Yes**           | Overwritten                                         |
| `home_domain`       | **Conditionally** | COALESCE — only overwrites if new value is non-NULL |

---

## `tokens`

### Description

Stores discovered token assets on the Stellar/Soroban network. Tracks three asset types: `classic`, `sac` (Stellar Asset Contract), and `soroban`. Currently **only SAC tokens** are produced by the indexer. The table is insert-or-ignore — effectively immutable once a row is created.

### Columns

| Column           | Type                   | Description                                                                                            |
| ---------------- | ---------------------- | ------------------------------------------------------------------------------------------------------ |
| `id`             | `SERIAL PRIMARY KEY`   | Auto-incrementing surrogate key.                                                                       |
| `asset_type`     | `VARCHAR(20) NOT NULL` | Token classification. CHECK: `'classic'`, `'sac'`, or `'soroban'`. Currently only `'sac'` is produced. |
| `asset_code`     | `VARCHAR(12)`          | Classic asset code (e.g. `USDC`). Currently always NULL.                                               |
| `issuer_address` | `VARCHAR(56)`          | Classic asset issuer. Currently always NULL.                                                           |
| `contract_id`    | `VARCHAR(56)`          | FK to `soroban_contracts` (no CASCADE). Set for SAC and soroban tokens.                                |
| `name`           | `VARCHAR(256)`         | Display name. Currently always NULL.                                                                   |
| `total_supply`   | `NUMERIC(28, 7)`       | Total supply. Currently always NULL.                                                                   |
| `holder_count`   | `INTEGER`              | Number of holders. Currently always NULL.                                                              |
| `metadata`       | `JSONB`                | Flexible metadata. Currently always NULL (not even in INSERT column list).                             |

### Indexes

| Name                 | Type   | Columns                        | Condition                                |
| -------------------- | ------ | ------------------------------ | ---------------------------------------- |
| `idx_tokens_classic` | Unique | `(asset_code, issuer_address)` | `WHERE asset_type IN ('classic', 'sac')` |
| `idx_tokens_soroban` | Unique | `(contract_id)`                | `WHERE asset_type = 'soroban'`           |
| `idx_tokens_sac`     | Unique | `(contract_id)`                | `WHERE asset_type = 'sac'`               |
| `idx_tokens_type`    | B-tree | `(asset_type)`                 | —                                        |

### Write Paths

| #   | Function              | File:Line                          | SQL                                 | Trigger                    | Columns                                                                                                              |
| --- | --------------------- | ---------------------------------- | ----------------------------------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_tokens_batch` | `crates/db/src/soroban.rs:364-411` | `INSERT ... ON CONFLICT DO NOTHING` | `persist_ledger()` step 12 | `asset_type`, `asset_code`, `issuer_address`, `contract_id`, `name`, `total_supply`, `holder_count` (NOT `metadata`) |

### Post-Insert Mutability

**Fully immutable.** `ON CONFLICT DO NOTHING` — no columns are ever updated. `total_supply`, `holder_count`, and `metadata` are always NULL with no UPDATE path.

---

## `nfts`

### Description

Stores derived NFT state from Soroban contract events (mint, transfer, burn). Each row represents a unique NFT identified by composite PK `(contract_id, token_id)`. Tracks current ownership with a `last_seen_ledger` watermark for concurrent/out-of-order safety.

### Columns

| Column             | Type                    | Description                                                                   |
| ------------------ | ----------------------- | ----------------------------------------------------------------------------- |
| `contract_id`      | `VARCHAR(56) NOT NULL`  | FK to `soroban_contracts` (no CASCADE). Part of composite PK.                 |
| `token_id`         | `VARCHAR(256) NOT NULL` | Token identifier (string representation of ScVal data). Part of composite PK. |
| `collection_name`  | `VARCHAR(256)`          | Optional collection name. Currently always NULL.                              |
| `owner_account`    | `VARCHAR(56)`           | Current owner. Set to `to` on mint/transfer, NULL on burn.                    |
| `name`             | `VARCHAR(256)`          | Optional display name. Currently always NULL.                                 |
| `media_url`        | `TEXT`                  | Optional media URL. Currently always NULL.                                    |
| `metadata`         | `JSONB`                 | Flexible metadata. Currently always NULL.                                     |
| `minted_at_ledger` | `BIGINT`                | Ledger at which NFT was minted. Only set for mint events.                     |
| `last_seen_ledger` | `BIGINT NOT NULL`       | Watermark — guards against stale overwrites.                                  |

### Indexes

| Name                  | Columns                          |
| --------------------- | -------------------------------- |
| PK                    | `(contract_id, token_id)`        |
| `idx_nfts_owner`      | `owner_account`                  |
| `idx_nfts_collection` | `(contract_id, collection_name)` |

### Write Paths

| #   | Function            | File:Line                          | SQL                                                                                                                    | Trigger                                                                           | Columns Affected                                                                                                                                    |
| --- | ------------------- | ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_nfts_batch` | `crates/db/src/soroban.rs:415-473` | `INSERT ... ON CONFLICT (contract_id, token_id) DO UPDATE SET ... WHERE last_seen_ledger <= EXCLUDED.last_seen_ledger` | `persist_ledger()` final step, after in-memory merge by `(contract_id, token_id)` | All on INSERT. On conflict: `owner_account` (always overwritten), `name`/`media_url`/`metadata` (COALESCE), `last_seen_ledger` (always overwritten) |

### Post-Insert Mutability

| Column             | Updated?        | Mechanism                                        |
| ------------------ | --------------- | ------------------------------------------------ |
| `owner_account`    | **Yes, always** | Unconditionally set — tracks ownership transfers |
| `name`             | Conditionally   | COALESCE — only if new value non-NULL            |
| `media_url`        | Conditionally   | COALESCE — only if new value non-NULL            |
| `metadata`         | Conditionally   | COALESCE — only if new value non-NULL            |
| `last_seen_ledger` | **Yes, always** | Watermark gate                                   |
| `collection_name`  | **No**          | Only set on initial INSERT                       |
| `minted_at_ledger` | **No**          | Only set on initial INSERT                       |

---

## `liquidity_pools`

### Description

Stores the **current state** of each Stellar liquidity pool (AMM). Unpartitioned entity table. Uses `last_updated_ledger` as a monotonic watermark to prevent out-of-order replays from overwriting newer state.

### Columns

| Column                | Type                      | Description                                          |
| --------------------- | ------------------------- | ---------------------------------------------------- |
| `pool_id`             | `VARCHAR(64) PRIMARY KEY` | Pool hash identifier (64-char hex).                  |
| `asset_a`             | `JSONB NOT NULL`          | First reserve asset descriptor (code, issuer, type). |
| `asset_b`             | `JSONB NOT NULL`          | Second reserve asset descriptor.                     |
| `fee_bps`             | `INTEGER NOT NULL`        | Trading fee in basis points (e.g. 30 = 0.30%).       |
| `reserves`            | `JSONB NOT NULL`          | Current reserves for both assets.                    |
| `total_shares`        | `NUMERIC NOT NULL`        | Total outstanding pool share tokens.                 |
| `tvl`                 | `NUMERIC`                 | Total value locked. Nullable.                        |
| `created_at_ledger`   | `BIGINT NOT NULL`         | Ledger at which pool was first created.              |
| `last_updated_ledger` | `BIGINT NOT NULL`         | Most recent ledger with state change. Watermark.     |

### Write Paths

| #   | Function                       | File:Line                      | SQL                                                                                                            | Trigger                    | Columns Affected                                                                          |
| --- | ------------------------------ | ------------------------------ | -------------------------------------------------------------------------------------------------------------- | -------------------------- | ----------------------------------------------------------------------------------------- |
| 1   | `upsert_liquidity_pools_batch` | `crates/db/src/soroban.rs:248` | `INSERT ... ON CONFLICT (pool_id) DO UPDATE SET ... WHERE last_updated_ledger <= EXCLUDED.last_updated_ledger` | `persist_ledger()` step 10 | All on INSERT. On conflict: `reserves`, `total_shares`, `tvl`, `last_updated_ledger` only |

### Post-Insert Mutability

| Column                                                          | Updated?                        |
| --------------------------------------------------------------- | ------------------------------- |
| `reserves`                                                      | **Yes**                         |
| `total_shares`                                                  | **Yes**                         |
| `tvl`                                                           | **Yes**                         |
| `last_updated_ledger`                                           | **Yes**                         |
| `pool_id`, `asset_a`, `asset_b`, `fee_bps`, `created_at_ledger` | **No** — immutable after insert |

---

## `liquidity_pool_snapshots`

### Description

Append-only time-series table recording point-in-time snapshots of pool state at each ledger where a change occurred. **Partitioned by range on `created_at`** (monthly).

### Columns

| Column            | Type                   | Description                                             |
| ----------------- | ---------------------- | ------------------------------------------------------- |
| `id`              | `BIGSERIAL`            | Surrogate key. Part of composite PK `(id, created_at)`. |
| `pool_id`         | `VARCHAR(64) NOT NULL` | FK to `liquidity_pools`.                                |
| `ledger_sequence` | `BIGINT NOT NULL`      | Ledger sequence at snapshot time.                       |
| `created_at`      | `TIMESTAMPTZ NOT NULL` | Snapshot timestamp. Partition key.                      |
| `reserves`        | `JSONB NOT NULL`       | Pool reserves at this point in time.                    |
| `total_shares`    | `NUMERIC NOT NULL`     | Total pool shares at snapshot time.                     |
| `tvl`             | `NUMERIC`              | TVL at snapshot time. Nullable.                         |
| `volume`          | `NUMERIC`              | Trading volume during snapshot period. Nullable.        |
| `fee_revenue`     | `NUMERIC`              | Fee revenue during snapshot period. Nullable.           |

### Partitioning

Monthly: `liquidity_pool_snapshots_y{YYYY}m{MM}`, plus default. Auto-managed by `db-partition-mgmt`.

### Write Paths

| #   | Function                                | File:Line                      | SQL                                                                        | Trigger                    | Columns            |
| --- | --------------------------------------- | ------------------------------ | -------------------------------------------------------------------------- | -------------------------- | ------------------ |
| 1   | `insert_liquidity_pool_snapshots_batch` | `crates/db/src/soroban.rs:310` | `INSERT ... ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING` | `persist_ledger()` step 11 | All non-id columns |

### Post-Insert Mutability

**Fully immutable.** `ON CONFLICT DO NOTHING`. Strictly append-only.

---

## `wasm_interface_metadata`

### Description

Permanent staging table that persists WASM interface metadata (function signatures, bytecode size) keyed by `wasm_hash`. Solves Soroban's **2-ledger deploy pattern**: WASM is uploaded in ledger A (producing interface data), but the contract is deployed in ledger B. This table bridges the gap — metadata is staged at upload time and applied when the contract deployment is upserted.

### Columns

| Column      | Type                      | Description                                                                                  |
| ----------- | ------------------------- | -------------------------------------------------------------------------------------------- |
| `wasm_hash` | `VARCHAR(64) PRIMARY KEY` | Hex-encoded SHA-256 hash of the WASM bytecode. Natural key (WASM is immutable on-chain).     |
| `metadata`  | `JSONB NOT NULL`          | Contains `"functions"` (array of function signatures) and `"wasm_byte_len"` (bytecode size). |

### Write Paths

| #   | Function                         | File:Line                          | SQL                                                                             | Trigger                                    | Columns Affected |
| --- | -------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------ | ---------------- |
| 1   | `upsert_wasm_interface_metadata` | `crates/db/src/soroban.rs:149-166` | `INSERT ... ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata` | `persist_ledger()` step 8, per WASM upload | Both columns     |

### Read Paths (used by other write paths)

- `upsert_contract_deployments_batch` (soroban.rs:128-139) JOINs this table to apply staged metadata to `soroban_contracts.metadata` during contract deployment.

### Post-Insert Mutability

`metadata` can be overwritten on conflict, but this is idempotent (same WASM hash = same interface). `wasm_hash` is the PK and never changes.

---

## Summary: Mutability Matrix

| Table                      |             Immutable             |        Watermark-guarded        |         Appendable          |
| -------------------------- | :-------------------------------: | :-----------------------------: | :-------------------------: |
| `ledgers`                  |              **yes**              |                                 |                             |
| `transactions`             | mostly (`operation_tree` updated) |                                 |                             |
| `operations`               |              **yes**              |                                 |                             |
| `soroban_contracts`        |                                   |                                 | `metadata` via `\|\|` merge |
| `soroban_events`           |              **yes**              |                                 |                             |
| `soroban_invocations`      |              **yes**              |                                 |                             |
| `accounts`                 |                                   |  **yes** (`last_seen_ledger`)   |                             |
| `tokens`                   |              **yes**              |                                 |                             |
| `nfts`                     |                                   |  **yes** (`last_seen_ledger`)   |                             |
| `liquidity_pools`          |                                   | **yes** (`last_updated_ledger`) |                             |
| `liquidity_pool_snapshots` |              **yes**              |                                 |                             |
| `wasm_interface_metadata`  |       idempotent overwrite        |                                 |                             |

## Persist Pipeline Order

| Step | Function                                                                     | Table(s)                                            |
| ---- | ---------------------------------------------------------------------------- | --------------------------------------------------- |
| 1    | `insert_ledger`                                                              | `ledgers`                                           |
| 2    | `insert_transactions_batch`                                                  | `transactions`                                      |
| 3    | `insert_operations_batch`                                                    | `operations`                                        |
| 3b   | `ensure_contracts_exist_batch`                                               | `soroban_contracts` (stub rows for FK satisfaction) |
| 4    | `insert_events_batch`                                                        | `soroban_events`                                    |
| 5    | `insert_invocations_batch`                                                   | `soroban_invocations`                               |
| 6    | `update_operation_trees_batch`                                               | `transactions` (operation_tree)                     |
| 7    | `upsert_contract_deployments_batch`                                          | `soroban_contracts`                                 |
| 8    | `upsert_wasm_interface_metadata` + `update_contract_interfaces_by_wasm_hash` | `wasm_interface_metadata` + `soroban_contracts`     |
| 9    | `upsert_account_states_batch`                                                | `accounts`                                          |
| 10   | `upsert_liquidity_pools_batch`                                               | `liquidity_pools`                                   |
| 11   | `insert_liquidity_pool_snapshots_batch`                                      | `liquidity_pool_snapshots`                          |
| 12   | `upsert_tokens_batch`                                                        | `tokens`                                            |
| 13   | `upsert_nfts_batch`                                                          | `nfts`                                              |
