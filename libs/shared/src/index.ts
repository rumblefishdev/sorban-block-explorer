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
