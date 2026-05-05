-- Synthetic ground-truth fixture: state that sequential backfill over
-- the full range [1..6] would produce. Equivalent to A then B applied
-- via the indexer's natural-key upserts in ledger order.
--
-- Specifically — the merged-state values per ADR 0040 §"Watermark
-- merges" + accounts upsert clause:
--   alice.first_seen = LEAST(1, 4) = 1
--   alice.last_seen  = GREATEST(3, 6) = 6
--   alice.sequence_number = 350 (B's value, since B's last_seen=6 ≥ A's last_seen=3)
--   alice.home_domain = 'updated.com' (B's value, same reason)
--   C1: A's full data wins (B is stub-only, COALESCE picks A)
--   pool P1.created_at_ledger = LEAST(1, 4) = 1
--   alice.lp_position: shares=600, first_deposit=2, last_updated=5
--   alice.native_balance: 2000 at ledger 6
--   nft.current_owner = charlie, current_owner_ledger=5 (latest event in nft_ownership)

INSERT INTO ledgers VALUES
  (1, decode('00'||repeat('aa',31),'hex'), '2026-04-01 00:00:00+00', 22, 1, 100),
  (2, decode('00'||repeat('bb',31),'hex'), '2026-04-01 00:00:01+00', 22, 2, 100),
  (3, decode('00'||repeat('cc',31),'hex'), '2026-04-01 00:00:02+00', 22, 0, 100),
  (4, decode('00'||repeat('dd',31),'hex'), '2026-04-01 00:01:00+00', 22, 0, 100),
  (5, decode('00'||repeat('ee',31),'hex'), '2026-04-01 00:01:01+00', 22, 2, 100),
  (6, decode('00'||repeat('ff',31),'hex'), '2026-04-01 00:01:02+00', 22, 0, 100);

-- alice id=1 (first observed at ledger 1), bob id=2, charlie id=3
INSERT INTO accounts(account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain) VALUES
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111', 1, 6, 350, 'updated.com'),
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA222', 2, 2, 100, NULL),
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA333', 5, 5, 100, NULL);

INSERT INTO wasm_interface_metadata VALUES
  (decode('11'||repeat('00',31),'hex'), '{"name":"contract_one_wasm"}');

-- C1 — A's deploy info preserved (B was stub)
INSERT INTO soroban_contracts(contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, deployed_at_ledger, contract_type, is_sac, metadata) VALUES
  ('CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111',
   decode('11'||repeat('00',31),'hex'), 1, 1, 1, 1, false, '{"name":"contract_one"}');

-- assets: native (seeded by migration), USDC (issuer=bob id 2), SorobanToken (contract=1)
INSERT INTO assets(asset_type, asset_code, issuer_id, contract_id, name, total_supply, holder_count) VALUES
  (1, 'USDC', 2, NULL, 'USD Coin', NULL, NULL),
  (3, NULL, NULL, 1, 'SorobanToken', NULL, NULL);

INSERT INTO liquidity_pools VALUES
  (decode('44'||repeat('00',31),'hex'), 0, NULL, NULL, 0, NULL, NULL, 30, 1);

-- N1: minted at ledger 2; current_owner=charlie (id 3) ledger 5 (latest event)
INSERT INTO nfts(contract_id, token_id, collection_name, name, media_url, metadata, minted_at_ledger, current_owner_id, current_owner_ledger) VALUES
  (1, 'token-1', 'TestColl', 'NFT One', 'http://x', '{}', 2, 3, 5);

-- 4 transactions in order
INSERT INTO transactions VALUES
  (DEFAULT, decode('aa'||repeat('00',31),'hex'), 2, 1, 1, 100, NULL, true, 1, false, false, '2026-04-01 00:00:01+00'),
  (DEFAULT, decode('bb'||repeat('00',31),'hex'), 2, 2, 2, 100, NULL, true, 1, true,  false, '2026-04-01 00:00:01.5+00'),
  (DEFAULT, decode('cc'||repeat('00',31),'hex'), 5, 1, 1, 100, NULL, true, 1, false, false, '2026-04-01 00:01:01+00'),
  (DEFAULT, decode('dd'||repeat('00',31),'hex'), 5, 2, 3, 100, NULL, true, 1, false, false, '2026-04-01 00:01:01.5+00');

INSERT INTO transaction_hash_index VALUES
  (decode('aa'||repeat('00',31),'hex'), 2, '2026-04-01 00:00:01+00'),
  (decode('bb'||repeat('00',31),'hex'), 2, '2026-04-01 00:00:01.5+00'),
  (decode('cc'||repeat('00',31),'hex'), 5, '2026-04-01 00:01:01+00'),
  (decode('dd'||repeat('00',31),'hex'), 5, '2026-04-01 00:01:01.5+00');

-- ops: 2 from A + 1 from B (B had only one op)
INSERT INTO operations_appearances(transaction_id, type, source_id, destination_id, contract_id, asset_code, asset_issuer_id, pool_id, amount, ledger_sequence, created_at) VALUES
  (1, 1, 1, 2, NULL, 'USDC', 2, NULL, 100, 2, '2026-04-01 00:00:01+00'),
  (2, 24, 2, NULL, 1, NULL, NULL, NULL, 1, 2, '2026-04-01 00:00:01.5+00'),
  (3, 1, 1, 3, NULL, NULL, NULL, NULL, 1, 5, '2026-04-01 00:01:01+00');

-- transaction_participants: 3 from A + 3 from B = 6 total
INSERT INTO transaction_participants VALUES
  (1, 1, '2026-04-01 00:00:01+00'),
  (1, 2, '2026-04-01 00:00:01+00'),
  (2, 2, '2026-04-01 00:00:01.5+00'),
  (3, 1, '2026-04-01 00:01:01+00'),
  (3, 3, '2026-04-01 00:01:01+00'),
  (4, 3, '2026-04-01 00:01:01.5+00');

-- 1 event from A (B had none)
INSERT INTO soroban_events_appearances VALUES
  (1, 2, 2, 1, '2026-04-01 00:00:01.5+00');

-- 1 invocation from A
INSERT INTO soroban_invocations_appearances(contract_id, transaction_id, ledger_sequence, caller_id, caller_contract_id, amount, created_at) VALUES
  (1, 2, 2, 2, NULL, 1, '2026-04-01 00:00:01.5+00');

-- nft_ownership: mint at ledger 2 + transfer at ledger 5
INSERT INTO nft_ownership(nft_id, transaction_id, owner_id, event_type, ledger_sequence, event_order, created_at) VALUES
  (1, 1, 1, 0, 2, 0, '2026-04-01 00:00:01+00'),
  (1, 3, 3, 1, 5, 0, '2026-04-01 00:01:01+00');

INSERT INTO liquidity_pool_snapshots(pool_id, ledger_sequence, reserve_a, reserve_b, total_shares, tvl, volume, fee_revenue, created_at) VALUES
  (decode('44'||repeat('00',31),'hex'), 2, 1000, 2000, 1500, NULL, NULL, NULL, '2026-04-01 00:00:01+00'),
  (decode('44'||repeat('00',31),'hex'), 5, 1100, 2200, 1600, NULL, NULL, NULL, '2026-04-01 00:01:01+00');

-- alice's lp_position: B's value wins via watermark (last_updated=5 > 2)
INSERT INTO lp_positions VALUES
  (decode('44'||repeat('00',31),'hex'), 1, 600, 2, 5);

-- alice native balance: B's 2000 at ledger 6 wins; alice USDC credit only in A
INSERT INTO account_balances_current(account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger) VALUES
  (1, 0, NULL, NULL, 2000, 6),
  (1, 1, 'USDC', 2, 50, 3);
