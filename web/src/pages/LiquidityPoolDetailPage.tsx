import { Stack } from '@mui/material';
import {
  DetailErrorState,
  isPoolIdentifier,
  NotFoundState,
  SectionErrorBoundary,
} from '@rumblefish/soroban-block-explorer-ui';
import type { ReactNode } from 'react';
import { useParams } from 'react-router-dom';

import { usePoolDetail } from '../api/index.js';

import { PoolCharts } from './pool-detail/PoolCharts.js';
import { PoolDetailHeader } from './pool-detail/PoolDetailHeader.js';
import { PoolDetailSkeleton } from './pool-detail/PoolDetailSkeleton.js';
import { PoolKpiStrip } from './pool-detail/PoolKpiStrip.js';
import { PoolParticipants } from './pool-detail/PoolParticipants.js';
import { PoolSummary } from './pool-detail/PoolSummary.js';
import { PoolActivity } from './pool-detail/PoolActivity.js';

/**
 * Liquidity-pool detail page (`/liquidity-pools/:id`). Composes the
 * Figma-defined sections from top to bottom:
 *
 *   1. Header (breadcrumb, pair name, Active/Stale badge, truncated id)
 *   2. KPI strip (Total shares, A-leg reserve, B-leg reserve, participants)
 *   3. Summary (key-value rows for Pool ID, Fee, Total shares, reserves)
 *   4. Activity chart (TVL/Volume/Fees tabs, 1D/7D/30D/1Y range)
 *   5. Pool participants table
 *   6. Recent transactions table
 *
 * Each section is fetched by its own query and wrapped in a
 * `SectionErrorBoundary` so one failing section never collapses
 * unrelated siblings.
 */
export default function LiquidityPoolDetailPage() {
  // The `= ''` fallback is only here to narrow `id` from
  // `string | undefined` (the `useParams` return type) down to `string`
  // for the hooks below. React Router guarantees `:id` is present and
  // non-empty whenever this component mounts, so the fallback value is
  // never actually observed at runtime.
  const { id = '' } = useParams<{ id: string }>();
  const poolId = id;
  // A pool is addressed by whichever form its kind uses — `L…` for a classic
  // pool, `C…` for a Soroban one, which IS a contract. Validate up-front so a
  // malformed id renders the entity-specific NotFoundState instead of firing a
  // doomed request; `usePoolDetail` skips the network when the id is empty.
  const validPoolId = isPoolIdentifier(poolId);
  const detail = usePoolDetail(validPoolId ? poolId : '');
  if (!validPoolId) {
    return <NotFoundState entity="liquidity-pool" identifier={poolId} />;
  }

  if (detail.isLoading) {
    return <PoolDetailSkeleton />;
  }

  let summarySection: ReactNode = null;
  let kpiSection: ReactNode = null;
  if (detail.isError) {
    summarySection = (
      <DetailErrorState
        error={detail.error}
        entity="liquidity-pool"
        identifier={poolId}
        onRetry={() => void detail.refetch()}
        py={6}
      />
    );
  } else if (detail.data) {
    kpiSection = <PoolKpiStrip pool={detail.data} />;
    summarySection = <PoolSummary pool={detail.data} />;
  }

  return (
    <Stack spacing={3}>
      <PoolDetailHeader poolId={poolId} pool={detail.data} />

      {kpiSection != null && (
        <SectionErrorBoundary sectionName="pool-kpi-strip">
          {kpiSection}
        </SectionErrorBoundary>
      )}
      <SectionErrorBoundary sectionName="pool-summary">
        {summarySection}
      </SectionErrorBoundary>
      {/* Gate the sub-sections on resolved parent data so their queries never
          fire while the pool is still loading — a parent 404 then produces
          zero sub-section 404s. */}
      {detail.data != null && (
        <>
          <SectionErrorBoundary sectionName="pool-charts">
            <PoolCharts poolId={poolId} />
          </SectionErrorBoundary>
          <SectionErrorBoundary sectionName="pool-participants">
            <PoolParticipants poolId={poolId} />
          </SectionErrorBoundary>
          <SectionErrorBoundary sectionName="pool-transactions">
            <PoolActivity poolId={poolId} pool={detail.data} />
          </SectionErrorBoundary>
        </>
      )}
    </Stack>
  );
}
