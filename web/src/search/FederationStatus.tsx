import { Typography } from '@mui/material';

/**
 * The one sentence both search surfaces show about a federated address.
 *
 * They lay it out differently — a row inside the header listbox, a panel
 * inside the results card — but the wording is the same decision three times
 * over: offer, in flight, or the reason it failed. Kept here so a change to
 * the failure copy, the one thing this feature argues about most, cannot land
 * in one place and not the other.
 */
export function FederationStatus({
  address,
  domain,
  armed,
  failure,
}: {
  address: string;
  domain: string;
  armed: boolean;
  failure: string | null;
}) {
  return (
    <Typography
      variant="bodyXsRegular"
      component="span"
      role="status"
      sx={(theme) => ({
        display: 'block',
        color:
          failure != null
            ? theme.palette.text.error
            : theme.palette.text.tertiary,
      })}
    >
      {failure ??
        (armed
          ? `Asking ${domain}…`
          : `Federated address — look it up with ${domain}`)}
    </Typography>
  );
}
