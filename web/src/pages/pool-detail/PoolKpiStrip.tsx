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
const UNKNOWN_SUBTITLE = 'not indexed';

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
  // "No recent snapshot" is only true of a pool that HAS snapshots. A Soroban
  // pool never does — the table is classic-only — so saying it went stale
  // describes a thing that never existed. Absence there is "not indexed".
  const stale =
    pool.latest_snapshot_ledger != null && isPoolStale(pool.latest_snapshot_at);

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
        caption={
          pool.total_shares != null
            ? 'shares outstanding'
            : stale
            ? STALE_SUBTITLE
            : UNKNOWN_SUBTITLE
        }
      />
      {/* The caption follows the VALUE, not the snapshot. A Soroban pool
          never has a snapshot, so keying off freshness told every one of them
          "no recent snapshot" while showing a current reserve — and hid the
          asset link while doing it. A value we have is never stale-captioned;
          one we do not have says which kind of absence it is. */}
      {poolReserves(pool).map(({ leg, amount }, i) => {
        const code = assetLegLabel(leg);
        const caption =
          amount != null
            ? assetSubtitle(leg, code)
            : stale
            ? STALE_SUBTITLE
            : UNKNOWN_SUBTITLE;
        return (
          <KpiCell
            key={i}
            label={`${code} reserve`}
            value={formatCompactAmount(amount)}
            caption={caption}
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
