# Audit harness — Phase 1 SQL invariants

**Timestamp:** 2026-04-30T17:05:59Z
**Database:** postgres://postgres:postgres@localhost:5432/soroban_block_explorer

---

## ledgers
### I1 — sequence contiguous within indexed range
 violations | sample 
------------+--------
          0 | 
(1 row)

### I2 — hash UNIQUE
 violations 
------------
          0
(1 row)

### I3 — closed_at strictly monotonic by sequence
 violations | sample 
------------+--------
          0 | 
(1 row)

### I4 — non-negative counts
 violations 
------------
          0
(1 row)


## transactions
### I1 — hash UNIQUE across partitions (uq_transactions_hash_created_at, but hash alone)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I2 — operation_count >= COUNT(operations_appearances rows) per tx
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/02_transactions.sql:32: ERROR:  could not resize shared memory segment "/PostgreSQL.2386597818" to 134483968 bytes: No space left on device
### I3 — every transaction.ledger_sequence exists in ledgers
 violations | sample 
------------+--------
          0 | 
(1 row)

### I4 — source_id FK valid (every source_id → accounts.id)
 violations 
------------
          0
(1 row)

### I5 — non-negative numeric fields
 violations 
------------
          0
(1 row)

### I6 — inner_tx_hash either NULL or 32 bytes (matches CHECK)
 violations 
------------
          0
(1 row)


## transaction_hash_index
### I1 — every hash routes to existing transactions row
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/03_transaction_hash_index.sql:16: ERROR:  could not resize shared memory segment "/PostgreSQL.902596550" to 16777216 bytes: No space left on device
CONTEXT:  parallel worker
### I2 — every transactions row has matching hash_index entry
 violations | sample 
------------+--------
          0 | 
(1 row)

### I3 — hash UNIQUE
 violations 
------------
          0
(1 row)

### I4 — hash exactly 32 bytes (matches CHECK)
 violations 
------------
          0
(1 row)


## operations_appearances
### I1 — every (transaction_id, created_at) → existing transactions row (composite FK)
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/04_operations_appearances.sql:17: ERROR:  could not resize shared memory segment "/PostgreSQL.1833916070" to 134483968 bytes: No space left on device
### I2 — source_id FK valid where set
 violations 
------------
          0
(1 row)

### I3 — destination_id FK valid where set
 violations 
------------
          0
(1 row)

### I4 — asset_issuer_id FK valid where set
 violations 
------------
          0
(1 row)

### I5 — pool_id FK valid where set
 violations 
------------
          0
(1 row)

### I6 — amount (folded duplicate count) >= 1 when present
 violations 
------------
          0
(1 row)


## transaction_participants
### I1 — composite FK to transactions valid
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/05_transaction_participants.sql:16: ERROR:  could not resize shared memory segment "/PostgreSQL.3574679276" to 134483968 bytes: No space left on device
### I2 — account_id FK to accounts valid
 violations 
------------
          0
(1 row)

### I3 — composite UNIQUE (transaction_id, account_id, created_at) — no duplicate participation
 violations 
------------
          0
(1 row)


## soroban_contracts
### I1 — contract_id matches StrKey shape (56 chars, prefix C, base32)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I2 — contract_id UNIQUE
 violations 
------------
          0
(1 row)

### I3 — deployer_id FK valid where set
 violations 
------------
          0
(1 row)

### I4 — wasm_hash (when set) → wasm_interface_metadata.wasm_hash
 violations | sample 
------------+--------
          0 | 
(1 row)

### I5 — contract_type SMALLINT in known range (per ADR 0031 + ADR 0036)
 violations 
------------
          0
(1 row)

### I6 — wasm_hash exactly 32 bytes when set
 violations 
------------
          0
(1 row)


## wasm_interface_metadata
### I1 — wasm_hash UNIQUE (PK)
 violations 
------------
          0
(1 row)

### I2 — wasm_hash exactly 32 bytes
 violations 
------------
          0
(1 row)

### I3 — metadata is valid JSONB object (not NULL, not array, not scalar)
 violations 
------------
          0
(1 row)


## soroban_events_appearances
### I1 — composite FK to transactions valid
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/08_soroban_events_appearances.sql:17: ERROR:  could not resize shared memory segment "/PostgreSQL.1051644132" to 134483968 bytes: No space left on device
### I2 — contract_id FK to soroban_contracts valid
 violations 
------------
          0
(1 row)

### I3 — ledger_sequence matches the parent transaction.ledger_sequence
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/08_soroban_events_appearances.sql:35: ERROR:  could not resize shared memory segment "/PostgreSQL.1241124238" to 134483968 bytes: No space left on device
### I4 — amount (folded duplicates) >= 1 when present
 violations 
------------
          0
(1 row)


## soroban_invocations_appearances
### I1 — composite FK to transactions valid
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/09_soroban_invocations_appearances.sql:11: ERROR:  could not resize shared memory segment "/PostgreSQL.1355867044" to 134483968 bytes: No space left on device
### I2 — contract_id FK to soroban_contracts valid
 violations 
------------
          0
(1 row)

### I3 — caller_id FK to accounts valid where set
 violations 
------------
          0
(1 row)

### I4 — ledger_sequence matches parent transaction.ledger_sequence
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/09_soroban_invocations_appearances.sql:29: ERROR:  could not resize shared memory segment "/PostgreSQL.1488997708" to 134483968 bytes: No space left on device
### I5 — amount (folded duplicates) >= 1 when present
 violations 
------------
          0
(1 row)

### I6 — every invoked contract has at least one event appearance OR is a no-event invocation
#### info: invocation rows | events rows | invocations w/o ANY event
 invocation_rows | event_rows | invocations_without_events 
-----------------+------------+----------------------------
         5894062 |   14103352 |                    5135574
(1 row)


## assets
### I1 — asset_type SMALLINT in known range (0-3 per ADR 0036)
 violations 
------------
          0
(1 row)

### I2 — ck_assets_identity per ADR 0038 (validate shape per type)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I3 — uidx_assets_native singleton (exactly one row with asset_type=0)
 violations | native_row_count 
------------+------------------
          0 |                1
(1 row)

### I4 — issuer_id FK valid where set
 violations 
------------
          0
(1 row)

### I5 — contract_id FK to soroban_contracts valid where set
 violations 
------------
          0
(1 row)

### I6 — non-negative supply / holder count
 violations 
------------
          0
(1 row)


## accounts
### I1 — account_id matches StrKey shape (G or M prefix, 56 or 69 chars, base32)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I2 — account_id UNIQUE
 violations 
------------
          0
(1 row)

### I3 — first_seen_ledger ≤ last_seen_ledger (monotonic)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I4 — non-negative ledger sequences
 violations 
------------
          0
(1 row)


## account_balances_current
### I1 — account_id FK valid
 violations 
------------
          0
(1 row)

### I2 — issuer_id FK valid where set
 violations 
------------
          0
(1 row)

### I3 — asset_type=0 (native) row has NULL asset_code/issuer_id; non-native has both
 violations 
------------
          0
(1 row)

### I4 — balance ≥ 0 (NUMERIC stored as NUMERIC(28,7))
 violations 
------------
          0
(1 row)

### I5 — uidx_abc_native: at most one native row per account_id
 violations 
------------
          0
(1 row)

### I6 — uidx_abc_credit: (account_id, asset_code, issuer_id) UNIQUE for non-native
 violations 
------------
          0
(1 row)


## nfts
### I1 — (contract_id, token_id) UNIQUE
 violations 
------------
          0
(1 row)

### I2 — contract_id FK to soroban_contracts valid
 violations 
------------
          0
(1 row)

### I3 — current_owner_id FK to accounts valid where set
 violations 
------------
          0
(1 row)

### I4 — minted_at_ledger ≤ current_owner_ledger (monotonic, mint precedes any transfer)
 violations 
------------
          0
(1 row)

### I5 — last nft_ownership row per nft → matches nfts.current_owner_id (mat-view consistency)
 violations | sample 
------------+--------
          0 | 
(1 row)


## nft_ownership
### I1 — nft_id FK to nfts valid
psql:/Users/stkrolikiewicz/Developer/RumbleFish/sbe-audit/crates/audit-harness/sql/14_nft_ownership.sql:11: ERROR:  could not resize shared memory segment "/PostgreSQL.2267442126" to 16777216 bytes: No space left on device
### I2 — transaction_id FK valid
 violations 
------------
          0
(1 row)

### I3 — owner_id FK to accounts valid where set
 violations 
------------
          0
(1 row)

### I4 — event_type SMALLINT in valid range (mint/transfer/burn enum)
 violations 
------------
          0
(1 row)

### I5 — first event per nft is a mint (event_type denoting mint)
 informational_only 
--------------------
                  0
(1 row)

### I6 — event_order non-negative within ledger
 violations 
------------
          0
(1 row)


## liquidity_pools
### I1 — pool_id is 32 bytes (SHA-256 of asset pair per Stellar protocol)
 violations 
------------
          0
(1 row)

### I2 — pool_id UNIQUE (PK)
 violations 
------------
          0
(1 row)

### I3 — asset_a < asset_b type/code ordering enforced (Stellar canonicalises pair order)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I4 — issuer FK valid where set (asset_a, asset_b)
 asset_a_violations | asset_b_violations 
--------------------+--------------------
                  0 |                  0
(1 row)

### I5 — fee_bps in [0, 10000] (basis points)
 violations 
------------
          0
(1 row)


## liquidity_pool_snapshots
### I1 — pool_id FK to liquidity_pools valid
 violations 
------------
          0
(1 row)

### I2 — non-negative reserves and shares
 violations | sample 
------------+--------
          0 | 
(1 row)

### I3 — analytics fields (tvl, volume, fee_revenue) non-negative when set
 violations 
------------
          0
(1 row)

### I4 — at most one snapshot per (pool_id, ledger_sequence) — uq_lp_snapshots_pool_ledger
 violations 
------------
          0
(1 row)

### I5 — ledger_sequence corresponds to existing ledgers row
 violations 
------------
          0
(1 row)


## lp_positions
### I1 — pool_id FK valid
 violations 
------------
          0
(1 row)

### I2 — account_id FK valid
 violations 
------------
          0
(1 row)

### I3 — shares ≥ 0 (zero shares retained for future-history per task 0162 emerged decision)
 violations 
------------
          0
(1 row)

### I4 — first_deposit_ledger ≤ last_updated_ledger (monotonic)
 violations | sample 
------------+--------
          0 | 
(1 row)

### I5 — (pool_id, account_id) UNIQUE (composite PK)
 violations 
------------
          0
(1 row)

### I6 — sum of active positions per pool ≈ latest snapshot.total_shares (within stale tolerance)
 violations | sample 
------------+--------
          0 | 
(1 row)


## partition_routing
### I1 — count rows in _default per parent (expect 0 across the board)
 total_violations | sample_per_parent 
------------------+-------------------
                0 | 
(1 row)

### I2 — count children per parent (sanity: 30 monthly + 1 default = 31)
             parent              | children 
---------------------------------+----------
 liquidity_pool_snapshots        |       31
 nft_ownership                   |       31
 operations_appearances          |       31
 soroban_events_appearances      |       31
 soroban_invocations_appearances |       31
 transaction_participants        |       31
 transactions                    |       31
(7 rows)

### I3 — informational: rows-per-month heatmap (last 6 months of activity)
#### transactions
       partition       | approx_rows 
-----------------------+-------------
 transactions_y2026m07 |          -1
 transactions_y2026m06 |          -1
 transactions_y2026m05 |          -1
 transactions_y2026m04 |     9501891
 transactions_y2026m03 |          -1
 transactions_y2026m02 |          -1
(6 rows)


