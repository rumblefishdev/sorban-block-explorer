import { Box, Stack, Typography } from '@mui/material';
import type { PoolItem } from '@rumblefish/api-types';
import { IdentifierDisplay } from '@rumblefish/soroban-block-explorer-ui';

import { routes } from '../../router/routes.js';
import { PageBreadcrumb } from '../detail/PageBreadcrumb.js';
import { PoolLegIcons } from '../pool-shared/PoolLegIcons.js';

import { poolLabel } from '../pool-shared/helpers.js';

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
          <IdentifierDisplay value={poolId} type="pool" linked={false} />
        </Stack>
      </Stack>
    </Box>
  );
}
