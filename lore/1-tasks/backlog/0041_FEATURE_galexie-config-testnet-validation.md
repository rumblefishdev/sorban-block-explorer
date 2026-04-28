---
id: '0041'
title: 'Galexie testnet/mainnet operational validation'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0001', '0034', '0145']
tags: [priority-high, effort-small, layer-indexing, layer-infra]
milestone: 1
links:
  - infra/src/lib/stacks/ingestion-stack.ts
  - infra/src/lib/stacks/cloudwatch-stack.ts
  - lore/3-wiki/backfill-execution-plan.md
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — gap identified during milestone alignment (task 0085)'
  - date: 2026-04-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Rescoped — Captive Core TOML, network passphrase mapping, ECS
      Fargate task definitions (live + backfill), S3 output wiring,
      VPC + SG (port 11625), and CloudWatch alarms have all shipped
      via the CDK stacks (`ingestion-stack.ts:169-265`,
      `network-stack.ts:122`, `cloudwatch-stack.ts:100+`). Original ACs
      #1 (config files) and parts of #2 are already covered. Remaining
      work is purely operational: deploy to testnet, observe ledger
      cadence, verify S3→Lambda trigger end-to-end, then re-validate
      with mainnet config. Effort medium → small.
---

# Galexie testnet/mainnet operational validation

## Summary

Empirically validate the already-deployed Galexie ingestion stack: deploy
to testnet, observe ledger cadence, confirm the S3→Lambda trigger fires,
then re-validate with mainnet config before live ingest is permitted.

## Context

Task 0034 specced the CDK infra; the work landed in
[`infra/src/lib/stacks/ingestion-stack.ts`](../../../infra/src/lib/stacks/ingestion-stack.ts)
along with Captive Core TOML emission, network-passphrase-keyed network
selection, ECS Fargate task definitions for both live and backfill
flavors, the S3 ledger-bucket wiring, the ECS-to-Stellar-overlay
security-group rule (port 11625), and a CloudWatch lag alarm. Originally
this task was to write that config; it's now to **prove it works on the
real networks** before the [backfill execution plan](../../3-wiki/backfill-execution-plan.md)
T7 cutover.

## Implementation Plan

### Step 1: Testnet deploy + observation

Deploy `ingestion-stack` (and dependencies) to a testnet env. Watch the
ledger bucket and CloudWatch:

- LedgerCloseMeta XDR files land in S3 at testnet ledger cadence
  (~5-6 s).
- Object keys match the layout the indexer Lambda expects.
- S3 `ObjectCreated:Put` event triggers the Ledger Processor Lambda
  (visible in CloudWatch invocations metric).
- `GalexieLagAlarm` does **not** fire under steady state.

### Step 2: Mainnet config flip + connectivity check

Switch config to mainnet. Without enabling continuous live writes yet,
confirm:

- Captive Core can sync against mainnet history archive URLs.
- Stellar overlay handshake completes (port 11625 outbound from the
  Galexie task).
- A short bounded run produces well-formed ledger files in S3.

### Step 3: Tear-down

Stop the validation services if not part of permanent staging — runtime
costs accumulate quickly on continuous Captive Core sync.

## Acceptance Criteria

- [ ] Testnet: at least one hour of consecutive `LedgerCloseMeta` files
      in S3 with no gaps
- [ ] Testnet: `Processor invocations / min` matches Galexie write rate
      (no broken trigger)
- [ ] Testnet: `GalexieLagAlarm` did not fire under steady state
- [ ] Mainnet: history archive sync proven (logs show fetched checkpoint)
- [ ] Mainnet: short bounded run confirms ledger files written to S3 in
      the same layout as testnet
- [ ] Findings recorded in [`backfill-execution-plan.md`](../../3-wiki/backfill-execution-plan.md)
      open-questions section

## Out of scope

- Continuous live ingestion — gated separately, after backfill cutover
- CDK config changes — already shipped (see frontmatter `links:`); this
  task only consumes them
