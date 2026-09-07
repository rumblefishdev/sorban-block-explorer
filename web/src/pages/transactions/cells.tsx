import { Box, Typography } from '@mui/material';
import {
  Chip,
  Dash,
  IdentifierDisplay,
  IdentifierWithCopy,
  StatusChip,
  type ExplorerTableColumn,
} from '@rumblefish/soroban-block-explorer-ui';

import { formatOperationType } from './operationTypes.js';

/**
 * Operation-type cell shared by every transaction table: the first operation
 * type as a chip, with a `+N` count when a transaction has more than one.
 */
export function OperationCell({ types }: { types: readonly string[] }) {
  if (types.length === 0) return <Dash />;
  const [first, ...rest] = types;
  return (
    <Box sx={{ display: 'inline-flex', alignItems: 'center', gap: 0.5 }}>
      <Chip size="sm" color="neutral" label={formatOperationType(first)} />
      {rest.length > 0 && (
        <Typography
          variant="bodyXsRegular"
          sx={(theme) => ({ color: theme.palette.text.tertiary })}
        >
          +{rest.length}
        </Typography>
      )}
    </Box>
  );
}

/**
 * Ledger-sequence column, identical across the transaction/event/invocation
 * tables. Generic over the row type — any row exposing `ledger_sequence`.
 */
export function ledgerColumn<
  T extends { ledger_sequence: number }
>(): ExplorerTableColumn<T> {
  return {
    id: 'ledger',
    header: 'Ledger',
    width: 120,
    cell: (row) => (
      <IdentifierDisplay value={String(row.ledger_sequence)} type="ledger" />
    ),
  };
}

/** Transaction-hash column with copy affordance, shared by the tx tables. */
export function hashColumn<
  T extends { hash: string }
>(): ExplorerTableColumn<T> {
  return {
    id: 'hash',
    header: 'Hash',
    width: 160,
    cell: (row) => <IdentifierWithCopy value={row.hash} type="transaction" />,
  };
}

/** Success/fail status chip column, shared by the tx tables. */
export function statusColumn<
  T extends { successful: boolean }
>(): ExplorerTableColumn<T> {
  return {
    id: 'status',
    header: 'Status',
    width: 120,
    cell: (row) => <StatusChip successful={row.successful} />,
  };
}
