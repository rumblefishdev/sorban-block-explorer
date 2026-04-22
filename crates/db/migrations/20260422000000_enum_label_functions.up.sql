-- ADR 0031 — IMMUTABLE SQL helpers that render readable labels for each
-- SMALLINT enum column in psql / BI dashboards. Planner inlines each
-- CASE expression; no runtime cost vs. a native ENUM cast.
--
-- Reversible per MIGRATIONS.md convention — the paired `.down.sql` drops
-- all six functions. The SMALLINT columns themselves live in the
-- irreversible 0002-0007 baseline (initial schema per ADR 0027).
--
-- Every function MUST stay bitwise-identical to the matching Rust enum's
-- `as_str()` output. An integration test (see
-- `crates/indexer/tests/persist_integration.rs`) enumerates every variant
-- of every enum and asserts `fn_name(v as i16) = v.as_str()` — silent
-- drift between Rust and SQL surfaces in CI, not in prod.
--
-- Unknown discriminants fall through the CASE and return NULL rather
-- than raising. The CHECK range on each column guards against that
-- happening in practice.

-- operations.type — Rust `domain::OperationType` (27 variants)
CREATE FUNCTION op_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN  0 THEN 'CREATE_ACCOUNT'
        WHEN  1 THEN 'PAYMENT'
        WHEN  2 THEN 'PATH_PAYMENT_STRICT_RECEIVE'
        WHEN  3 THEN 'MANAGE_SELL_OFFER'
        WHEN  4 THEN 'CREATE_PASSIVE_SELL_OFFER'
        WHEN  5 THEN 'SET_OPTIONS'
        WHEN  6 THEN 'CHANGE_TRUST'
        WHEN  7 THEN 'ALLOW_TRUST'
        WHEN  8 THEN 'ACCOUNT_MERGE'
        WHEN  9 THEN 'INFLATION'
        WHEN 10 THEN 'MANAGE_DATA'
        WHEN 11 THEN 'BUMP_SEQUENCE'
        WHEN 12 THEN 'MANAGE_BUY_OFFER'
        WHEN 13 THEN 'PATH_PAYMENT_STRICT_SEND'
        WHEN 14 THEN 'CREATE_CLAIMABLE_BALANCE'
        WHEN 15 THEN 'CLAIM_CLAIMABLE_BALANCE'
        WHEN 16 THEN 'BEGIN_SPONSORING_FUTURE_RESERVES'
        WHEN 17 THEN 'END_SPONSORING_FUTURE_RESERVES'
        WHEN 18 THEN 'REVOKE_SPONSORSHIP'
        WHEN 19 THEN 'CLAWBACK'
        WHEN 20 THEN 'CLAWBACK_CLAIMABLE_BALANCE'
        WHEN 21 THEN 'SET_TRUST_LINE_FLAGS'
        WHEN 22 THEN 'LIQUIDITY_POOL_DEPOSIT'
        WHEN 23 THEN 'LIQUIDITY_POOL_WITHDRAW'
        WHEN 24 THEN 'INVOKE_HOST_FUNCTION'
        WHEN 25 THEN 'EXTEND_FOOTPRINT_TTL'
        WHEN 26 THEN 'RESTORE_FOOTPRINT'
    END
$$;

-- liquidity_pools.asset_{a,b}_type + account_balances*.asset_type —
-- Rust `domain::AssetType` (XDR, 4 variants).
CREATE FUNCTION asset_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'native'
        WHEN 1 THEN 'credit_alphanum4'
        WHEN 2 THEN 'credit_alphanum12'
        WHEN 3 THEN 'pool_share'
    END
$$;

-- tokens.asset_type — explorer-synthetic Rust `domain::TokenAssetType` (4 variants).
CREATE FUNCTION token_asset_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'native'
        WHEN 1 THEN 'classic'
        WHEN 2 THEN 'sac'
        WHEN 3 THEN 'soroban'
    END
$$;

-- soroban_events.event_type — Rust `domain::ContractEventType` (3 variants).
CREATE FUNCTION event_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'system'
        WHEN 1 THEN 'contract'
        WHEN 2 THEN 'diagnostic'
    END
$$;

-- nft_ownership.event_type — Rust `domain::NftEventType` (3 variants).
CREATE FUNCTION nft_event_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'mint'
        WHEN 1 THEN 'transfer'
        WHEN 2 THEN 'burn'
    END
$$;

-- soroban_contracts.contract_type — Rust `domain::ContractType` (2 variants).
CREATE FUNCTION contract_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'token'
        WHEN 1 THEN 'other'
    END
$$;
