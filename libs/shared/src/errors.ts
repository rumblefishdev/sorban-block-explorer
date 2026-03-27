// --- Parse error types ---
// Core principle: "log, store raw, mark parse_error, keep visible."
// Partial records are always preferred over missing records.

export type ParseErrorType =
  | 'XdrParseError'
  | 'UnknownOperationType'
  | 'ScValDecodeError'
  | 'ContractMetadataError';

export interface ParseErrorContext {
  transactionHash?: string;
  ledgerSequence?: number;
  contractId?: string;
}

export interface ExplorerParseError {
  errorType: ParseErrorType;
  message: string;
  stack?: string;
  context: ParseErrorContext;
  timestamp: string;
}

/** fromXDR() throws during ingestion. */
export interface XdrParseError extends ExplorerParseError {
  errorType: 'XdrParseError';
  rawXdr: string;
  decodeStep: string;
}

/** A protocol upgrade introduces an operation type the SDK doesn't know yet. */
export interface UnknownOperationTypeError extends ExplorerParseError {
  errorType: 'UnknownOperationType';
  operationType: number;
  rawXdr: string;
}

/** Malformed or unexpected ScVal encountered during decode. */
export interface ScValDecodeError extends ExplorerParseError {
  errorType: 'ScValDecodeError';
  rawValue: string;
  fieldContext: string;
  parentId: string;
}

/** WASM interface extraction fails for a deployed contract. */
export interface ContractMetadataError extends ExplorerParseError {
  errorType: 'ContractMetadataError';
  contractId: string;
  wasmHash: string;
  extractionStep: string;
}
