import type { TransactionListItem } from '@rumblefish/api-types';
import {
  Dash,
  EXPLORER_TABLE_ROW_HEIGHT_TALL,
  ExplorerTable,
  formatFee,
  IdentifierDisplay,
  type ExplorerTableColumn,
} from '@rumblefish/soroban-block-explorer-ui';
import { Typography } from '@mui/material';

import {
  hashColumn,
  ledgerColumn,
  OperationCell,
  statusColumn,
} from './cells.js';
import { TransactionTime } from './TransactionTime.js';

interface TransactionsTableProps {
  rows: readonly TransactionListItem[];
  loading?: boolean;
  skeletonRows?: number;
}

const columns: ExplorerTableColumn<TransactionListItem>[] = [
  hashColumn<TransactionListItem>(),
  ledgerColumn<TransactionListItem>(),
  {
    id: 'source',
    header: 'Source account',
    width: 160,
    cell: (row) =>
      row.source_account ? (
        <IdentifierDisplay value={row.source_account} type="account" />
      ) : (
        <Dash />
      ),
  },
  {
    id: 'operation',
    header: 'Operation',
    width: 190,
    cell: (row) => <OperationCell types={row.operation_types} />,
  },
  statusColumn<TransactionListItem>(),
  {
    id: 'fee',
    header: 'Fee',
    width: 140,
    cell: (row) => (
      <Typography component="span" variant="bodySmRegular">
        {formatFee(row.fee_charged)}
      </Typography>
    ),
  },
  {
    id: 'time',
    header: 'Time',
    width: 210,
    cell: (row) => <TransactionTime createdAt={row.created_at} />,
  },
];

/** Number of columns — used to size the loading skeleton consistently. */
export const TRANSACTION_COLUMN_COUNT = columns.length;

/**
 * The Transactions list table — hash, ledger, source account, operation,
 * status, net settled, fee and time columns, per the Figma design.
 */
export function TransactionsTable({
  rows,
  loading,
  skeletonRows,
}: TransactionsTableProps) {
  return (
    <ExplorerTable
      columns={columns}
      rows={rows}
      rowKey={(row) => row.hash}
      rowHeight={EXPLORER_TABLE_ROW_HEIGHT_TALL}
      loading={loading}
      skeletonRows={skeletonRows}
    />
  );
}
