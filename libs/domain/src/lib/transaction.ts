import type { BigIntString, JsonValue } from './primitives.js';
import type { OperationType, Operation } from './operation.js';
import type { SorobanEvent } from './soroban.js';

// --- Transaction domain types ---

export interface Transaction {
  id: BigIntString;
  hash: string;
  ledgerSequence: BigIntString;
  sourceAccount: string;
  feeCharged: BigIntString;
  successful: boolean;
  resultCode: string | null;
  envelopeXdr: string;
  resultXdr: string;
  resultMetaXdr: string | null;
  memoType: string | null;
  memo: string | null;
  createdAt: string;
  parseError: boolean;
  operationTree: JsonValue | null;
}

export type TransactionPointer = Pick<Transaction, 'hash' | 'ledgerSequence'>;

export interface TransactionSummary {
  hash: string;
  ledgerSequence: BigIntString;
  sourceAccount: string;
  operationType: OperationType;
  successful: boolean;
  feeCharged: BigIntString;
  createdAt: string;
}

export interface TransactionDetail extends TransactionSummary {
  resultCode: string | null;
  envelopeXdr: string;
  resultXdr: string;
  resultMetaXdr: string | null;
  memoType: string | null;
  memo: string | null;
  parseError: boolean;
  operationTree: JsonValue | null;
  operations: readonly Operation[];
  events: readonly SorobanEvent[];
}
