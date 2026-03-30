/**
 * Workspace import verification.
 *
 * Re-exports from libs/domain and libs/shared prove that Nx workspace
 * resolution works correctly for this app. These re-exports are available
 * for feature modules to use directly.
 */
export type {
  NetworkStats,
  PaginatedResponse,
  PaginationRequest,
} from '@rumblefish/soroban-block-explorer-domain';

export type {
  ExplorerParseError,
  ParseErrorType,
} from '@rumblefish/soroban-block-explorer-shared';
