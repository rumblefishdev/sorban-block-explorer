export interface LedgerPointer {
  sequence: number;
  closedAt: string;
}

export interface TransactionPointer {
  hash: string;
  ledgerSequence: number;
}

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
