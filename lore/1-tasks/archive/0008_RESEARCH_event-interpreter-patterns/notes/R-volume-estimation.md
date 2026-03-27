---
prefix: R
title: Volume Estimation and Lambda Performance
status: mature
spawned_from: null
spawns: []
---

# Volume Estimation and Lambda Performance

Research into expected Soroban event volumes and implications for the Lambda-based Event Interpreter.

## Sources

- [Stellar Expert: Network Statistics](../sources/stellar-expert-network-stats.md) — operations/day, Soroban invocations, SDF reports
- [Stellar Dashboard: Network Metrics](../sources/stellar-dashboard-stats.md) — live network stats, TPS roadmap
- [AWS: Making retries safe with idempotent APIs](../sources/aws-idempotent-apis.md) — Lambda processing patterns

## Stellar Network Baseline (from SDF reports, Q3 2025)

- **Operations/day**: ~9.9M (annual avg), ~11M+ (Q3 2025)
- **Ledger close time**: 5.76 seconds → ~15,000 ledgers/day
- **Soroban invocations/day**: ~1M (Q3 2025, +700% QoQ growth)
- **Theoretical max TPS**: 3,000 (post-Whisk upgrade, Sept 2025)
- **Daily transactions**: 1.5–2.5M (H1 2025 baseline)

## Soroban Event Volume

Key observations:

- ~1M Soroban invocations/day = ~11.6 invocations/second average
- A single contract invocation can emit 0-N events (DEX swaps: 2-4 events)
- Average ~2-3 events per invocation → ~2-3M events/day
- **Growth trajectory**: +700% QoQ in Q3 2025 — must size for continued growth

### Per 5-Minute Window

| Scenario           | Events          | Derivation                                    |
| ------------------ | --------------- | --------------------------------------------- |
| Low activity       | ~3,000-5,000    | Off-peak hours, ~10 inv/sec × 2 events × 300s |
| Normal activity    | ~7,000-10,000   | Average ~12 inv/sec × 2.5 events × 300s       |
| Peak activity      | ~20,000-50,000  | High trading, ~30 inv/sec × 3 events × 300s   |
| Future (2x growth) | ~40,000-100,000 | Accounting for continued QoQ growth           |

## Lambda Performance Assessment

| Metric                                | Estimate                                     |
| ------------------------------------- | -------------------------------------------- |
| Event interpretation time (per event) | < 1ms (pattern matching + string formatting) |
| DB read (fetch events batch)          | 50-200ms (indexed query on `id`)             |
| DB write (batched upsert)             | 100-500ms (depends on batch size)            |
| **Total Lambda execution time**       | **1-3s (normal), up to 10s (peak/future)**   |
| Lambda memory                         | 256MB more than sufficient                   |
| Lambda timeout                        | Set to 60s (generous margin)                 |

## Key Conclusions

1. **Volume is manageable today.** At normal load (~7-10K events/5min), a single Lambda invocation handles everything. Even at peak (~50K events), processing completes well within timeout.

2. **Growth is aggressive.** +700% QoQ in Q3 2025 means sizing must account for continued growth. At 2x current volume, peak batches reach ~100K events — still manageable but worth monitoring.

3. **Batch DB writes.** A single multi-row `INSERT ... ON CONFLICT` with 10,000 rows completes in ~1 second on PostgreSQL.

4. **Cold start is the main latency concern.** Node.js runtime cold starts are 200-500ms. Warm Lambdas execute in < 3s total for normal load.

5. **5-minute cadence is appropriate.** With ~7-10K events per window, 5 minutes provides a good batch size. Could run every 1 minute if lower latency is needed.

6. **No concurrency concerns today.** Single Lambda instance handles everything. If volume grows 10x, consider reserved concurrency > 1 with watermark partitioning.

7. **Cost is negligible.** At 256MB memory and ~3s execution, running every 5 minutes:
   - ~8,640 invocations/month
   - ~25,920 GB-seconds/month
   - Well within Lambda free tier (400,000 GB-seconds/month)

## Recommendation

The Lambda configuration should be:

- **Memory**: 256MB
- **Timeout**: 60 seconds
- **Schedule**: EventBridge every 5 minutes
- **Batch size**: 1,000 events per DB query (loop until no more events)
- **Concurrency**: Reserved concurrency = 1 (prevent duplicate processing)
