import { Stack } from '@mui/material';
import type { ListPoolsData } from '@rumblefish/api-types';
import { useCursorPagination } from '@rumblefish/soroban-block-explorer-ui';
import { useCallback, useMemo } from 'react';

import { PAGE_SIZE, usePoolsList, usePagedRows } from '../api/index.js';

import { DataListCard } from './detail/DataListCard.js';
import { PageHeader } from './detail/PageHeader.js';
import { PoolsFilterBar } from './liquidity-pools/PoolsFilterBar.js';
import { POOL_COLUMN_COUNT, PoolsTable } from './liquidity-pools/PoolsTable.js';

type Filters = NonNullable<ListPoolsData['query']>;

/**
 * Liquidity-pools list page (`/liquidity-pools`) — every liquidity pool,
 * classic and Soroban alike, with asset-code search, a pool-kind filter and a
 * minimum-TVL preset filter. Cursor paginated. Wires the Figma node
 * `266:35969` layout against the `GET /liquidity-pools` endpoint as extended
 * by task 0246.
 */
export default function LiquidityPoolsListPage() {
  const { state, cursor, goNext, goPrev, setFilter, clearFilters } =
    useCursorPagination({
      filterKeys: ['asset', 'kind', 'min_tvl'],
    });
  const asset = state.filters.asset ?? '';
  const kind = state.filters.kind ?? '';
  const minTvl = state.filters.min_tvl ?? '';
  const hasFilters = asset !== '' || kind !== '' || minTvl !== '';

  const queryFilters = useMemo<Filters>(() => {
    const filters: Filters = { limit: PAGE_SIZE };
    if (asset) filters['filter[asset_code]'] = asset;
    if (kind) filters['filter[pool_kind]'] = kind;
    if (minTvl) filters['filter[min_tvl]'] = minTvl;
    return filters;
  }, [asset, kind, minTvl]);

  const { data, isLoading, isPlaceholderData, isError, error, refetch } =
    usePoolsList(cursor, queryFilters);

  const { rows, canPrev, canNext, handlePrev, handleNext } = usePagedRows(
    data,
    goNext,
    goPrev
  );

  const handleAssetChange = useCallback(
    (value: string) => setFilter('asset', value || null),
    [setFilter]
  );
  const handleKindChange = useCallback(
    (value: string) => setFilter('kind', value || null),
    [setFilter]
  );
  const handleMinTvlChange = useCallback(
    (value: string) => setFilter('min_tvl', value || null),
    [setFilter]
  );

  return (
    <Stack spacing={3}>
      <PageHeader
        title="Liquidity Pools"
        subtitle="All AMM liquidity pools on the Stellar network"
      />
      <DataListCard
        filters={
          <PoolsFilterBar
            asset={asset}
            kind={kind}
            minTvl={minTvl}
            onAssetChange={handleAssetChange}
            onKindChange={handleKindChange}
            onMinTvlChange={handleMinTvlChange}
          />
        }
        columnCount={POOL_COLUMN_COUNT}
        isLoading={isLoading}
        isReloading={isPlaceholderData}
        isError={isError}
        error={error}
        onRetry={() => void refetch()}
        rows={rows}
        renderTable={(visibleRows) => <PoolsTable rows={visibleRows} />}
        renderSkeleton={() => (
          <PoolsTable rows={[]} loading skeletonRows={PAGE_SIZE} />
        )}
        hasActiveFilters={hasFilters}
        emptyKind="pools"
        emptyNoun="pools"
        onClearFilters={clearFilters}
        canPrev={canPrev}
        canNext={canNext}
        onPrev={handlePrev}
        onNext={handleNext}
      />
    </Stack>
  );
}
