import { Box } from '@mui/material';
import type { PoolAssetLeg } from '@rumblefish/api-types';

import { AssetIcon } from '../assets/AssetIcon.js';
import { assetLegLabel } from './helpers.js';

/**
 * A pool's legs as overlapping coin avatars. Each uses the shared `AssetIcon`,
 * which colours itself per asset identity (`assetColor`) — the same asset reads
 * the same colour here, on its detail page, and in the reserve dots. The 2px
 * ring + negative margin live here, in the group layout, so `AssetIcon` stays a
 * plain single-asset avatar.
 *
 * Renders however many legs the pool has. It was `PoolAssetPair`, taking `a`
 * and `b`: a classic pool has exactly two legs, but a soroban one has two to
 * four, and a three-leg stable pool had no way to show its third asset.
 */
export function PoolLegIcons({
  legs,
  size = 32,
}: {
  legs: readonly PoolAssetLeg[];
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
      {/* A pool with no indexed legs still gets one avatar: without it the
          column loses its left anchor and every text line in that row shifts,
          which reads as a layout bug rather than as missing data. `AssetIcon`
          renders a nameless asset as `?`. */}
      {legs.length === 0 && <AssetIcon code={null} size={size} />}
      {legs.map((leg, i) => (
        <AssetIcon
          // Legs are positional and an asset can repeat across pools but not
          // within one; the index is the only stable identity a leg has.
          key={i}
          code={assetLegLabel(leg)}
          iconUrl={leg.icon_url}
          size={size}
        />
      ))}
    </Box>
  );
}
