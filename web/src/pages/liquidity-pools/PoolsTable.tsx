import { Box, Stack, Typography } from '@mui/material';
import type { PoolAssetLeg, PoolItem } from '@rumblefish/api-types';
import {
  Chip,
  Dash,
  EXPLORER_TABLE_ROW_HEIGHT_TALL,
  ExplorerTable,
  formatAmount,
  formatCompactAmount,
  formatCompactUsd,
  IdentifierDisplay,
  type ExplorerTableColumn,
} from '@rumblefish/soroban-block-explorer-ui';
import type { ReactNode } from 'react';

import { routes } from '../../router/routes.js';
// The labelling + linking rules apply equally to the list and the detail
// page — reuse the shared helpers rather than duplicating, to keep the
// native-asset / SAC-mirror / classic-credit precedence in one place.
import {
  assetLegLabel,
  legHref,
  poolLabel,
  poolReserves,
  reserveDotColor,
} from '../pool-shared/helpers.js';

import { PoolLegIcons } from '../pool-shared/PoolLegIcons.js';

import { poolKindMeta } from './poolKind.js';

export const POOL_COLUMN_COUNT = 6;

/** Render leg code text — wrapped in RouterLink when legHref resolves
 *  (native, classic credit, contract-id fallback); plain text otherwise (schema
 *  drift). Matches the precedence used by PoolSummary + PoolKpiStrip. */
function assetCodeNode(leg: PoolAssetLeg): ReactNode {
  const code = assetLegLabel(leg);
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

/** Colored dot for the per-leg reserves rows — color comes from the
 *  same per-asset `assetColor` hash that drives the leg `AssetIcon`. */
function AssetDot({ color }: { color: string }) {
  return (
    <Box
      component="span"
      sx={{
        display: 'inline-block',
        width: 8,
        height: 8,
        borderRadius: '50%',
        bgcolor: color,
        flexShrink: 0,
      }}
    />
  );
}

const columns: ExplorerTableColumn<PoolItem>[] = [
  {
    id: 'pool',
    header: 'Pool',
    width: 260,
    cell: (row) => {
      // A pool whose legs are not backfilled yet has no name to show. Render it
      // as the absence it is — secondary colour, like `Dash` — so the row does
      // not read as a pool actually called "Composition not indexed".
      const unindexed = row.legs.length === 0;
      const kind = poolKindMeta(row.pool_kind);
      return (
        <Stack
          direction="row"
          spacing={1}
          alignItems="center"
          sx={{ minWidth: 0 }}
        >
          <PoolLegIcons legs={row.legs} />
          <Stack spacing={0.25} sx={{ minWidth: 0 }}>
            <Typography
              variant="bodySmMedium"
              // Without `noWrap` the name is hard-clipped mid-character: the
              // cell owns the ellipsis, and a flex child inside it does not
              // inherit one. Two legs never reached the edge; a pool named by
              // truncated contract addresses does.
              noWrap
              sx={(theme) => ({
                color: unindexed
                  ? theme.palette.text.secondary
                  : theme.palette.text.primary,
              })}
            >
              {poolLabel(row.legs)}
            </Typography>
            <Stack direction="row" spacing={1} alignItems="center">
              <IdentifierDisplay
                value={row.pool_id}
                type="pool"
                href={routes.pool(row.pool_id)}
              />
              {/* The two kinds are indistinguishable in every other column,
                  and the filter above can select between them — so the row has
                  to say which one it is. Same badge the assets list wears. */}
              <Chip size="sm" color={kind.color} label={kind.label} />
            </Stack>
          </Stack>
        </Stack>
      );
    },
  },
  {
    id: 'reserves',
    header: 'Reserves',
    width: 150,
    cell: (row) => {
      // Stale pools (no fresh snapshot) come back with null reserves —
      // render an em-dash rather than "0". A pool whose legs are not indexed
      // yet gets the same treatment: the amounts cannot be attributed to
      // anything, so there is nothing honest to label them with.
      const reserves = poolReserves(row);
      if (reserves.length === 0) return <Dash />;
      if (row.reserve_a == null && row.reserve_b == null) return <Dash />;
      return (
        <Stack spacing={0.5}>
          {reserves.map(({ leg, amount }, i) => (
            <Stack key={i} direction="row" spacing={1} alignItems="center">
              <AssetDot color={reserveDotColor(leg)} />
              <Typography variant="bodyXsMedium" component="span">
                {amount != null ? formatCompactAmount(amount) : '—'}{' '}
                {assetCodeNode(leg)}
              </Typography>
            </Stack>
          ))}
        </Stack>
      );
    },
  },
  {
    id: 'tvl',
    header: 'TVL',
    align: 'right',
    width: 120,
    cell: (row) => {
      // Unpriceable pools (an untracked leg, or no fresh snapshot) come
      // back with null TVL — em-dash, consistent with the reserves column.
      if (row.tvl == null) return <Dash />;
      return (
        <Typography
          variant="bodySmMedium"
          sx={(theme) => ({ color: theme.palette.text.primary })}
        >
          {formatCompactUsd(row.tvl)}
        </Typography>
      );
    },
  },
  {
    id: 'total_shares',
    // Figma reuses the "Reserves" header for this column too. Use a
    // distinct label so screen readers (and column-mapping helpers)
    // don't see two identical headers — visually it still reads as a
    // "reserves" sibling because of the right-aligned amount + "shares"
    // unit label below.
    header: 'Total shares',
    align: 'right',
    width: 150,
    cell: (row) => {
      if (row.total_shares == null) return <Dash />;
      return (
        <Stack spacing={0.25} alignItems="flex-end">
          <Typography
            variant="bodySmMedium"
            sx={(theme) => ({ color: theme.palette.text.primary })}
          >
            {formatCompactAmount(row.total_shares)}
          </Typography>
          <Typography
            variant="bodyXsRegular"
            sx={(theme) => ({ color: theme.palette.text.secondary })}
          >
            shares
          </Typography>
        </Stack>
      );
    },
  },
  {
    id: 'participants',
    header: 'Participants',
    align: 'right',
    width: 110,
    cell: (row) => (
      <Typography
        variant="bodySmMedium"
        sx={(theme) => ({ color: theme.palette.text.primary })}
      >
        {row.participant_count != null ? (
          formatAmount(row.participant_count)
        ) : (
          <Dash />
        )}
      </Typography>
    ),
  },
];

interface PoolsTableProps {
  rows: readonly PoolItem[];
  loading?: boolean;
  skeletonRows?: number;
}

/**
 * Table for the liquidity-pools list page. Columns mirror the Figma node
 * `266:36052` design: Pool (stacked color-coded asset avatars + pair +
 * truncated id) / Reserves (per-leg) / TVL (USD, task 0199 Phase A2 —
 * issue #367's ask; em-dash when a leg is unpriceable) / Total shares
 * (right-aligned, unit label) / Participants. Fee column dropped (task
 * 0348 F9): every classic pool is protocol-fixed at 0.30%
 * (`LIQUIDITY_POOL_FEE_V18`), so a per-row Fee column carried no
 * comparative signal.
 */
export function PoolsTable({ rows, loading, skeletonRows }: PoolsTableProps) {
  return (
    <ExplorerTable
      columns={columns}
      rows={rows}
      rowKey={(row) => row.pool_id}
      rowHeight={EXPLORER_TABLE_ROW_HEIGHT_TALL}
      loading={loading}
      skeletonRows={skeletonRows}
    />
  );
}
