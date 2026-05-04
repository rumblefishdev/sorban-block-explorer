=== ledgers archive-diff ===
# audit-harness Phase 2c — DB ↔ archive XDR re-parse

**Timestamp:** 2026-04-30T17:11:35.060639+00:00
**Table:** `ledgers`
**Sampled:** 100
**Mismatched rows:** 0
**Unreachable:** 0

✓ All sampled rows match the archive XDR field-for-field.

=== liquidity_pools archive-diff ===
error: invalid value 'liquidity-pools' for '--table <TABLE>'
  [possible values: ledgers]

For more information, try '--help'.

=== horizon-diff transactions ===
# audit-harness Phase 2a — DB ↔ Horizon diff

**Timestamp:** 2026-04-30T17:11:53.280123+00:00
**Table:** `transactions`
**Sampled:** 100
**Mismatched rows:** 0
**Unreachable on Horizon:** 0

✓ All sampled rows match Horizon field-for-field.

=== horizon-diff accounts ===
# audit-harness Phase 2a — DB ↔ Horizon diff

**Timestamp:** 2026-04-30T17:12:01.528216+00:00
**Table:** `accounts`
**Sampled:** 100
**Mismatched rows:** 0
**Unreachable on Horizon:** 5

DONE
