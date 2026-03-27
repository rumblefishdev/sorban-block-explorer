# Stellar Expert: Network Statistics

**Source:** https://stellar.expert/explorer/public
**Fetched:** 2026-03-27

---

> Note: stellar.expert is a JavaScript-rendered SPA and returned no parseable content via static fetch. Network statistics below are sourced from official Stellar Foundation reports (Q3 2025, Year-in-Review 2025) and the Horizon API, which provide the same underlying on-chain data.

---

## Live Horizon API Snapshot (2026-03-27)

Fetched from https://horizon.stellar.org/ and https://horizon.stellar.org/fee_stats

- **Latest closed ledger:** 61,841,265
- **Ledger timestamp:** 2026-03-27T08:08:04Z
- **Ledger range in history:** 55,533,601 – 61,841,265
- **Protocol version:** 25
- **Horizon version:** 25.1.0
- **Stellar Core version:** 25.2.2
- **Network passphrase:** Public Global Stellar Network ; September 2015
- **Current ledger capacity usage:** 52% (moderate load)
- **Base fee:** 100 stroops

### Fee Distribution (fee_charged, last ledger)

| Percentile | Fee (stroops) |
| ---------- | ------------- |
| p10–p90    | 100 (minimum) |
| p95        | 9,337         |
| p99        | 13,864        |
| max        | 24,120        |

Most operations pay the minimum fee (100 stroops = 0.00001 XLM). The p95/p99 values indicate occasional surge pricing during congestion windows.

---

## Network Throughput Statistics (Official Stellar Foundation Data)

### 2025 Annual Totals

- **Transactions processed in 2025:** 3.6 billion
- **Lifetime total transactions:** 21.5 billion
- **Network uptime:** 99.99%
- **Average fee per operation:** ~$0.0007 (≈$0.00055 per operation per Q3 report)
- **Theoretical max throughput:** 3,000 TPS (post-Whisk upgrade, September 2025)
- **Target throughput (2025 roadmap):** 5,000 TPS

### Q3 2025 (Jul–Sep 2025)

- **Operations processed:** >1 billion (+70% QoQ)
- **Average ledger close time:** 5.76 seconds
- **Smart-contract (Soroban) invocations:** ~1 million per day (+700% QoQ)
- **Network uptime:** 99.99%
- **Global actual network usage rank:** #4

### H1 2025 (Jan–Jun 2025)

- **Daily transaction count:** 1.5 – 2.5 million (baseline range)
- **Notable spikes:** mid-March and early July
- **Daily active addresses:** 60,000 – 90,000

### Derived Estimates (from annual + quarterly figures)

| Metric                      | Estimate         | Basis                      |
| --------------------------- | ---------------- | -------------------------- |
| Operations/day (annual avg) | ~9.9 million/day | 3.6B / 365                 |
| Operations/day (Q3 peak)    | ~11+ million/day | >1B / ~90 days             |
| Soroban invocations/day     | ~1 million       | Q3 2025 direct figure      |
| Ledgers/day                 | ~15,000          | 86,400s / 5.76s close time |

---

## Network Growth Metrics (End of 2025)

- **Monthly active addresses:** 632,000 (+24% YoY)
- **Unique addresses:** 10.3 million
- **Total value locked (TVL):** $173 million (+127% in 2025)
- **Onchain real-world assets:** $785 million (+158%); exceeded $1B in January 2026
- **Payment volume:** $55.6 billion (+52% YoY)

---

## Infrastructure Upgrades Affecting Throughput

### Whisk Protocol Upgrade (September 2025)

- Enabled **parallel transaction execution**
- Added **Soroban state caching**
- Doubled Soroban ledger limits (SLP4 configuration change)
- Reduced non-refundable resource costs by up to 4x
- Pushed theoretical max throughput to **3,000 TPS**

### Protocol 23 (H2 2025)

- Multi-threaded smart contract execution
- WebAssembly module caching (ahead-of-time compilation)
- Partial ledger archival
- Optimized resource metering
- Signature verification moved to background processing

### Road to 5,000 TPS (2025 Roadmap)

Five scaling initiatives: increased parallelism, consensus/execution decoupling, aggressive in-memory caching, smarter benchmarking, and Protocol 23 multi-threading.

---

## Sources

- https://horizon.stellar.org/ (live API)
- https://horizon.stellar.org/fee_stats (live API)
- https://stellar.org/blog/foundation-news/2025-year-in-review
- https://stellar.org/blog/foundation-news/q3-2025-quarterly-report
- https://research.nansen.ai/articles/stellar-half-year-report-h1-2025
- https://stellar.org/blog/developers/the-road-to-5000-tps-scaling-stellar-in-2025
