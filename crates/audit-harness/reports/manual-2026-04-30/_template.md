# EXX — `<METHOD> <path>`

**Date:** 2026-04-30
**Auditor:** stkrolikiewicz (following [`manual-endpoint-audit.md`](../../../../lore/3-wiki/manual-endpoint-audit.md))
**Dataset:** 30k smoke (mainnet ledgers 62016000–62046000), develop binary
post 0173 + 0177 + 0181 + 0182 + 0183 + 0178 + 0179 fix stack.
**SQL spec:** [`<NN>_<file>.sql`](../../../../docs/architecture/database-schema/endpoint-queries/<NN>_<file>.sql)
**Frontend spec:** [`frontend-overview.md` §6.X](../../../../docs/architecture/frontend/frontend-overview.md)

## Step 1 — Run endpoint SQL

<!-- ./run_endpoint.sh NN, capture stdout, note row count + sample -->

## Step 2 — Response shape vs frontend §6.X

| Frontend field | DB SQL output | Status |
| --- | --- | --- |
| ... | ... | ✓ / ⚠ / ✗ |

## Step 3 — Sample (N rows)

<!-- N=2-3 picked by hand, biased for diversity -->

| key | field | value |
| --- | --- | --- |

## Step 4 — Cross-check

### Source A: <stellar.expert | stellarchain.io | Horizon>

| key | field | DB | external | match |
| --- | --- | --- | --- | --- |

### Source B: <second source>

<!-- when first source insufficient -->

## Step 5 — Findings

### ✓ Matches

### ⚠ Drift (acceptable)

### ✗ Mismatches

## Bug spawns

<!-- list of NNNN_BUG_<slug>.md tasks spawned, or "None" -->

## Hand-off

<!-- per runbook step 5: stop, hand back to user for confirmation -->

## Cross-link to automated harness

<!-- which harness phases / runs cover the same surface -->
