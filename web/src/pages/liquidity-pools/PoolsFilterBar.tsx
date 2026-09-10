import {
  Box,
  Divider,
  MenuItem,
  Select,
  type SelectChangeEvent,
} from '@mui/material';
import { DebouncedField } from '@rumblefish/soroban-block-explorer-ui';

import { POOL_KIND_FILTERS } from './poolKind.js';

import { FilterChipRow } from '../detail/FilterChipRow.js';

/**
 * The Min-TVL preset row stays hidden: `filter[min_tvl]` is REJECTED by
 * the API (400). Pool TVL is computed at read from off-chain prices
 * (task 0199), so it cannot filter page membership — the old SQL
 * pre-filter read a snapshot column that is never written and silently
 * returned an empty page. Re-enabling this needs server-side TVL for ALL
 * pools per request, i.e. the prices-side materialized series; until then
 * the row *would* break the list, not merely read as broken.
 */
const TVL_FILTER_ENABLED = false;

export const TVL_PRESETS: ReadonlyArray<{ value: string; label: string }> = [
  { value: '', label: 'Any TVL' },
  { value: '10000', label: 'Min $10,000' },
  { value: '100000', label: 'Min $100,000' },
  { value: '1000000', label: 'Min $1,000,000' },
];

interface PoolsFilterBarProps {
  /** Asset-code search value (URL key `asset`, API `filter[asset_code]`). */
  asset: string;
  /** Active pool kind, or `''` for "All pools" (URL key `kind`). */
  kind: string;
  /** Active TVL preset (URL key `min_tvl`, API `filter[min_tvl]`). */
  minTvl: string;
  onAssetChange: (value: string) => void;
  onKindChange: (value: string) => void;
  onMinTvlChange: (value: string) => void;
}

/**
 * Filter bar for the liquidity-pools list — text input for asset-code
 * search, a pool-kind chip row, plus a TVL preset dropdown. Geometry and
 * surface tokens match Figma node `266:36052` (search 400px, TVL 280px,
 * alt-gray surface with bottom divider).
 */
export function PoolsFilterBar({
  asset,
  kind,
  minTvl,
  onAssetChange,
  onKindChange,
  onMinTvlChange,
}: PoolsFilterBarProps) {
  const handleTvlChange = (event: SelectChangeEvent<string>) => {
    onMinTvlChange(event.target.value);
  };

  return (
    <Box
      sx={(theme) => ({
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        gap: 2,
        p: 2,

        backgroundColor: theme.palette.surface.grayMainAlt,
        borderBottom: `1px solid ${theme.palette.stroke.default}`,
      })}
    >
      <DebouncedField
        value={asset}
        placeholder="Filter by asset or pair, e.g. USDC/XLM"
        ariaLabel="Filter by asset or pair"
        width={400}
        onCommit={onAssetChange}
      />
      <Divider
        orientation="vertical"
        flexItem
        sx={{ display: { xs: 'none', sm: 'block' }, my: 0.5 }}
      />
      <FilterChipRow
        options={POOL_KIND_FILTERS}
        value={kind}
        onChange={onKindChange}
      />
      {TVL_FILTER_ENABLED && (
        <Select
          value={minTvl}
          onChange={handleTvlChange}
          aria-label="Minimum TVL"
          size="small"
          // `displayEmpty` keeps the "Any TVL" label visible when no
          // preset is active — otherwise MUI renders an empty box and
          // the dropdown's purpose is invisible.
          displayEmpty
          renderValue={(value) =>
            TVL_PRESETS.find((opt) => opt.value === value)?.label ?? 'Any TVL'
          }
          sx={{ width: 280, maxWidth: '100%' }}
        >
          {TVL_PRESETS.map((option) => (
            <MenuItem key={option.value || 'any'} value={option.value}>
              {option.label}
            </MenuItem>
          ))}
        </Select>
      )}
    </Box>
  );
}
