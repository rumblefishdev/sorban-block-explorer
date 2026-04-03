---
id: '0103'
title: 'Research: backfill cost estimation with verified data'
type: RESEARCH
status: active
related_adr: []
related_tasks: ['0030', '0029', '0028', '0034']
tags: [priority-high, effort-medium, layer-database, research]
milestone: 1
links: []
history:
  - date: 2026-04-03
    status: active
    who: stkrolikiewicz
    note: >
      Created to produce a rigorous, source-verified backfill plan.
      Initial estimates in lore/3-wiki/backfill-plan.md were based on
      unverified assumptions. This task replaces them with hard data.
---

# Research: backfill cost estimation with verified data

## Summary

Produce a detailed, source-verified backfill cost estimation for ingesting all Soroban-era ledgers. Every number must have a source (AWS docs URL, Stellar API response, benchmark result). No "estimates" without explicit error bars.

## Status: Active

## Research Questions

1. **How many ledgers?** — Exact current mainnet ledger sequence. Exact Soroban activation ledger. Delta.
2. **How big are ledgers?** — Actual compressed XDR file sizes from Stellar public data lake.
3. **How fast can we process?** — Lambda processing time per ledger (needs benchmark or comparable data).
4. **What does RDS t4g.micro actually sustain?** — Baseline IOPS, CPU, write throughput under sustained load.
5. **What does Fargate cost?** — Exact pricing for ARM64 4vCPU/16GB in us-east-1.
6. **What does the full pipeline cost?** — End-to-end with buffers.
7. **How long will it take?** — With realistic bottleneck analysis.

## Acceptance Criteria

- [ ] Every number has a URL source or is derived from a verified source with calculation shown
- [ ] Stellar mainnet current ledger verified from live API
- [ ] Soroban activation ledger verified from official source
- [ ] AWS pricing verified from current pricing pages
- [ ] RDS t4g.micro baseline performance documented from AWS docs
- [ ] Realistic buffer (20-30%) applied to all time estimates
- [ ] Final document replaces lore/3-wiki/backfill-plan.md
