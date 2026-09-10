import { listPoolsOptions, type ListPoolsData } from '@rumblefish/api-types';
import { useQuery } from '@tanstack/react-query';

import { listPolicy } from '../polling.js';

type Filters = NonNullable<ListPoolsData['query']>;

/**
 * `GET /liquidity-pools` — cursor-paginated liquidity-pool list.
 *
 * Supported filters per task 0246:
 *   * `filter[asset_code]` — case-insensitive substring of any leg, or a
 *     `A/B` pair query requiring both codes on two different legs (task 0440).
 *     Backs the list's free-text asset input.
 *   * `filter[pool_kind]` — `classic` | `soroban`. Backs the kind chip row.
 *   * `filter[min_tvl]` — decimal threshold.
 *
 * Each (filter, cursor) combination forms a distinct queryKey, so
 * revisiting a cursor is a cache hit. URL-as-state pagination — caller
 * passes the current cursor from `useCursorPagination`.
 */
export const usePoolsList = (cursor: string | null = null, filters?: Filters) =>
  useQuery({
    ...listPoolsOptions({
      query: { ...(filters ?? {}), ...(cursor ? { cursor } : {}) },
    }),
    ...listPolicy,
  });
