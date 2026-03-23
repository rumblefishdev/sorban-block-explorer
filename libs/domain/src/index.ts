export interface LedgerPointer {
  sequence: number;
  closedAt: string;
}

export interface TransactionPointer {
  hash: string;
  ledgerSequence: number;
}
