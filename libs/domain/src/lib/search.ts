// --- Search types ---

export type SearchEntityType =
  | 'transaction'
  | 'contract'
  | 'token'
  | 'account'
  | 'nft'
  | 'pool';

export interface SearchRequest {
  q: string;
  type?: readonly SearchEntityType[];
}

export interface SearchResultItem {
  identifier: string;
  entityType: SearchEntityType;
  context: string;
}

export interface SearchResultGroup {
  entityType: SearchEntityType;
  count: number;
  results: readonly SearchResultItem[];
}
