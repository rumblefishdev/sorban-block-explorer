---
url: 'https://lumigo.io/blog/provisioned-concurrency-the-end-of-cold-starts/'
title: 'AWS Lambda Provisioned Concurrency: The End of Cold Starts'
fetched_date: 2026-03-26
task_id: '0004'
overwritten: false
image_count: 0
---

# AWS Lambda Provisioned Concurrency: The End of Cold Starts

## What Is Provisioned Concurrency?

Provisioned concurrency is a Lambda feature that extends control over serverless application performance. It pre-initializes execution environments so functions can respond immediately with double-digit millisecond latency.

## What Are Cold Starts and How Does Provisioned Concurrency Help?

Cold starts occur during the first request handled by a new Lambda worker and can take up to **five seconds** for heavy frameworks. Lambda initializes a worker and function module before passing requests to handler functions. Functions remain warm for only 30-45 minutes after executing before being spun down.

Once enabled, Provisioned Concurrency maintains the desired number of concurrent executions initialized and ready to respond, effectively **eliminating cold starts**.

**Example scenario:** A food delivery service experiences peak traffic during lunch and dinner hours. By increasing Provisioned Concurrency before these predictable spikes, the service avoids cold starts when users flood in.

When requests exceed Provisioned Concurrency capacity, Lambda overflows to on-demand scaling. Cold starts may occur for these spillover invocations, but they remain infrequent with adequate provisioned concurrency configuration.

## How to Configure Provisioned Concurrency

Steps via AWS Management Console:

1. Select an existing Lambda function
2. From **Actions**, select **Publish new version**
3. Add optional description and select **Publish**
4. From **Actions**, select **Create alias** and enter a name
5. From **Version** dropdown, select **1** and select **Create**
6. In the **Concurrency** card, select **Add**
7. For **Qualifier Type**, select **Alias** and choose the created alias
8. Specify the **Provisioned Concurrency** value (number of continuously running instances)
9. Select **Save**

> Cannot configure Provisioned Concurrency on `$LATEST` alias or any alias pointing to `$LATEST`.

## Monitoring Provisioned Concurrency

Monitor using these CloudWatch metrics:

| Metric                                       | Description                                                 |
| -------------------------------------------- | ----------------------------------------------------------- |
| `ProvisionedConcurrentExecutions`            | Concurrent executions using Provisioned Concurrency         |
| `ProvisionedConcurrencyUtilization`          | Fraction of Provisioned Concurrency in use                  |
| `ProvisionedConcurrencyInvocations`          | Number of invocations using Provisioned Concurrency         |
| `ProvisionedConcurrencySpilloverInvocations` | Invocations exceeding Provisioned Concurrency (cold starts) |

## Important Details

- **Provisioning time**: Lambda provisions requested concurrent executions within 1-2 minutes of enabling
- **Version dependency**: Provisioned Concurrency applies to a specific version/alias, not the function itself
- **Combination limitations**: Cannot configure Provisioned Concurrency on both an alias and its underlying version simultaneously

## Concurrency Limits

Provisioned Concurrency counts against your regional concurrency limit.

If a function has reserved concurrency:

```
sum(Provisioned Concurrency of all versions) <= reserved concurrency
```

## Provisioned Concurrency Pricing

**On-demand pricing:**

- Invocation duration: $0.06 per GB-hour (100ms rounding)
- Requests: $0.20 per 1M requests

**Provisioned Concurrency pricing:**

- Invocation duration: $0.035 per GB-hour (100ms rounding)
- Requests: $0.20 per 1M requests
- Uptime (just for being provisioned): $0.015 per GB-hour (5-minute rounding)

**Key cost facts:**

- 1 Provisioned Concurrency unit on a 1GB function costs **$0.015/hour even with zero invocations**
- 10 units at 1GB = **$0.15/hour baseline**
- Fully utilized provisioned concurrency ($0.035 + $0.015 = $0.05/GB-hour) is **16% cheaper** than on-demand ($0.06/GB-hour)
- High utilization = cost savings; low utilization = wasted spend

## Auto Scaling with Provisioned Concurrency

### Scaling by Utilization

Register alias as a scaling target:

```bash
aws application-autoscaling register-scalable-target \
  --service-namespace lambda \
  --resource-id function:my-function:canary \
  --scalable-dimension lambda:function:ProvisionedConcurrency \
  --min-capacity 1 \
  --max-capacity 100
```

Apply target tracking policy:

```bash
aws application-autoscaling put-scaling-policy \
  --service-namespace lambda \
  --scalable-dimension lambda:function:ProvisionedConcurrency \
  --resource-id function:my-function:canary \
  --policy-name TestPolicy \
  --policy-type TargetTrackingScaling \
  --target-tracking-scaling-policy-configuration file://config.json
```

`config.json`:

```json
{
  "TargetValue": 0.7,
  "PredefinedMetricSpecification": {
    "PredefinedMetricType": "LambdaProvisionedConcurrencyUtilization"
  }
}
```

### Scheduled Scaling

Configure 20 instances for a canary alias at a specific time:

```bash
aws application-autoscaling put-scheduled-action \
  --service-namespace lambda \
  --scheduled-action-name TestScheduledAction \
  --resource-id function:my-function:canary \
  --scalable-dimension lambda:function:ProvisionedConcurrency \
  --scalable-target-action MinCapacity=20,MaxCapacity=20 \
  --schedule "at(2019-11-28T11:05:00)"
```

Use cron expressions (`--schedule "cron(...)"`") to enable/disable Provisioned Concurrency daily at predictable traffic hours.

## Decision Framework

| Scenario                          | Recommendation                   |
| --------------------------------- | -------------------------------- |
| User-facing API with latency SLAs | Use Provisioned Concurrency      |
| Predictable daily traffic peaks   | Use scheduled scaling            |
| Unpredictable traffic patterns    | Use target tracking auto-scaling |
| Background batch processing       | Skip Provisioned Concurrency     |
| Infrequent admin tasks            | Skip Provisioned Concurrency     |
| NestJS with 1s+ cold starts       | Evaluate cost vs. user impact    |

> **Cost trap:** Teams enabling Provisioned Concurrency without running the numbers have discovered monthly Lambda bills jumping from $50 to $500 for functions that didn't need it. Always model the cost difference before enabling.
