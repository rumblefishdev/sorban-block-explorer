-- ADR 0027 + ADR 0031 + ADR 0035 — initial schema, step 7/7: account balances (current only)
-- Native-XLM rows use NULL for asset_code and issuer_id; credit-asset rows
-- require both. Enforced via a CHECK constraint plus partial UNIQUE indexes
-- (one row per account for native, one row per (account, asset_code, issuer)
-- for credit assets).
--
-- `asset_type` SMALLINT is the Rust `AssetType` XDR enum
-- (0=native, 1=credit_alphanum4, 2=credit_alphanum12 — label helper:
-- asset_type_name). Balances never carry pool_share (3) because parser
-- state.rs skips pool_share trustlines — LP positions live in `lp_positions`.
--
-- ADR 0035: `account_balance_history` was dropped — zero read consumers,
-- ~10-20 ms/ledger write cost; chart feature design deferred to launch time.
--
-- Tables:
--   17. account_balances_current  (unpartitioned current state)

-- 17. account_balances_current (ADR 0027 §17 + ADR 0031)
CREATE TABLE account_balances_current (
    account_id          BIGINT        NOT NULL REFERENCES accounts(id),
    asset_type          SMALLINT      NOT NULL, -- ADR 0031: AssetType (XDR)
    asset_code          VARCHAR(12),
    issuer_id           BIGINT        REFERENCES accounts(id),
    balance             NUMERIC(28,7) NOT NULL,
    last_updated_ledger BIGINT        NOT NULL,
    CONSTRAINT ck_abc_asset_type_range CHECK (asset_type BETWEEN 0 AND 15),
    CONSTRAINT ck_abc_native
      CHECK ((asset_type =  0 AND asset_code IS NULL     AND issuer_id IS NULL)           -- native
          OR (asset_type <> 0 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL))      -- credit
);
CREATE UNIQUE INDEX uidx_abc_native ON account_balances_current (account_id)
    WHERE asset_type = 0;                         -- native
CREATE UNIQUE INDEX uidx_abc_credit ON account_balances_current (account_id, asset_code, issuer_id)
    WHERE asset_type <> 0;                        -- credit
CREATE INDEX idx_abc_asset ON account_balances_current (asset_code, issuer_id)
    WHERE asset_code IS NOT NULL;
