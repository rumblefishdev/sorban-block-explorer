# Stellar Dashboard: Network Metrics

**Source:** https://dashboard.stellar.org/
**Fetched:** 2026-03-27

---

> Note: dashboard.stellar.org is a JavaScript-rendered SPA (React/Vue). Static fetch returned only the page shell: title "Stellar Network Dashboard" and description "Live metrics about the Stellar public network, testnet, and lumen supply numbers" — no metric values were accessible. Throughput data below is sourced from Horizon API live endpoints and official Stellar Foundation reports.

---

## Live Ledger Metrics (Horizon API, 2026-03-27T08:08:04Z)

Fetched from https://horizon.stellar.org/fee_stats and https://horizon.stellar.org/

| Metric                           | Value                     |
| -------------------------------- | ------------------------- |
| Latest ledger                    | 61,841,265                |
| Protocol version                 | 25                        |
| Ledger capacity usage            | 52%                       |
| Base fee                         | 100 stroops (0.00001 XLM) |
| Ledger close time (avg, Q3 2025) | 5.76 seconds              |

### Fee Stats — fee_charged distribution (last ledger)

| Percentile | Stroops           |
| ---------- | ----------------- |
| p10 – p90  | 100 (minimum fee) |
| p95        | 9,337             |
| p99        | 13,864            |
| max        | 24,120            |

Interpretation: ~90% of operations pay the minimum base fee. The long tail at p95+ reflects surge-priced transactions during capacity spikes.

---

## Network Throughput (Official Reports)

### Transaction Volume

| Period                 | Metric                | Value             |
| ---------------------- | --------------------- | ----------------- |
| Full year 2025         | Total transactions    | 3.6 billion       |
| Lifetime (to end 2025) | Total transactions    | 21.5 billion      |
| H1 2025 daily range    | Transactions/day      | 1.5 – 2.5 million |
| Q3 2025                | Operations (total)    | >1 billion        |
| Q3 2025                | Operations QoQ growth | +70%              |

### Derived Daily Rates

| Metric                           | Estimate     | Source basis          |
| -------------------------------- | ------------ | --------------------- |
| Operations/day (2025 annual avg) | ~9.9 million | 3.6B / 365 days       |
| Operations/day (Q3 2025 avg)     | ~11+ million | >1B / ~90 days        |
| Soroban invocations/day          | ~1 million   | Q3 2025 direct report |
| Ledgers/day                      | ~15,000      | 86,400s / 5.76s       |
| Ledgers/year                     | ~5.5 million | derived               |

### TPS Capacity

| Metric                                     | Value     |
| ------------------------------------------ | --------- |
| Theoretical max TPS (post-Whisk, Sep 2025) | 3,000 TPS |
| Target TPS (2025 roadmap)                  | 5,000 TPS |
| Current capacity usage (live)              | 52%       |

---

## Soroban (Smart Contract) Metrics

| Metric                                 | Value            | Period                |
| -------------------------------------- | ---------------- | --------------------- |
| Daily smart contract invocations       | ~1 million/day   | Q3 2025               |
| QoQ growth in Soroban invocations      | +700%            | Q3 2025 vs Q2         |
| Ledger limit change                    | 2× increase      | SLP4 config, Sep 2025 |
| Non-refundable resource cost reduction | up to 4× cheaper | Whisk upgrade         |

---

## Network Health

| Metric                                | Value              |
| ------------------------------------- | ------------------ |
| Uptime (2025)                         | 99.99%             |
| Average fee per operation             | ~$0.0007           |
| Monthly active addresses (end 2025)   | 632,000 (+24% YoY) |
| Unique addresses (end 2025)           | 10.3 million       |
| Global actual network usage rank      | #4 (Q3 2025)       |
| Full-time developer growth (YTD 2025) | +37%               |

---

## Payment & DeFi Volume

| Metric                          | Value         | Period                   |
| ------------------------------- | ------------- | ------------------------ |
| Total payment volume            | $55.6 billion | 2025 FY (+52% YoY)       |
| Cross-border RWA payment volume | $5.4 billion  | Q3 2025 (+27% QoQ)       |
| TVL                             | $173 million  | End 2025 (+127%)         |
| Onchain RWAs                    | $785 million  | End 2025 (>$1B Jan 2026) |

---

## Sources

- https://dashboard.stellar.org/ (shell only — JS-rendered)
- https://horizon.stellar.org/ (live API)
- https://horizon.stellar.org/fee_stats (live API)
- https://stellar.org/blog/foundation-news/2025-year-in-review
- https://stellar.org/blog/foundation-news/q3-2025-quarterly-report
- https://research.nansen.ai/articles/stellar-half-year-report-h1-2025
