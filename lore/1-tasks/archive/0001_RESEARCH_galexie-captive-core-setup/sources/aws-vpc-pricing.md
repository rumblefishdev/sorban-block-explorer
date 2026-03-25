---
url: 'https://aws.amazon.com/vpc/pricing/'
title: 'Amazon VPC Pricing'
fetched_date: 2026-03-25
task_id: '0001'
image_count: 0
---

# Amazon VPC Pricing

## NAT Gateway Pricing

**Hourly Charges:**

- Standard NAT Gateway: $0.045 per hour
- Regional NAT Gateway: $0.045 per hour per Availability Zone

**Data Processing:**

- $0.045 per GB processed through the NAT gateway

**Additional Charges:**
Standard AWS data transfer charges apply for all data transferred via the NAT gateway, based on destination (AWS services vs. internet).

### NAT Gateway Monthly Cost Calculation

At $0.045/hour for a single NAT Gateway:

- $0.045 × 24 hours × 30 days = **$32.40/month** (hourly charge only)
- Plus $0.045/GB of data processed
- Plus standard data transfer charges for internet-bound traffic

### NAT Gateway Examples

**Standard NAT Gateway Example:**
A single NAT gateway processing 1 GB of data to Amazon S3 in the same region incurs $0.045 (hourly) + $0.045 (data processing). No data transfer charge applies for same-region S3 transfers.

**Regional NAT Gateway Example:**
A Regional NAT Gateway spanning three Availability Zones processing 1 GB to the internet incurs: $0.135 hourly ($0.045 × 3 AZs) + $0.045 data processing + $0.09 data transfer = $0.27 total.

## IPAM (IP Address Manager)

**Free Tier:** No charges for single region/account management including BYOIP and IPv6 capabilities.

**Advanced Tier:** $0.00027 for each active IP address managed in IPAM hourly. Active IPs are those associated with attached Elastic Network Interfaces.

## Network Analysis

**Traffic Mirroring:** $0.015 per session-hour per ENI

**Reachability Analyzer:** $0.10 per analysis performed
