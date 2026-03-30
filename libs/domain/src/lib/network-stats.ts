import type { BigIntString } from './primitives.js';

// --- Network stats ---

/**
 * Aggregated network statistics — all fields are explorer-derived.
 *
 * No single Horizon/RPC endpoint returns these values directly.
 * The indexer maintains running counts; `transactionsPerSecond` is
 * computed from recent ledger close times. Highly cacheable (5-15 s TTL).
 */
export interface NetworkStats {
  currentLedgerSequence: BigIntString;
  transactionsPerSecond: number;
  totalAccounts: number;
  totalContracts: number;
}
