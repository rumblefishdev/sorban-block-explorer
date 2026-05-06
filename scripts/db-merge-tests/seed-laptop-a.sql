-- Synthetic fixture for "laptop A" — ledgers 1..3.
-- Models what `backfill-runner` over [1..3] would produce.
--
-- Accounts:
--   alice (G…111) — appears in both laptop A and laptop B (overlap → exercises
--     accounts remap with conflict-update path on merge).
--   bob   (G…222) — only laptop A.
-- Contract C1 deployed in A by alice; referenced again in B (overlap).
-- Native LP P1 created at ledger 1; same row appears in B (LEAST(created_at)).
-- NFT N1 minted in A (event at ledger 2), transferred in B.

INSERT INTO ledgers VALUES
  (1, decode('00'||repeat('aa',31),'hex'), '2026-04-01 00:00:00+00', 22, 1, 100),
  (2, decode('00'||repeat('bb',31),'hex'), '2026-04-01 00:00:01+00', 22, 2, 100),
  (3, decode('00'||repeat('cc',31),'hex'), '2026-04-01 00:00:02+00', 22, 0, 100);

-- ids 1, 2 (in this DB)
INSERT INTO accounts(account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain) VALUES
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111', 1, 3, 200, 'example.com'),
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA222', 2, 2, 100, NULL);

INSERT INTO wasm_interface_metadata VALUES
  (decode('11'||repeat('00',31),'hex'), '{"name":"contract_one_wasm"}');

-- contract C1: deployer = alice (account id 1 in this DB)
INSERT INTO soroban_contracts(contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, deployed_at_ledger, contract_type, is_sac, metadata) VALUES
  ('CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111',
   decode('11'||repeat('00',31),'hex'), 1, 1, 1, 1, false, '{"name":"contract_one"}');

INSERT INTO assets(asset_type, asset_code, issuer_id, contract_id, name, total_supply, holder_count) VALUES
  (1, 'USDC', 2, NULL, 'USD Coin', NULL, NULL),
  (3, NULL, NULL, 1, 'SorobanToken', NULL, NULL);

INSERT INTO liquidity_pools VALUES
  (decode('44'||repeat('00',31),'hex'), 0, NULL, NULL, 0, NULL, NULL, 30, 1);

INSERT INTO nfts(contract_id, token_id, collection_name, name, media_url, metadata, minted_at_ledger, current_owner_id, current_owner_ledger) VALUES
  (1, 'token-1', 'TestColl', 'NFT One', 'http://x', '{}', 2, 1, 2);

INSERT INTO transactions VALUES
  (DEFAULT, decode('aa'||repeat('00',31),'hex'), 2, 1, 1, 100, NULL, true, 1, false, false, '2026-04-01 00:00:01+00'),
  (DEFAULT, decode('bb'||repeat('00',31),'hex'), 2, 2, 2, 100, NULL, true, 1, true,  false, '2026-04-01 00:00:01.5+00');

INSERT INTO transaction_hash_index VALUES
  (decode('aa'||repeat('00',31),'hex'), 2, '2026-04-01 00:00:01+00'),
  (decode('bb'||repeat('00',31),'hex'), 2, '2026-04-01 00:00:01.5+00');

INSERT INTO operations_appearances(transaction_id, type, source_id, destination_id, contract_id, asset_code, asset_issuer_id, pool_id, amount, ledger_sequence, created_at) VALUES
  (1, 1, 1, 2, NULL, 'USDC', 2, NULL, 100, 2, '2026-04-01 00:00:01+00'),
  (2, 24, 2, NULL, 1, NULL, NULL, NULL, 1, 2, '2026-04-01 00:00:01.5+00');

INSERT INTO transaction_participants VALUES
  (1, 1, '2026-04-01 00:00:01+00'),
  (1, 2, '2026-04-01 00:00:01+00'),
  (2, 2, '2026-04-01 00:00:01.5+00');

INSERT INTO soroban_events_appearances VALUES
  (1, 2, 2, 1, '2026-04-01 00:00:01.5+00');

INSERT INTO soroban_invocations_appearances(contract_id, transaction_id, ledger_sequence, caller_id, caller_contract_id, amount, created_at) VALUES
  (1, 2, 2, 2, NULL, 1, '2026-04-01 00:00:01.5+00');

-- mint event on N1: owner=alice (id 1), ledger 2
INSERT INTO nft_ownership(nft_id, transaction_id, owner_id, event_type, ledger_sequence, event_order, created_at) VALUES
  (1, 1, 1, 0, 2, 0, '2026-04-01 00:00:01+00');

INSERT INTO liquidity_pool_snapshots(pool_id, ledger_sequence, reserve_a, reserve_b, total_shares, tvl, volume, fee_revenue, created_at) VALUES
  (decode('44'||repeat('00',31),'hex'), 2, 1000, 2000, 1500, NULL, NULL, NULL, '2026-04-01 00:00:01+00');

-- alice deposits to pool at ledger 2
INSERT INTO lp_positions VALUES
  (decode('44'||repeat('00',31),'hex'), 1, 500, 2, 2);

-- alice's native balance = 1000 at ledger 3; alice's USDC balance = 50 at ledger 3
INSERT INTO account_balances_current(account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger) VALUES
  (1, 0, NULL, NULL, 1000, 3),
  (1, 1, 'USDC', 2, 50, 3);
