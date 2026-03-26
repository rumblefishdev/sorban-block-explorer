---
prefix: R
title: 'Provisioned Concurrency Recommendation for Launch'
status: mature
spawned_from: '0004'
---

# R: Provisioned Concurrency Recommendation for Launch

## Recommendation: Do NOT use provisioned concurrency at launch

### Rationale

1. **Cold start frequency is already low** — At moderate traffic (10 req/s), only 0.5% of requests experience cold starts. With esbuild bundling + ARM, cold starts should be 300-500ms, acceptable for a block explorer API.

> Source: [nestjs-lambda-cold-starts-mono-lambda.md](../sources/nestjs-lambda-cold-starts-mono-lambda.md), "Key Metrics" — 4 cold starts per ~1000 requests

2. **Cost overhead is significant** — Provisioned concurrency costs $0.015/GB-hour even with zero invocations. For a 1GB function with 5 provisioned instances running 24/7:
   - Baseline cost: $0.075/hour = **$54/month just for being provisioned**
   - This is wasted spend if traffic is low or unpredictable at launch

> Source: [provisioned-concurrency-cost-best-practices.md](../sources/provisioned-concurrency-cost-best-practices.md), "Provisioned Concurrency Pricing" section

3. **Traffic patterns unknown at launch** — Provisioned concurrency is most effective when usage patterns are predictable. The block explorer is a new product with uncertain traffic.

> Source: [nestjs-lambda-cold-starts-mono-lambda.md](../sources/nestjs-lambda-cold-starts-mono-lambda.md), Conclusions — "provisioned concurrency... once usage patterns become predictable"

### When to Revisit

Add provisioned concurrency when:

- **P99 latency matters** — If block explorer users report slow first-load times
- **Traffic becomes predictable** — Known daily/weekly patterns emerge
- **High utilization threshold** — When `ProvisionedConcurrencyUtilization` would stay above 60-70%, it becomes cost-effective (16% cheaper than on-demand at full utilization)

> Source: [provisioned-concurrency-cost-best-practices.md](../sources/provisioned-concurrency-cost-best-practices.md), cost analysis — "Fully utilized provisioned concurrency ($0.035 + $0.015 = $0.05/GB-hour) is 16% cheaper than on-demand ($0.06/GB-hour)"

### Auto-Scaling Option for Future

When ready to adopt provisioned concurrency, use Application Auto Scaling with target tracking:

```bash
aws application-autoscaling register-scalable-target \
  --service-namespace lambda \
  --resource-id function:my-function:prod \
  --scalable-dimension lambda:function:ProvisionedConcurrency \
  --min-capacity 1 \
  --max-capacity 100
```

> Source: [provisioned-concurrency-cost-best-practices.md](../sources/provisioned-concurrency-cost-best-practices.md), "Auto Scaling" section

### Monitoring Metrics to Track

| Metric                                       | Purpose                              |
| -------------------------------------------- | ------------------------------------ |
| `ProvisionedConcurrencySpilloverInvocations` | Cold starts exceeding capacity       |
| `ProvisionedConcurrencyUtilization`          | Cost efficiency indicator            |
| `InitDuration`                               | Actual cold start time in CloudWatch |

> Source: [provisioned-concurrency-cost-best-practices.md](../sources/provisioned-concurrency-cost-best-practices.md), "Monitoring" section
