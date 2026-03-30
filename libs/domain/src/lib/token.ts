import type { JsonValue, NumericString } from './primitives.js';

// --- Token domain types ---

export type AssetType = 'classic' | 'sac' | 'soroban';

/**
 * Explorer token — unifies classic Stellar assets and Soroban-native tokens.
 *
 * Identity varies by asset type:
 * - classic: `assetCode` + `issuerAddress` (UNIQUE constraint)
 * - sac/soroban: `contractId` (UNIQUE constraint, FK → soroban_contracts)
 */
export interface Token {
  id: number;
  assetType: AssetType;
  assetCode: string | null;
  issuerAddress: string | null;
  contractId: string | null;
  name: string | null;
  totalSupply: NumericString | null;
  /** DDL: INT DEFAULT 0. Semantically non-nullable — always initialized to 0. */
  holderCount: number;
  metadata: JsonValue | null;
}
