---
id: '0125'
title: 'LP analytics: price oracle, TVL, volume, and fee revenue'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0052', '0077']
tags: [priority-low, effort-large, layer-indexer, layer-backend, audit-gap]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — LP tvl/volume/fee_revenue columns exist but are likely always NULL without external pricing.'
---

# LP analytics: price oracle, TVL, volume, and fee revenue

## Summary

`liquidity_pools.tvl`, `liquidity_pool_snapshots.tvl`, `.volume`, and `.fee_revenue`
columns exist in the schema but are effectively always NULL. Computing real values requires:

- **TVL**: USD price oracle to convert reserves to dollar value.
- **Volume**: Tracking individual swap operations per pool per time window.
- **Fee revenue**: Derived from volume \* fee_bps.

## Implementation

1. **Price oracle**: Integrate external price feed (CoinGecko, StellarExpert API, or
   Horizon aggregation endpoint) to get USD prices for pool assets.
2. **TVL computation**: reserve*a * price*a + reserve_b * price_b, updated per snapshot.
3. **Volume tracking**: Identify swap operations in `soroban_invocations` or
   `soroban_events` for each pool and aggregate per time window.
4. **Fee revenue**: volume \* (fee_bps / 10000).
5. Decide: inline during indexing or separate scheduled enrichment job?

## Acceptance Criteria

- [ ] `liquidity_pools.tvl` populated with USD-denominated value
- [ ] `liquidity_pool_snapshots.tvl` populated per snapshot
- [ ] `liquidity_pool_snapshots.volume` populated from trade activity
- [ ] `liquidity_pool_snapshots.fee_revenue` computed from volume
- [ ] Chart endpoint (`GET /liquidity-pools/:id/chart`) returns non-null time series
