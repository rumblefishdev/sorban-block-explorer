ALTER TABLE soroban_invocations_appearances
    DROP CONSTRAINT IF EXISTS ck_sia_caller_xor;

ALTER TABLE soroban_invocations_appearances
    DROP COLUMN IF EXISTS caller_contract_id;
