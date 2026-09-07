import { Typography } from '@mui/material';
import type { AccountTransactionItem } from '@rumblefish/api-types';
import {
  EXPLORER_TABLE_ROW_HEIGHT_TALL,
  ExplorerTable,
  formatFee,
  PaginationControls,
  QueryErrorState,
  type SortDirection,
  TableEmptyState,
  useCursorPagination,
  type ExplorerTableColumn,
} from '@rumblefish/soroban-block-explorer-ui';
import { useCallback, type ReactNode } from 'react';

import { useAccountTransactions, usePagedRows } from '../../api/index.js';
import { SectionCard } from '../detail/SectionCard.js';
import {
  hashColumn,
  ledgerColumn,
  OperationCell,
  statusColumn,
} from '../transactions/cells.js';
import { TransactionTime } from '../transactions/TransactionTime.js';

const columns: ExplorerTableColumn<AccountTransactionItem>[] = [
  hashColumn<AccountTransactionItem>(),
  ledgerColumn<AccountTransactionItem>(),
  {
    id: 'operation',
    header: 'Operation',
    width: 190,
    cell: (row) => <OperationCell types={row.operation_types} />,
  },
  statusColumn<AccountTransactionItem>(),
  {
    id: 'fee',
    header: 'Fee',
    align: 'right',
    width: 140,
    cell: (row) => (
      <Typography
        component="span"
        variant="bodySmMedium"
        sx={(theme) => ({ color: theme.palette.text.primary })}
      >
        {formatFee(row.fee_charged)}
      </Typography>
    ),
  },
  {
    id: 'time',
    header: 'Time',
    sortable: true,
    width: 210,
    cell: (row) => <TransactionTime createdAt={row.created_at} />,
  },
];

/**
 * Recent transactions section of the account detail page — a paginated table
 * of transactions involving the account, fetched independently of the
 * account summary so a failure here never collapses the rest of the page.
 */
export function AccountTransactions({ accountId }: { accountId: string }) {
  // Cursors are account-scoped — `resetKey` drops the URL cursor when
  // the user navigates to a different account.
  const { state, cursor, goNext, goPrev, setSort } = useCursorPagination({
    resetKey: accountId,
  });
  // Sort lives in the URL `sort` (column) + `dir` (direction) params via
  // `setSort`, so it survives reload / deep links and stays paired with
  // the cursor.
  const sortDir = state.sortDir;

  const handleSortChange = useCallback(
    // Column id comes from the table; `setSort` writes `?sort=&dir=` and
    // resets the cursor.
    (id: string, next: SortDirection) => setSort(id, next),
    [setSort]
  );

  const { data, isLoading, isPlaceholderData, isError, error, refetch } =
    useAccountTransactions(accountId, cursor, sortDir);

  const { rows, canPrev, canNext, handlePrev, handleNext } = usePagedRows(
    data,
    goNext,
    goPrev
  );

  let body: ReactNode;
  if (isLoading || isPlaceholderData) {
    body = (
      <ExplorerTable
        columns={columns}
        rows={[]}
        rowKey={(row) => row.hash}
        loading
        skeletonRows={20}
        rowHeight={EXPLORER_TABLE_ROW_HEIGHT_TALL}
      />
    );
  } else if (isError) {
    body = <QueryErrorState error={error} onRetry={() => void refetch()} />;
  } else if (rows.length === 0) {
    body = <TableEmptyState kind="transactions" py={6} />;
  } else {
    body = (
      <ExplorerTable
        columns={columns}
        rows={rows}
        rowKey={(row) => row.hash}
        sortBy="time"
        sortDir={sortDir}
        onSortChange={handleSortChange}
        rowHeight={EXPLORER_TABLE_ROW_HEIGHT_TALL}
      />
    );
  }

  return (
    <SectionCard title="Recent transactions">
      {body}
      <PaginationControls
        caption="Latest results"
        canPrev={canPrev}
        canNext={canNext}
        onPrev={handlePrev}
        onNext={handleNext}
      />
    </SectionCard>
  );
}
