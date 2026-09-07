import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';

import { getSearchOptions } from '@rumblefish/api-types';
import type {
  EntityType,
  SearchHit,
  SearchResults,
} from '@rumblefish/api-types';
import {
  DEFAULT_DEBOUNCE_MS,
  useDebounced,
} from '@rumblefish/soroban-block-explorer-ui';

import { searchPolicy } from '../api/polling.js';
import { federatedDomain } from './federation.js';

// `/v1/search` returns a flat `SearchResults` payload — task 0271
// dropped the previous `SearchResponse::Redirect` wire variant. Task
// 0527 removed the singleton auto-navigation that used to read
// `totalCount`; the count is still what the tab badges render.

export const TAB_ORDER: ReadonlyArray<EntityType> = [
  'transaction',
  'account',
  'contract',
  'asset',
  'nft',
  'pool',
];

export const ENTITY_LABEL: Record<EntityType, string> = {
  transaction: 'Transactions',
  account: 'Accounts',
  contract: 'Contract',
  asset: 'Token',
  nft: 'NFT',
  pool: 'Liquidity Pool',
};

interface UseSearchResultsParams {
  q: string;
}

/**
 * Per-entity result cap sent to `/search`. A saturated bucket means "at least
 * this many", never "exactly this many" — the badge renders `N+` at the cap so
 * a truncated bucket is not read as a total (0377 F7).
 *
 * Applies to the four searched buckets (accounts, contracts, assets, NFTs).
 * The transaction and pool buckets are exact-identity lookups — `WHERE hash =`
 * / `WHERE pool_id =` with `LIMIT 1` server-side — so they ignore this value
 * and can never saturate. Do not describe it as a cap the whole endpoint obeys.
 */
export const SEARCH_GROUP_LIMIT = 10;

export interface SearchResultsState {
  effectiveQuery: string;
  data: SearchResults | undefined;
  isFetching: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
  counts: Record<EntityType, number>;
  totalCount: number;
  activeTab: EntityType;
  setActiveTab: (t: EntityType) => void;
  hitsForActiveTab: readonly SearchHit[];
}

export function useSearchResults({
  q,
}: UseSearchResultsParams): SearchResultsState {
  const debouncedRaw = useDebounced(q, DEFAULT_DEBOUNCE_MS);
  const effectiveQuery = debouncedRaw.trim();
  // `/v1/search` knows nothing about SEP-2, so for a federated address it can
  // only answer zero hits, which renders as "No results for karol*lobstr.co" —
  // a claim that the address does not exist. Suppressed here, on the hook that
  // issues the request, so the gate and the request cannot disagree about what
  // the query is.
  const enabled =
    effectiveQuery.length > 0 && federatedDomain(effectiveQuery) == null;

  const query = useQuery({
    // Sent explicitly rather than relying on the server default, so the cap
    // the badge reasons about and the cap the API applies to the searched
    // buckets are one value and cannot drift (0377 F7).
    ...getSearchOptions({
      query: { q: effectiveQuery, limit: SEARCH_GROUP_LIMIT },
    }),
    ...searchPolicy,
    enabled,
  });

  const data = query.data;

  const counts = useMemo<Record<EntityType, number>>(() => {
    const empty: Record<EntityType, number> = {
      transaction: 0,
      account: 0,
      contract: 0,
      asset: 0,
      nft: 0,
      pool: 0,
    };
    if (!data) return empty;
    const { groups } = data;
    return {
      transaction: groups.transactions?.length ?? 0,
      account: groups.accounts?.length ?? 0,
      contract: groups.contracts?.length ?? 0,
      asset: groups.assets?.length ?? 0,
      nft: groups.nfts?.length ?? 0,
      pool: groups.pools?.length ?? 0,
    };
  }, [data]);

  const totalCount = TAB_ORDER.reduce((sum, t) => sum + counts[t], 0);

  const [activeTab, setActiveTab] = useState<EntityType>('transaction');

  useEffect(() => {
    if (!enabled || query.isFetching || totalCount === 0) return;
    if (counts[activeTab] > 0) return;
    const firstWithHits = TAB_ORDER.find((t) => counts[t] > 0);
    if (firstWithHits) setActiveTab(firstWithHits);
  }, [enabled, query.isFetching, totalCount, counts, activeTab]);

  const hitsForActiveTab = useMemo<readonly SearchHit[]>(() => {
    if (!data) return [];
    const { groups } = data;
    switch (activeTab) {
      case 'transaction':
        return groups.transactions ?? [];
      case 'account':
        return groups.accounts ?? [];
      case 'contract':
        return groups.contracts ?? [];
      case 'asset':
        return groups.assets ?? [];
      case 'nft':
        return groups.nfts ?? [];
      case 'pool':
        return groups.pools ?? [];
    }
  }, [data, activeTab]);

  return {
    effectiveQuery,
    data,
    isFetching: enabled && query.isFetching,
    isError: query.isError,
    error: query.error,
    refetch: () => {
      void query.refetch();
    },
    counts,
    totalCount,
    activeTab,
    setActiveTab,
    hitsForActiveTab,
  };
}
