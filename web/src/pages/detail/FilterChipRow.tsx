import { Stack } from '@mui/material';
import { Chip } from '@rumblefish/soroban-block-explorer-ui';

export interface FilterChipOption {
  label: string;
  value: string;
}

/**
 * A single-select row of filter chips — the app's control for "pick one class
 * of thing". The assets list (type), the contracts list (type) and the pools
 * list (kind) each held a verbatim copy of it, so the affordance and the
 * `aria-pressed` contract could drift between three lists that look identical.
 *
 * The leading `Divider` stays with each filter bar: where the row sits relative
 * to the search box is a layout decision, and the chips are the part that has
 * to behave the same everywhere.
 *
 * `''` is the "all" option by convention — the callers map it back to "no
 * filter" — so the active chip is decided by value equality, not by index.
 */
export function FilterChipRow({
  options,
  value,
  onChange,
}: {
  options: readonly FilterChipOption[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
      {options.map((option) => {
        const active = option.value === value;
        return (
          <Chip
            key={option.value}
            label={option.label}
            size="lg"
            color={active ? 'accent' : 'neutral'}
            clickable
            onClick={() => onChange(option.value)}
            aria-pressed={active}
          />
        );
      })}
    </Stack>
  );
}
