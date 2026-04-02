---
id: '0097'
title: 'CDK: API Gateway + WAF + usage plans'
type: FEATURE
status: active
related_adr: ['0005']
related_tasks: ['0033', '0035']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-04-01
    status: backlog
    who: fmazur
    note: 'Split from task 0033 — API Gateway, WAF, and usage plans scope'
  - date: 2026-04-02
    status: active
    who: fmazur
    note: 'activate task'
---

# CDK: API Gateway + WAF + usage plans

## Summary

Define the REST API Gateway with Lambda proxy integration, response caching, throttling, WAF attachment, and optional API key usage plans. This task consumes the API Lambda ARN exported by task 0033 (ComputeStack) and exposes the public API endpoint.

## Status: Active

**Current state:** Not started. Depends on task 0033 (API Lambda definition) for the Lambda integration target.

## Context

The block explorer API needs a public-facing REST API Gateway to deliver traffic to the Rust/axum API Lambda. REST API mode (not HTTP API) is chosen for response caching, request validation, and WAF integration — HTTP API is cheaper but lacks these features.

The WAF WebACL (defined in task 0035) protects the API from abuse without requiring API keys for browser traffic. Optional API key usage plans serve non-browser consumers (automation, partner integrations).

### Source Code Location

- `infra/aws-cdk/lib/stacks/` (new `api-gateway-stack.ts`)

## Implementation Plan

### Step 1: API Gateway Definition

Define the REST API Gateway:

- Type: REST API (not HTTP API) for full feature support
- Integration: Lambda proxy integration with API Lambda (ARN from ComputeStack)
- Throttling: environment-specific rate and burst limits
- Request validation: enable request body and parameter validation where schemas are defined
- Stage: environment-specific (staging, production)
- HTTPS/TLS enforced for all public traffic

### Step 2: Response Caching

Configure API Gateway cache:

- Long TTL for immutable data (e.g., transaction by hash, ledger by sequence)
- Short TTL (5-15 seconds) for mutable data (e.g., recent transactions, account balances)
- Cache keys include path + query parameters
- Cache size: environment-specific

### Step 3: WAF Attachment

Attach the WAF WebACL (resource defined in task 0035) to the API Gateway stage. This protects the API from abuse without requiring API keys for browser traffic.

**Note:** If task 0035 is not yet implemented, define the WAF attachment point and leave wiring as a TODO.

### Step 4: API Key Usage Plans

Define optional API key usage plans for non-browser consumers:

- Not required for browser traffic (the SPA does not embed API keys)
- Available for trusted automation, partner integrations, or rate-limited programmatic access
- Usage plan with throttle and quota settings

### Step 5: EnvironmentConfig Extension

Add API Gateway fields to `EnvironmentConfig` in `types.ts`:

- `apiGatewayThrottleRate`, `apiGatewayThrottleBurst`
- `apiGatewayCacheSize`, `apiGatewayCacheTtlImmutable`, `apiGatewayCacheTtlMutable`

Update `envs/staging.json` and `envs/production.json`.

### Step 6: Stack Wiring

- Create `ApiGatewayStack` in `stacks/api-gateway-stack.ts`
- Wire in `app.ts`: pass API Lambda function from ComputeStack
- Export API endpoint URL as stack output

## Acceptance Criteria

- [ ] API Gateway REST API defined with Lambda proxy integration
- [ ] Throttling configured (environment-specific rate and burst limits)
- [ ] Response caching configured with long TTL for immutable and short TTL for mutable data
- [ ] Cache keys include path + query parameters
- [ ] WAF WebACL attachment point defined (wired when task 0035 is complete)
- [ ] API key usage plans defined for non-browser consumers (optional)
- [ ] API Gateway enforces HTTPS/TLS for all public traffic
- [ ] Browser traffic is anonymous read-only; API keys not required for default usage
- [ ] EnvironmentConfig extended with API Gateway fields, both env JSONs updated
- [ ] ApiGatewayStack wired in app.ts
- [ ] API endpoint URL exported as stack output

## Notes

- API Gateway REST API mode (vs HTTP API) is chosen for response caching, request validation, and WAF integration. HTTP API is cheaper but lacks these features.
- WAF (task 0035) may not be ready when this task starts. The attachment code should be conditional or clearly marked for wiring later.
- This task depends on task 0033 for the API Lambda ARN. The two stacks will have a cross-stack reference.
