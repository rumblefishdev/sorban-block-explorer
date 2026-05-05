-- Synthetic fixture for "laptop B" — ledgers 4..6.
-- Strict-later than laptop A; same alice + same C1 (overlap exercises remap).
-- new account: charlie.
-- New tx 3, 4. NFT N1 transferred from alice to charlie at ledger 5.
-- alice's native balance updated to 2000 at ledger 6 (newer than A's last_updated=3).

INSERT INTO ledgers VALUES
  (4, decode('00'||repeat('dd',31),'hex'), '2026-04-01 00:01:00+00', 22, 0, 100),
  (5, decode('00'||repeat('ee',31),'hex'), '2026-04-01 00:01:01+00', 22, 2, 100),
  (6, decode('00'||repeat('ff',31),'hex'), '2026-04-01 00:01:02+00', 22, 0, 100);

-- alice (id 1 in this laptop's DB) and charlie (id 2 in this laptop's DB).
-- Note: alice's id here (1) is the same numeric value as in laptop A by coincidence
-- of insertion order — that's exactly what the merge remap needs to disambiguate.
INSERT INTO accounts(account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain) VALUES
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111', 4, 6, 350, 'updated.com'),
  ('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA333', 5, 5, 100, NULL);

-- C1 already deployed in A; B references it without new deploy info.
-- Stub-now-fill-later: B's row carries only contract_id, NULL deploy info.
-- (Indexer's upsert-by-natural-key handles this — we mirror.)
INSERT INTO soroban_contracts(contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id, deployed_at_ledger, contract_type, is_sac, metadata) VALUES
  ('CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA111', NULL, NULL, NULL, NULL, NULL, false, NULL);

-- B references the same SorobanToken (asset_type=3, contract_id) as A
INSERT INTO assets(asset_type, asset_code, issuer_id, contract_id, name, total_supply, holder_count) VALUES
  (3, NULL, NULL, 1, 'SorobanToken', NULL, NULL);

-- Same pool as A; created_at_ledger=4 (B's range). LEAST(1, 4)=1 wins on merge.
INSERT INTO liquidity_pools VALUES
  (decode('44'||repeat('00',31),'hex'), 0, NULL, NULL, 0, NULL, NULL, 30, 4);

-- Same NFT (natural key contract+token_id), transferred to charlie (id 2 here).
-- current_owner_* set to charlie at ledger 5.
INSERT INTO nfts(contract_id, token_id, collection_name, name, media_url, metadata, minted_at_ledger, current_owner_id, current_owner_ledger) VALUES
  (1, 'token-1', 'TestColl', 'NFT One', 'http://x', '{}', 2, 2, 5);

-- B's transactions: source=alice (id 1) and source=charlie (id 2)
INSERT INTO transactions VALUES
  (DEFAULT, decode('cc'||repeat('00',31),'hex'), 5, 1, 1, 100, NULL, true, 1, false, false, '2026-04-01 00:01:01+00'),
  (DEFAULT, decode('dd'||repeat('00',31),'hex'), 5, 2, 2, 100, NULL, true, 1, false, false, '2026-04-01 00:01:01.5+00');

INSERT INTO transaction_hash_index VALUES
  (decode('cc'||repeat('00',31),'hex'), 5, '2026-04-01 00:01:01+00'),
  (decode('dd'||repeat('00',31),'hex'), 5, '2026-04-01 00:01:01.5+00');

INSERT INTO operations_appearances(transaction_id, type, source_id, destination_id, contract_id, asset_code, asset_issuer_id, pool_id, amount, ledger_sequence, created_at) VALUES
  -- alice transfers NFT to charlie (op type=8 invokeHostFunction or similar — using 1 for simplicity)
  (1, 1, 1, 2, NULL, NULL, NULL, NULL, 1, 5, '2026-04-01 00:01:01+00');

INSERT INTO transaction_participants VALUES
  (1, 1, '2026-04-01 00:01:01+00'),
  (1, 2, '2026-04-01 00:01:01+00'),
  (2, 2, '2026-04-01 00:01:01.5+00');

-- transfer event on N1: from alice to charlie at ledger 5 event_order 0
INSERT INTO nft_ownership(nft_id, transaction_id, owner_id, event_type, ledger_sequence, event_order, created_at) VALUES
  (1, 1, 2, 1, 5, 0, '2026-04-01 00:01:01+00');

INSERT INTO liquidity_pool_snapshots(pool_id, ledger_sequence, reserve_a, reserve_b, total_shares, tvl, volume, fee_revenue, created_at) VALUES
  (decode('44'||repeat('00',31),'hex'), 5, 1100, 2200, 1600, NULL, NULL, NULL, '2026-04-01 00:01:01+00');

-- alice's lp_position updated: shares now 600, last_updated=5
INSERT INTO lp_positions VALUES
  (decode('44'||repeat('00',31),'hex'), 1, 600, 4, 5);

-- alice's native balance at ledger 6 = 2000 (newer than A's 1000 at ledger 3)
INSERT INTO account_balances_current(account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger) VALUES
  (1, 0, NULL, NULL, 2000, 6);
