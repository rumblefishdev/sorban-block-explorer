// --- Shared type primitives ---

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

/** Decoded Soroban ScVal. Placeholder until task 0013 provides a full ScVal type. */
export type ScVal = JsonValue;

/** String representation of a PostgreSQL BIGINT/BIGSERIAL value. */
export type BigIntString = string;

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
  transactions: TransactionSummary[];
}

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
  operationType: string;
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
  operations: Operation[];
  events: SorobanEvent[];
}

// --- Operation domain types ---

export type OperationType =
  | 'CREATE_ACCOUNT'
  | 'PAYMENT'
  | 'PATH_PAYMENT_STRICT_RECEIVE'
  | 'PATH_PAYMENT_STRICT_SEND'
  | 'MANAGE_SELL_OFFER'
  | 'MANAGE_BUY_OFFER'
  | 'CREATE_PASSIVE_SELL_OFFER'
  | 'SET_OPTIONS'
  | 'CHANGE_TRUST'
  | 'ALLOW_TRUST'
  | 'ACCOUNT_MERGE'
  | 'INFLATION'
  | 'MANAGE_DATA'
  | 'BUMP_SEQUENCE'
  | 'CREATE_CLAIMABLE_BALANCE'
  | 'CLAIM_CLAIMABLE_BALANCE'
  | 'BEGIN_SPONSORING_FUTURE_RESERVES'
  | 'END_SPONSORING_FUTURE_RESERVES'
  | 'REVOKE_SPONSORSHIP'
  | 'CLAWBACK'
  | 'CLAWBACK_CLAIMABLE_BALANCE'
  | 'SET_TRUST_LINE_FLAGS'
  | 'LIQUIDITY_POOL_DEPOSIT'
  | 'LIQUIDITY_POOL_WITHDRAW'
  | 'INVOKE_HOST_FUNCTION'
  | 'EXTEND_FOOTPRINT_TTL'
  | 'RESTORE_FOOTPRINT'
  | (string & {});

export interface InvokeHostFunctionDetails {
  contractId: string;
  functionName: string;
  functionArgs: readonly ScVal[];
  returnValue: ScVal | null;
}

export interface Operation {
  id: BigIntString;
  transactionId: BigIntString;
  type: string;
  details: Readonly<Record<string, JsonValue>>;
}

// --- Pagination types ---

export interface PaginationRequest {
  cursor: string | null;
  limit: number;
}

export interface PaginatedResponse<T> {
  data: T[];
  nextCursor: string | null;
  hasMore: boolean;
}

// --- Soroban domain types ---

export type ContractType = 'token' | 'dex' | 'lending' | 'nft' | 'other';

export interface ContractFunction {
  name: string;
  parameters: readonly { name: string; type: string }[];
  returnType: string;
}

export interface ContractMetadata {
  functions?: readonly ContractFunction[];
  [key: string]: JsonValue | readonly ContractFunction[] | undefined;
}

export interface SorobanContract {
  contractId: string;
  wasmHash: string | null;
  deployerAccount: string | null;
  deployedAtLedger: BigIntString | null;
  contractType: ContractType | null;
  isSac: boolean | null;
  metadata: ContractMetadata | null;
}

export type EventType = 'contract' | 'system' | 'diagnostic';

export interface SorobanInvocation {
  id: BigIntString;
  transactionId: BigIntString | null;
  contractId: string | null;
  callerAccount: string | null;
  functionName: string;
  functionArgs: ScVal | null;
  returnValue: ScVal | null;
  successful: boolean;
  ledgerSequence: BigIntString;
  createdAt: string;
}

export interface SorobanEvent {
  id: BigIntString;
  transactionId: BigIntString | null;
  contractId: string | null;
  eventType: EventType;
  topics: readonly ScVal[];
  data: ScVal;
  ledgerSequence: BigIntString;
  createdAt: string;
}

export type InterpretationType = 'swap' | 'transfer' | 'mint' | 'burn';

export interface EventInterpretation {
  id: BigIntString;
  eventId: BigIntString | null;
  interpretationType: InterpretationType;
  humanReadable: string;
  structuredData: Readonly<Record<string, JsonValue>>;
}
