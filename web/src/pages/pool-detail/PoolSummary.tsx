import { Box, Stack, Typography } from '@mui/material';
import type { PoolItem } from '@rumblefish/api-types';
import {
  formatAmount,
  IdentifierDisplay,
  IdentifierWithCopy,
} from '@rumblefish/soroban-block-explorer-ui';

import { SectionCard } from '../detail/SectionCard.js';
import { SummaryRow } from '../detail/SummaryRow.js';

import {
  assetLegLabel,
  legHref,
  poolReserves,
  reserveDotColor,
} from '../pool-shared/helpers.js';

interface AssetReserveCellProps {
  amount: string | null | undefined;
  code: string;
  dotColor: string;
  href?: string;
}

function AssetReserveCell({
  amount,
  code,
  dotColor,
  href,
}: AssetReserveCellProps) {
  const codeNode = href ? (
    <IdentifierDisplay
      value={code}
      type="asset"
      truncate={false}
      href={href}
      fontSize={12}
    />
  ) : (
    <Typography component="span" variant="bodyXsMedium">
      {code}
    </Typography>
  );

  return (
    <Stack direction="row" spacing={1} alignItems="center">
      <Box
        component="span"
        sx={{
          display: 'inline-block',
          width: 8,
          height: 8,
          borderRadius: '50%',
          bgcolor: dotColor,
          flexShrink: 0,
        }}
      />
      <Typography component="span" variant="bodyXsMedium">
        {/* LP amounts (reserves, shares) arrive pre-scaled: CH stores them as
            Decimal128(7) so `toString` already renders "750.699916", not a raw
            integer. Do NOT wrap in scaleByDecimals — unlike account balances /
            asset supply (raw Int128 + `decimals`, scaled on the FE), LP assets
            are always classic 7dp and scaled DB-side. */}
        {amount != null ? formatAmount(amount) : '—'}
      </Typography>
      {amount != null ? codeNode : null}
    </Stack>
  );
}

/** Split a list into rows of two, the layout `SummaryRow` renders. */
function chunkPairs<T>(items: readonly T[]): T[][] {
  const rows: T[][] = [];
  for (let i = 0; i < items.length; i += 2) rows.push(items.slice(i, i + 2));
  return rows;
}

interface PoolSummaryProps {
  pool: PoolItem;
}

/**
 * "Summary" key-value card on the LP detail page (Figma node `325:7192`).
 * Row layout:
 *
 *   • Pool ID — the pool's canonical identifier, copyable, full-width row
 *   • Fee % (left) │ Total shares (right)
 *   • One reserve cell (dot + amount) per leg, two to a row
 */
export function PoolSummary({ pool }: PoolSummaryProps) {
  const reserves = poolReserves(pool);

  return (
    <SectionCard title="Summary">
      <SummaryRow
        cells={[
          {
            label: 'Pool ID',
            value: (
              <IdentifierWithCopy
                value={pool.pool_id}
                type="pool"
                linked={false}
                truncate={false}
              />
            ),
          },
        ]}
      />
      <SummaryRow
        cells={[
          { label: 'Fee', value: `${formatAmount(pool.fee_percent, 2)}%` },
          {
            label: 'Total shares',
            value: formatAmount(pool.total_shares),
          },
        ]}
      />
      {/* `SummaryRow` lays out two cells per row, which the pair shape hit
          exactly. A three- or four-leg pool needs the legs chunked into rows
          instead — one row of two, then the remainder. */}
      {chunkPairs(reserves).map((row, i) => (
        <SummaryRow
          key={i}
          cells={row.map(({ leg, amount }) => {
            const code = assetLegLabel(leg);
            return {
              label: `${code} reserve`,
              value: (
                <AssetReserveCell
                  amount={amount}
                  code={code}
                  dotColor={reserveDotColor(leg)}
                  href={legHref(leg)}
                />
              ),
            };
          })}
        />
      ))}
    </SectionCard>
  );
}
