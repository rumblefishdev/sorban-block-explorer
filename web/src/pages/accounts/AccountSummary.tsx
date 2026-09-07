import { Stack, Typography } from '@mui/material';

import type { AccountDetailResponse } from '@rumblefish/api-types';
import {
  formatAmount,
  IdentifierDisplay,
  IdentifierWithCopy,
} from '@rumblefish/soroban-block-explorer-ui';

import { useFederatedName } from '../../search/useFederation.js';
import { SectionCard } from '../detail/SectionCard.js';
import { SummaryRow } from '../detail/SummaryRow.js';

/**
 * Account summary card — account ID (copyable), sequence number, and the
 * first / last seen ledgers, each linked to its ledger detail page.
 */
export function AccountSummary({
  account,
}: {
  account: AccountDetailResponse;
}) {
  // SEP-2 name this account's own home domain claims for it (issue #363).
  // Shown only when both sides agree; an account with no home domain, or a
  // domain that does not federate, simply has no name beside its key.
  const federatedName = useFederatedName(
    account.account_id,
    account.home_domain ?? ''
  );

  return (
    <SectionCard title="Summary">
      <SummaryRow
        cells={[
          {
            label: 'Account ID',
            value: (
              // Beside the key, not in a row of its own. The name arrives from
              // two network round-trips, so a row would appear seconds after
              // the card had settled and push everything below it down —
              // reserving the row instead only moves the jump to the accounts
              // whose domain answers nothing. Inline, the height is fixed
              // before anything is asked and only this cell grows sideways.
              <Stack
                direction="row"
                spacing={1}
                alignItems="center"
                flexWrap="wrap"
              >
                <IdentifierWithCopy
                  value={account.account_id}
                  type="account"
                  linked={false}
                  truncate={false}
                />
                {federatedName != null && (
                  <Typography
                    variant="bodySmRegular"
                    component="span"
                    sx={(theme) => ({ color: theme.palette.text.tertiary })}
                  >
                    {federatedName}
                  </Typography>
                )}
              </Stack>
            ),
          },
        ]}
      />
      <SummaryRow
        cells={[
          {
            label: 'Sequence number',
            value: formatAmount(account.sequence_number),
          },
        ]}
      />
      <SummaryRow
        cells={[
          {
            label: 'First seen ledger',
            value: (
              <IdentifierDisplay
                value={String(account.first_seen_ledger)}
                type="ledger"
              />
            ),
          },
          {
            label: 'Last seen ledger',
            value: (
              <IdentifierDisplay
                value={String(account.last_seen_ledger)}
                type="ledger"
              />
            ),
          },
        ]}
      />
    </SectionCard>
  );
}
