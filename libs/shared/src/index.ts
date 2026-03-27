export const projectName = 'soroban-block-explorer';

export type {
  ParseErrorType,
  ParseErrorContext,
  ExplorerParseError,
  XdrParseError,
  UnknownOperationTypeError,
  ScValDecodeError,
  ContractMetadataError,
} from './errors.js';

export type {
  XdrParseErrorResult,
  UnknownOperationResult,
  ScValDecodeErrorResult,
  ContractMetadataErrorResult,
} from './error-handlers.js';

export {
  handleXdrParseError,
  handleUnknownOperation,
  handleScValDecodeError,
  handleContractMetadataError,
} from './error-handlers.js';

// --- XDR parsing utilities ---
export {
  decodeScVal,
  computeTransactionHash,
  extractMemo,
  decodeEventTopics,
  decodeContractEvent,
  extractContractDeployments,
  extractAccountStates,
  extractLiquidityPoolStates,
  decodeInvocationTree,
  extractContractInterface,
  extractContractInterfaceFromEntries,
} from './xdr/index.js';

export type {
  DecodedScVal,
  ExtractedMemo,
  DecodedEventType,
  DecodedContractEvent,
  ExtractedContractDeployment,
  ExtractedAccountState,
  ExtractedLiquidityPoolState,
  InvocationNode,
  ContractFunction,
} from './xdr/index.js';
