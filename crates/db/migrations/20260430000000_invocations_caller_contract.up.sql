-- Task 0183 — full Soroban invocation coverage via fn_call/fn_return
-- diagnostic events. The diagnostic-tree walker exposes contract-to-contract
-- callers (auth-less router → pool sub-calls in DeFi multi-hop swaps),
-- which the existing `caller_id BIGINT REFERENCES accounts(id)` column
-- cannot represent. Add a parallel contract-FK column gated by an XOR
-- CHECK so each row carries exactly one caller variant (or NULL/NULL
-- defensively, for cases where the parser cannot resolve a caller).
--
-- The column is unindexed for the same reason `caller_id` is unindexed
-- (ADR 0034 §3): callers are payload, not a query path on this table.
-- E11 surfaces them through `COUNT(DISTINCT)` over the small per-contract
-- slice, not range scans.
--
-- Partitioned-parent + plain `ALTER TABLE` cascades to children. No
-- CONCURRENTLY since Postgres forbids it on partitioned tables and this
-- runs against a freshly populated staging DB pre-traffic (per
-- backfill-execution-plan.md), matching the convention in
-- 20260428000100_add_endpoint_query_indexes.up.sql.

ALTER TABLE soroban_invocations_appearances
    ADD COLUMN caller_contract_id BIGINT REFERENCES soroban_contracts(id);

ALTER TABLE soroban_invocations_appearances
    ADD CONSTRAINT ck_sia_caller_xor
    CHECK (
        caller_id IS NULL
        OR caller_contract_id IS NULL
    );
