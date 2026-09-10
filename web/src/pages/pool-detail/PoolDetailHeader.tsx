import { Box, Stack, Typography } from '@mui/material';
import type { PoolItem } from '@rumblefish/api-types';
import { Chip, IdentifierDisplay } from '@rumblefish/soroban-block-explorer-ui';

import { routes } from '../../router/routes.js';
import { PageBreadcrumb } from '../detail/PageBreadcrumb.js';
import { PoolLegIcons } from '../pool-shared/PoolLegIcons.js';

import { poolLabel } from '../pool-shared/helpers.js';
import { poolKindMeta } from '../liquidity-pools/poolKind.js';

interface PoolDetailHeaderProps {
  poolId: string;
  pool?: PoolItem;
}

export function PoolDetailHeader({ poolId, pool }: PoolDetailHeaderProps) {
  const name = pool ? poolLabel(pool.legs) : 'Liquidity pool';

  return (
    <Box>
      <PageBreadcrumb
        items={[
          { label: 'Liquidity Pools', to: routes.pools },
          { label: name },
        ]}
      />
      <Stack direction="row" spacing={1.5} alignItems="center" sx={{ mb: 0.5 }}>
        {pool && <PoolLegIcons legs={pool.legs} size={44} />}
        <Stack spacing={0.5}>
          {/* Fee badge dropped (task 0348 F9): classic pools are all
              protocol-fixed at 0.30%, so the header pill was decorative.
              The fee stays as a quiet key-value in the Summary card. */}
          <Typography variant="heading5SemiBold" component="h1">
            {name}
          </Typography>
          {/* The list badges every row with its kind; the detail page has to
              say it too, or the one page about a single pool is the only place
              that does not. Nothing else here distinguishes the two. */}
          <Stack direction="row" spacing={1} alignItems="center">
            <IdentifierDisplay value={poolId} type="pool" linked={false} />
            {pool && (
              <Chip
                size="sm"
                color={poolKindMeta(pool.pool_kind).color}
                label={poolKindMeta(pool.pool_kind).label}
              />
            )}
          </Stack>
        </Stack>
      </Stack>
    </Box>
  );
}
