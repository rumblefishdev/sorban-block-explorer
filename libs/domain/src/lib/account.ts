import type { BigIntString, JsonValue } from './primitives.js';

// --- Account domain types ---

/**
 * Explorer account — derived-state entity with ledger-sequence watermarks.
 *
 * `firstSeenLedger` / `lastSeenLedger` (FK → ledgers.sequence) enforce
 * monotonic updates: older batches cannot overwrite newer state.
 */
export interface Account {
  accountId: string;
  firstSeenLedger: BigIntString;
  lastSeenLedger: BigIntString;
  sequenceNumber: BigIntString | null;
  balances: readonly JsonValue[];
  homeDomain: string | null;
}
