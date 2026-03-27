import type {
  XdrParseError,
  UnknownOperationTypeError,
  ScValDecodeError,
  ContractMetadataError,
  ParseErrorContext,
  ExplorerParseError,
} from './errors.js';

function now(): string {
  return new Date().toISOString();
}

function logError(
  error: ExplorerParseError,
  extra?: Record<string, unknown>
): void {
  console.error(
    JSON.stringify({
      level: 'error',
      event: error.errorType,
      message: error.message,
      ...error.context,
      ...extra,
      timestamp: error.timestamp,
    })
  );
}

// --- Handler result types ---

export interface XdrParseErrorResult {
  parseError: true;
  error: XdrParseError;
}

export interface UnknownOperationResult {
  operationType: 'unknown';
  rawXdr: string;
  error: UnknownOperationTypeError;
}

export interface ScValDecodeErrorResult {
  unparsed: true;
  rawValue: string;
  error: ScValDecodeError;
}

export interface ContractMetadataErrorResult {
  metadataMissing: true;
  error: ContractMetadataError;
}

// --- Handlers ---

export function handleXdrParseError(
  err: Error,
  context: ParseErrorContext & { decodeStep: string; rawXdr: string }
): XdrParseErrorResult {
  const { rawXdr, decodeStep, ...baseContext } = context;
  const error: XdrParseError = {
    errorType: 'XdrParseError',
    message: err.message,
    stack: err.stack,
    context: baseContext,
    timestamp: now(),
    rawXdr,
    decodeStep,
  };
  logError(error, { decodeStep });
  return { parseError: true, error };
}

export function handleUnknownOperation(
  operationType: number,
  rawXdr: string,
  context: ParseErrorContext
): UnknownOperationResult {
  const error: UnknownOperationTypeError = {
    errorType: 'UnknownOperationType',
    message: `Unknown operation type: ${String(operationType)}`,
    context,
    timestamp: now(),
    operationType,
    rawXdr,
  };
  logError(error, { operationType });
  return { operationType: 'unknown', rawXdr, error };
}

export function handleScValDecodeError(
  err: Error,
  fieldContext: string,
  rawValue: string,
  context: ParseErrorContext & { parentId: string }
): ScValDecodeErrorResult {
  const { parentId, ...baseContext } = context;
  const error: ScValDecodeError = {
    errorType: 'ScValDecodeError',
    message: err.message,
    stack: err.stack,
    context: baseContext,
    timestamp: now(),
    rawValue,
    fieldContext,
    parentId,
  };
  logError(error, { fieldContext, parentId });
  return { unparsed: true, rawValue, error };
}

export function handleContractMetadataError(
  err: Error,
  context: ParseErrorContext & {
    contractId: string;
    wasmHash: string;
    extractionStep: string;
  }
): ContractMetadataErrorResult {
  const { contractId, wasmHash, extractionStep, ...baseContext } = context;
  const error: ContractMetadataError = {
    errorType: 'ContractMetadataError',
    message: err.message,
    stack: err.stack,
    context: baseContext,
    timestamp: now(),
    contractId,
    wasmHash,
    extractionStep,
  };
  logError(error, { contractId, wasmHash, extractionStep });
  return { metadataMissing: true, error };
}
