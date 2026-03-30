import type { BigIntString, JsonValue } from './primitives.js';

// --- NFT domain types ---

/**
 * Explorer NFT — token scoped by contract.
 *
 * UNIQUE(contractId, tokenId). Transfer history is derived from stored
 * Soroban events, not a separate table. Many fields are nullable because
 * NFT contract conventions are not standardized.
 */
export interface NFT {
  id: BigIntString;
  contractId: string;
  tokenId: string;
  collectionName: string | null;
  ownerAccount: string | null;
  name: string | null;
  mediaUrl: string | null;
  metadata: JsonValue | null;
  mintedAtLedger: BigIntString | null;
  lastSeenLedger: BigIntString | null;
}
