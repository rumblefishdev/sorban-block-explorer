---
id: '0035'
title: 'CDK: CloudFront, WAF, Route 53, S3 static hosting'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006']
tags: [priority-medium, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: CloudFront, WAF, Route 53, S3 static hosting

## Summary

Define the public delivery layer using CDK: CloudFront distribution for the React SPA, WAF WebACL with managed rules and abuse controls, Route 53 hosted zone with DNS aliases, and ACM TLS certificates. API Gateway traffic does NOT route through CloudFront. CloudFront is for static content only. Swagger UI is served directly from the API (NestJS `/api-docs` endpoint). Staging uses password protection via CloudFront Functions.

## Status: Backlog

**Current state:** Not started. No dependencies on other infrastructure tasks for the delivery layer definition, though WAF is attached to API Gateway in task 0033.

## Context

The block explorer serves the React SPA frontend through CloudFront with its own Route 53 alias. API Gateway handles API traffic directly and does NOT route through CloudFront — CloudFront is for static content delivery only.

WAF provides browser-facing protection without requiring API keys or secrets in the SPA bundle. The same WebACL is attached to the CloudFront distribution and to API Gateway.

### Source Code Location

- `infra/aws-cdk/lib/delivery/`

## Implementation Plan

### Step 1: CloudFront Distribution - React SPA

Define a CloudFront distribution for the React frontend:

- Origin: S3 bucket hosting the built React SPA (OAI or OAC for private bucket access)
- Default root object: `index.html`
- Error pages: 403 and 404 redirect to `index.html` with 200 status (SPA client-side routing fallback)
- Cache behavior: long TTL for static assets (JS, CSS, images with content hash), short TTL for `index.html`
- HTTPS only: redirect HTTP to HTTPS
- Price class: appropriate for target audience geography
- WAF WebACL: attached (defined in Step 3)

### Step 2: Route 53 Configuration

Define DNS routing:

- Hosted zone for the project domain
- A record (alias) pointing to the SPA CloudFront distribution (e.g., `explorer.example.com`)
- A record (alias) pointing to API Gateway (e.g., `api.example.com`)
- AAAA records (IPv6 aliases) for both

### Step 3: WAF WebACL

Define a WAF WebACL with:

- AWS Managed Rules: Common Rule Set, Known Bad Inputs, IP Reputation List
- Rate-based rule for abuse control (e.g., limit requests per IP per 5-minute window)
- Geo-restriction if needed (optional)
- Logging to CloudWatch Logs for visibility

**WAF attachment points:**

- SPA CloudFront distribution (attached here)
- API Gateway (attached in task 0033)

### Step 4: ACM TLS Certificates

Provision TLS certificates:

- CloudFront certificate: must be in us-east-1 (CloudFront requirement). Covers SPA domain.
- API Gateway certificate: in the stack's deployment region. Covers the API domain.
- Validation: DNS validation via Route 53 (automated by CDK)
- Auto-renewal: managed by ACM

### Step 5: Staging Password Protection

For the staging environment:

- Implement basic auth via CloudFront Functions or Lambda@Edge
- Protect the SPA CloudFront distribution with username/password
- Credentials stored in environment configuration (not hard-coded)
- Production distributions have no password protection

## Acceptance Criteria

- [ ] SPA CloudFront distribution is defined with S3 origin and index.html fallback for client routes
- [ ] API Gateway traffic does NOT route through CloudFront
- [ ] WAF WebACL is defined with managed rules, IP reputation, and rate-based abuse controls
- [ ] WAF is attached to CloudFront distribution and made available for API Gateway (task 0033)
- [ ] Route 53 hosted zone has A/AAAA aliases for frontend and API domains
- [ ] ACM certificates are provisioned: us-east-1 for CloudFront, stack region for API Gateway
- [ ] DNS validation is automated via Route 53
- [ ] Staging: CloudFront password protection is implemented via CloudFront Functions or Lambda@Edge
- [ ] Production: no password protection on CloudFront distribution
- [ ] HTTP to HTTPS redirect is enabled
- [ ] Staging web password stored in AWS Secrets Manager or SSM Parameter Store SecureString; not committed to repository
- [ ] SPA build pipeline does not embed API keys, secrets, or credentials into the frontend bundle

## Notes

- The SPA CloudFront distribution must handle client-side routing by returning index.html for all paths that do not match a static file. This is achieved through custom error responses (403/404 -> index.html with 200).
- WAF rules should be tuned after initial deployment based on observed traffic patterns. Start with AWS managed rules and adjust.
- CloudFront invalidation will be needed on each SPA deployment. This can be triggered in the CI/CD pipeline (task 0039).
- The staging password protection pattern (CloudFront Functions basic auth) is lightweight and does not require Lambda@Edge if the logic is simple enough.
- All domain names and hosted zone IDs must be parameterized for redeployability across different AWS accounts and domains.
- Swagger UI is served from the API directly (NestJS `/api-docs` endpoint) — no separate CloudFront distribution or S3 bucket needed for API docs.
