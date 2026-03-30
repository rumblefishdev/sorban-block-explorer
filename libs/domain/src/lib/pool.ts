import type { BigIntString, NumericString } from './primitives.js';

// --- Liquidity pool domain types ---

/**
 * Single asset within a liquidity pool.
 *
 * Horizon format: `"CODE:ISSUER"` for classic assets or contract ID for
 * Soroban tokens. Amount is a fixed-precision decimal string
 * (`NUMERIC(28,7)` as stored in PostgreSQL).
 */
export interface PoolAsset {
  asset: string;
  amount: NumericString;
}

/**
 * Explorer liquidity pool — current-state entity.
 *
 * Asset fields are stored as JSONB in PostgreSQL (because a pool can pair
 * a classic Stellar asset with a Soroban-native token) but represented as
 * typed `PoolAsset` in the domain model.
 */
export interface LiquidityPool {
  poolId: string;
  assetA: PoolAsset;
  assetB: PoolAsset;
  /**
   * Fee in basis points. Classic Stellar AMM pools are hardcoded to 30 bps
   * (CAP-0038). Nullable for Soroban DEX pools with non-standard fee models.
   */
  feeBps: number | null;
  reserves: readonly PoolAsset[];
  totalShares: NumericString | null;
  totalTrustlines: number | null;
  /** Explorer-derived: reserves × price. Not a chain primitive. */
  tvl: NumericString | null;
  createdAtLedger: BigIntString | null;
  lastUpdatedLedger: BigIntString | null;
}

/**
 * Point-in-time snapshot of a liquidity pool.
 *
 * Append-only, written in ledger order, monthly-partitioned by `createdAt`.
 * Metrics (`volume`, `feeRevenue`) are explorer-derived measures, not chain
 * primitives.
 */
export interface LiquidityPoolSnapshot {
  id: BigIntString;
  poolId: string;
  ledgerSequence: BigIntString;
  createdAt: string;
  reserves: readonly PoolAsset[];
  totalShares: NumericString | null;
  tvl: NumericString | null;
  volume: NumericString | null;
  feeRevenue: NumericString | null;
}

export type PoolChartInterval = '1h' | '1d' | '1w';

export interface PoolChartDataPoint {
  createdAt: string;
  tvl: NumericString;
  volume: NumericString;
  feeRevenue: NumericString;
}
