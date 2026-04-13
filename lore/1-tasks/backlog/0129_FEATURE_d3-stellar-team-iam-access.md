---
id: '0129'
title: 'D3: Stellar team read-only IAM access'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0036', '0040']
tags: [priority-low, effort-small, layer-infra, audit-gap]
milestone: 3
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — D3 acceptance criteria #3 requires Stellar team monitoring access but no task existed.'
---

# D3: Stellar team read-only IAM access

## Summary

Deliverable 3 acceptance criteria #3 requires providing the Stellar team with read-only
access to production monitoring dashboards (CloudWatch).

## Implementation

1. Create IAM role with read-only CloudWatch access.
2. Configure cross-account access or shared dashboard link.
3. Document access instructions for Stellar team.

## Acceptance Criteria

- [ ] IAM role with read-only CloudWatch access created
- [ ] Stellar team can view production dashboards and alarms
- [ ] Access documented and delivered
