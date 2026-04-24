-- Rollback the five constraints that were dropped too aggressively.
-- Every one of them is referenced by an ON CONFLICT clause in
-- crates/indexer/src/handler/persist/write.rs — dropping them makes
-- every INSERT into the corresponding table fail with SQLSTATE 42P10.

BEGIN;

ALTER TABLE transaction_participants
  ADD CONSTRAINT transaction_participants_pkey
  PRIMARY KEY (account_id, created_at, transaction_id);

ALTER TABLE soroban_events_appearances
  ADD CONSTRAINT soroban_events_appearances_pkey
  PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at);

ALTER TABLE soroban_invocations_appearances
  ADD CONSTRAINT soroban_invocations_appearances_pkey
  PRIMARY KEY (contract_id, transaction_id, ledger_sequence, created_at);

ALTER TABLE nft_ownership
  ADD CONSTRAINT nft_ownership_pkey
  PRIMARY KEY (nft_id, created_at, ledger_sequence, event_order);

ALTER TABLE operations_appearances
  ADD CONSTRAINT uq_ops_app_identity
  UNIQUE NULLS NOT DISTINCT
    (transaction_id, type, source_id, destination_id, contract_id,
     asset_code, asset_issuer_id, pool_id, ledger_sequence, created_at);

COMMIT;
