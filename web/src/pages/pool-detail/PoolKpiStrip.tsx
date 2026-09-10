import { Stack } from '@mui/material';
import type { PoolAssetLeg, PoolItem } from '@rumblefish/api-types';
import {
  formatCompactAmount,
  formatInteger,
  IdentifierDisplay,
} from '@rumblefish/soroban-block-explorer-ui';
import type { ReactNode } from 'react';

import { KpiCell } from '../detail/KpiCell.js';

import {
  assetLegLabel,
  isPoolStale,
  legHref,
  poolReserves,
  reserveDotColor,
} from '../pool-shared/helpers.js';

const STALE_SUBTITLE = 'no recent snapshot';

interface PoolKpiStripProps {
  pool: PoolItem;
}

/**
 * KPI strip above the Summary card on the LP detail page — total shares, one
 * cell per leg reserve, and participant count. Reserves render with compact
 * notation (`1.2M`, `480K`); the subtitle carries the asset code so the value
 * reads cleanly without units stacked on top.
 *
 * Stale pools (no fresh snapshot in 7 days) come back with null reserves
 * and shares — those cells render as "—". `participant_count` stays
 * accurate regardless of freshness (per task 0246).
 */
function assetSubtitle(leg: PoolAssetLeg, code: string): ReactNode {
  const href = legHref(leg);
  if (!href) return code;
  return (
    <IdentifierDisplay
      value={code}
      type="asset"
      truncate={false}
      href={href}
      fontSize="inherit"
    />
  );
}

export function PoolKpiStrip({ pool }: PoolKpiStripProps) {
  const stale = isPoolStale(pool.latest_snapshot_at);

  return (
    <Stack
      direction={{ xs: 'column', sm: 'row' }}
      // The strip was exactly four cells; it is now two plus one per leg, so a
      // four-leg pool puts six across. Wrapping keeps every reserve visible
      // rather than compressing the labels past reading — `rowGap` because
      // `spacing` only sets the gap along the main axis.
      flexWrap="wrap"
      spacing={{ xs: 2, sm: 3 }}
      sx={{ width: '100%', rowGap: { xs: 2, sm: 3 } }}
    >
      <KpiCell
        label="Total shares"
        value={formatCompactAmount(pool.total_shares)}
        caption={stale ? STALE_SUBTITLE : 'shares outstanding'}
      />
      {poolReserves(pool).map(({ leg, amount }, i) => {
        const code = assetLegLabel(leg);
        return (
          <KpiCell
            key={i}
            label={`${code} reserve`}
            value={formatCompactAmount(amount)}
            caption={stale ? STALE_SUBTITLE : assetSubtitle(leg, code)}
            valueColor={reserveDotColor(leg)}
          />
        );
      })}
      <KpiCell
        label="Participants"
        value={formatInteger(pool.participant_count)}
        caption="liquidity providers"
      />
    </Stack>
  );
}
