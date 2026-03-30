import type { BigIntString } from './primitives.js';
import type { TransactionSummary } from './transaction.js';

// --- Ledger domain types ---

export interface Ledger {
  sequence: BigIntString;
  hash: string;
  closedAt: string;
  protocolVersion: number;
  transactionCount: number;
  baseFee: BigIntString;
}

export type LedgerPointer = Pick<Ledger, 'sequence' | 'closedAt'>;

export type LedgerSummary = Ledger;

export interface LedgerDetail extends LedgerSummary {
  transactions: readonly TransactionSummary[];
}
