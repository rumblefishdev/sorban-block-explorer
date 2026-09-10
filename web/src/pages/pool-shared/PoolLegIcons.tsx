import { Box } from '@mui/material';
import type { PoolAssetLeg } from '@rumblefish/api-types';

import { AssetIcon } from '../assets/AssetIcon.js';
import { assetLegLabel } from './helpers.js';

/**
 * A liquidity pool is two assets. Render each leg with the shared
 * `AssetIcon`, which colours itself per asset identity (`assetColor`) — the
 * same asset reads the same colour here, on its detail page, and in the
 * reserve dots. Laid out as an overlapping coin pair: the 2px ring +
 * negative margin live here, in the pair layout, so `AssetIcon` stays a
 * plain single-asset avatar.
 */
export function PoolAssetPair({
  a,
  b,
  size = 32,
}: {
  a: PoolAssetLeg;
  b: PoolAssetLeg;
  size?: number;
}) {
  return (
    <Box
      sx={(theme) => ({
        display: 'flex',
        alignItems: 'center',
        '& .MuiAvatar-root': {
          // The ring separates the overlapping avatars from the surface
          // behind them. Dark matches the card fill, so it reads as negative
          // space; light sits on white, where that trick is invisible and it
          // needs a real hairline instead.
          border:
            theme.palette.mode === 'light'
              ? `1px solid ${theme.palette.stroke.default}`
              : `2px solid ${theme.palette.surface.grayMain}`,
        },
        '& .MuiAvatar-root:not(:first-of-type)': { marginLeft: '-8px' },
      })}
    >
      <AssetIcon code={assetLegLabel(a)} iconUrl={a.icon_url} size={size} />
      <AssetIcon code={assetLegLabel(b)} iconUrl={b.icon_url} size={size} />
    </Box>
  );
}
