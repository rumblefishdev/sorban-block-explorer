---
id: '0035'
title: 'CDK: CloudFront, WAF, Route 53, S3 static hosting'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0006', '0092', '0097', '0038']
tags: [priority-medium, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: NestJS /api-docs → utoipa-swagger-ui'
  - date: 2026-04-03
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Phase 1: CloudFront + WAF + S3 without custom domain.
      Route 53 + ACM deferred until hosted zone is set up.
  - date: 2026-04-07
    status: completed
    who: stkrolikiewicz
    note: >
      Implemented full delivery layer with custom domain (hosted zone +
      cert provisioned out of band). 3 commits, 9 files, ~770 lines.
      WafWebAcl reusable construct (eliminates dual-stack duplication),
      KVS-backed basic auth, security headers (HSTS env-aware), config
      validateConfig() fail-fast on CHANGE_ME placeholders. PR #69.
      Production envs still have CHANGE_ME hostedZoneId/certificateArn —
      deferred to task 0038 (env config). Snapshot tests deferred (no
      test framework in repo yet).
---

# CDK: CloudFront, WAF, Route 53, S3 static hosting

## Summary

Define the public delivery layer using CDK: CloudFront distribution for the React SPA, WAF WebACL with managed rules and abuse controls, Route 53 hosted zone with DNS aliases, and ACM TLS certificates. API Gateway traffic does NOT route through CloudFront. CloudFront is for static content only. Swagger UI is served directly from the API (utoipa-swagger-ui `/api-docs` endpoint). Staging uses password protection via CloudFront Functions.

## Status: Backlog

**Current state:** Not started. No dependencies on other infrastructure tasks for the delivery layer definition, though WAF is attached to API Gateway in task 0097.

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
- API Gateway (attached in task 0097)

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

- [x] SPA CloudFront distribution is defined with S3 origin and index.html fallback for client routes
- [x] API Gateway traffic does NOT route through CloudFront
- [x] WAF WebACLs are defined with managed rules, IP reputation, and rate-based abuse controls (one per scope — see Deviation)
- [x] WAF is attached to CloudFront distribution (CLOUDFRONT scope) and to API Gateway (REGIONAL scope, defined in `ApiGatewayStack`)
- [x] Route 53 hosted zone has A/AAAA aliases for frontend (DeliveryStack) and API domains (ApiGatewayStack)
- [x] ACM certificate consumed for CloudFront and API Gateway (single us-east-1 cert; see Deviation)
- [ ] DNS validation is automated via Route 53 — N/A, certificate is imported by ARN, not provisioned by CDK
- [x] Staging: CloudFront password protection is implemented via CloudFront Functions backed by KeyValueStore
- [x] Production: no password protection on CloudFront distribution
- [x] HTTP to HTTPS redirect is enabled
- [x] Staging web credentials stored outside repository in CloudFront KeyValueStore (see Deviation — chosen over Secrets Manager/SSM SecureString)
- [x] SPA build pipeline does not embed API keys, secrets, or credentials into the frontend bundle (no SPA build wired in this task)

## Deviations from Original Spec

### 1. Two WAF WebACLs instead of one shared

**Original AC**: "WAF is attached to CloudFront distribution and made available for API Gateway"

**Implemented**: Two distinct WebACLs with identical rule sets — one in `DeliveryStack` (`scope: CLOUDFRONT`) attached to the CloudFront distribution, one in `ApiGatewayStack` (`scope: REGIONAL`) attached to the API Gateway stage.

**Why**: AWS WAF design forbids attaching a `CLOUDFRONT`-scoped WebACL to a regional resource (API Gateway, ALB, AppSync) and vice versa. The "shared WebACL" assumption in the original AC is technically impossible. Two ACLs are the only valid configuration. Both ACLs carry the same managed rule groups (Common, KnownBadInputs, IpReputation) and a 2000-req/5min IP rate limit, plus dedicated CloudWatch log groups with resource policies for `delivery.logs.amazonaws.com`.

### 2. Staging credentials in CloudFront KeyValueStore, not Secrets Manager / SSM SecureString

**Original AC**: "Staging web password stored in AWS Secrets Manager or SSM Parameter Store SecureString"

**Implemented**: CloudFront KeyValueStore (`cloudfront.KeyValueStore` construct), one key `auth-token` containing pre-encoded `base64(user:password)`. The CloudFront Function reads it per-request via `cloudfront.kvs().get('auth-token')`.

**Why**: CloudFront Functions cannot make network calls — they cannot read from Secrets Manager, SSM, or any AWS API in runtime. Both Secrets Manager and SSM SecureString require either:

- (a) `valueFromLookup` at synth time → value gets baked into CFN template / Function code in plain text, defeating "stored in SecureString" intent
- (b) Lambda@Edge instead of CloudFront Function → cold start penalty, replication delay, blocked distribution deletion, ~$0.60/M requests for staging gating — overkill

CloudFront KeyValueStore is the AWS-native pattern for runtime configuration in CloudFront Functions: credentials live in a dedicated IAM-gated store, never enter git, never enter the Function code or CFN template, and rotate in seconds via `aws cloudfront-keyvaluestore put-key` without redeploy. Threat model for staging gating (shared static password protecting non-sensitive UI from indexing/casual access) does not justify Lambda@Edge complexity.

**Bootstrap requirement**: KVS is empty after first deploy. Until populated, the Function returns `503 Service Unavailable` (closed-by-default — safer than open). Populate once after first deploy:

```bash
KVS_ARN=$(aws cloudfront list-key-value-stores \
  --query "KeyValueStoreList.Items[?Name=='staging-soroban-explorer-basic-auth'].ARN" \
  --output text)
ETAG=$(aws cloudfront-keyvaluestore describe-key-value-store \
  --kvs-arn "$KVS_ARN" --query "ETag" --output text)
TOKEN=$(printf 'staging:<password>' | base64)
aws cloudfront-keyvaluestore put-key \
  --kvs-arn "$KVS_ARN" --key auth-token --value "$TOKEN" --if-match "$ETAG"
```

### 3. Single ACM certificate covers both CloudFront and API Gateway

**Original AC**: "ACM certificates are provisioned: us-east-1 for CloudFront, stack region for API Gateway"

**Implemented**: One certificate ARN in `EnvironmentConfig.certificateArn`, consumed by both `DeliveryStack` (CloudFront) and `ApiGatewayStack` (API Gateway custom domain).

**Why**: Stack region is `us-east-1` (per `envs/staging.json`), which happens to be the required region for CloudFront certs. A single wildcard cert in us-east-1 satisfies both. If the project ever moves to another region, `EnvironmentConfig` will need separate `cloudfrontCertificateArn` and `apiCertificateArn` fields, and the API cert must be re-issued in the new stack region.

### 4. Certificate is imported, not provisioned

**Original AC**: "DNS validation is automated via Route 53"

**Implemented**: Certificate already exists in ACM (managed outside CDK), imported by ARN via `acm.Certificate.fromCertificateArn`. No CDK-managed validation.

**Why**: The certificate is a pre-existing wildcard for `*.sorobanscan.rumblefish.dev`, shared across environments and managed manually. Provisioning via CDK would compete for the same DNS validation records and create lifecycle coupling between this stack and certificate renewal. Outside scope of 0035.

### 5. Relationship to research task 0006 / PR #32

PR #32 corrected research 0006 to recommend per-service stack decomposition (`ApiStack`, `IndexerStack`, `IngestionStack`, `FrontendStack`) and elimination of `DeliveryStack`. This task implements the **monolithic `DeliveryStack` + separate `ApiGatewayStack`** pattern instead, consistent with the architecture already established by tasks 0099/0033/0097 (monolithic `ComputeStack`). Per-service refactor is deferred — would require touching 4 archived tasks and ~12 backlog tasks. Tracked as future work, not in scope of 0035.

## Notes

- The SPA CloudFront distribution must handle client-side routing by returning index.html for all paths that do not match a static file. This is achieved through custom error responses (403/404 -> index.html with 200).
- WAF rules should be tuned after initial deployment based on observed traffic patterns. Start with AWS managed rules and adjust.
- CloudFront invalidation will be needed on each SPA deployment. This can be triggered in the CI/CD pipeline (task 0039).
- The staging password protection pattern (CloudFront Functions basic auth) is lightweight and does not require Lambda@Edge if the logic is simple enough.
- All domain names and hosted zone IDs must be parameterized for redeployability across different AWS accounts and domains.
- Swagger UI is served from the API directly (utoipa-swagger-ui `/api-docs` endpoint) — no separate CloudFront distribution or S3 bucket needed for API docs.

## Implementation Notes

**Files created:**

- `infra/src/lib/stacks/delivery-stack.ts` (~245 lines) — S3 + CloudFront + Route 53 + ResponseHeadersPolicy + optional KVS-backed basic auth
- `infra/src/lib/constructs/waf-web-acl.ts` (~180 lines) — reusable WAF construct (managed rules + rate limit + CW Logs with confused-deputy guarded resource policy)
- `infra/src/lib/cloudfront-functions/basic-auth.ts` (~45 lines) — CF Function source generator with explicit KVS id binding

**Files modified:**

- `infra/src/lib/stacks/api-gateway-stack.ts` — REGIONAL WAF added via `WafWebAcl` construct (replaces deferred placeholder from task 0097)
- `infra/src/lib/types.ts` — added `enableWaf`, `enableBasicAuth`, `cloudFrontWafRateLimit`, `apiWafRateLimit` to `EnvironmentConfig`; added `validateConfig()` helper
- `infra/src/lib/app.ts` — wires `DeliveryStack`, calls `validateConfig()` at start
- `infra/envs/staging.json` — populated `hostedZoneId`, `certificateArn`, plus the four new flags
- `infra/envs/production.json` — added the four new flags (hostedZoneId/cert still `CHANGE_ME`, deferred to 0038)
- `infra/Makefile` — added `deploy-{staging,production}-delivery` targets

**Verification:**

- `cdk synth staging` passes — all 9 stacks generated cleanly
- `cdk synth production` fails with clear error from `validateConfig()` (CHANGE_ME placeholders) — intentional, replaces previous silent passthrough

## Design Decisions

### From Plan

1. **CloudFront with OAC + private S3 bucket** — straight from spec.
2. **WAF with AWS managed rule groups + rate-based rule** — straight from spec.
3. **HTTPS-only with redirect from HTTP** — straight from spec.
4. **No password protection on production** — straight from spec.
5. **CloudFront Functions over Lambda@Edge** for staging gating — Notes mentioned both, picked CF Functions for cost/latency/reversibility.

### Emerged

6. **Two WAF WebACLs instead of one shared.** Original AC implied a single shared ACL, but `CLOUDFRONT` and `REGIONAL` scopes cannot share an ACL — AWS WAF design constraint. Two ACLs with identical rule sets is the only valid configuration.

7. **`WafWebAcl` reusable construct.** First iteration duplicated ~150 lines of WAF setup between `DeliveryStack` and `ApiGatewayStack`. Refactored to a shared construct so the two scopes stay in lockstep on rule changes. Eliminates copy-paste drift risk.

8. **CloudFront KeyValueStore over Secrets Manager / SSM SecureString.** AC literally specified Secrets Manager OR SSM SecureString, but neither is reachable from a CloudFront Function in runtime — `valueFromLookup` would bake plaintext into the CFN template, defeating the intent. Lambda@Edge would meet the literal AC but is overkill for staging gating. KVS is the AWS-native pattern: credentials in a dedicated IAM-gated store, never in git/code/template, rotatable in seconds via `aws cloudfront-keyvaluestore put-key`. Documented in task Deviation #2.

9. **Single ACM certificate covers both CloudFront and API Gateway.** AC specified separate certs (us-east-1 + stack region), but stack region happens to be us-east-1, so one wildcard works. If region ever moves, `EnvironmentConfig` will need split fields. Documented in Deviation #3.

10. **Certificate imported, not provisioned by CDK.** AC specified DNS validation via Route 53. The cert pre-exists (managed out of band, shared with other infra), so imported by ARN. CDK-managed validation would compete on the same DNS records and create lifecycle coupling. Documented in Deviation #4.

11. **`enableWaf: false` on staging.** WAF fixed cost is ~$15-20/month/env. Staging is gated by basic auth (5-person team, low traffic), so WAF adds little. Disabled on staging via `enableWaf` flag, enabled on production. ~$200-300/year saved.

12. **HSTS env-aware: short max-age + no preload + no includeSubdomains on staging.** First iteration set `preload: true, includeSubdomains: true, max-age=1y` everywhere. HSTS preload is a one-way door (months to remove via hstspreload.org), and `includeSubdomains` on a staging subdomain affects all sibling subdomains. Tightened to: production gets full hardening, staging gets `max-age=7d` only. Preload is opt-in only via explicit security sign-off.

13. **CSP intentionally omitted.** AWS managed `SECURITY_HEADERS` policy includes `default-src 'self'` which breaks typical SPAs loading fonts/assets from CDNs. Custom policy ships HSTS + nosniff + frame-options + referrer-policy without CSP — to be added in a follow-up task once the SPA is deployed and what it loads is observable, then dial in via `Content-Security-Policy-Report-Only` before enforcing.

14. **`validateConfig()` fail-fast at synth.** Production envs have `CHANGE_ME` placeholders pending task 0038. Without validation, `cdk synth production` would silently produce a broken template; with it, the error message names exactly which fields are wrong. Returns all errors at once, not one at a time.

15. **Soft warning for `enableWaf=false && enableBasicAuth=false`.** That combination leaves CloudFront publicly open with no gating — almost always a staging mistake but might be intentional on production with application-layer controls. Logs a warning rather than blocking.

16. **CF Function code in separate TS module (`cloudfront-functions/basic-auth.ts`).** First iteration had it as a string template literal inside the stack. Extracted to a generator function `basicAuthFunctionCode(kvsId: string)` for testability, syntax highlighting, and so the JS source is auditable separately from the TypeScript stack code.

## Issues Encountered

- **`cdk synth` failed initially on missing SSM parameters** for the early SSM-based basic auth attempt. This led to the KVS pivot (Decision 8). KVS has zero synth-time dependencies — it just needs to exist and be populated post-deploy.

- **HSTS preload regression in iteration 2 of senior review.** Added `preload: true` in `cloudfront.ResponseHeadersPolicy` based on managed-policy default, without analyzing the consequences. Caught and fixed in iteration 3 (Decision 12). Future tasks touching CloudFront response headers should explicitly review HSTS parameters per environment.

- **Account ID mismatch during first synth** — local AWS profile pointed to `045028348791`, but `certificateArn` references account `750702271865`. Resolved by user setting `AWS_PROFILE` to staging account in their shell. No code change needed, but worth flagging that the staging hosted zone and cert live on a specific AWS account that the synth profile must match.

- **Rebase conflicts on `app.ts` and `Makefile`** during merge with develop. Develop had landed `PartitionStack` (0022) and `IngestionStack` (0034) in parallel. Both stacks coexist with `DeliveryStack` cleanly — no architectural conflicts, just text-level merge resolution.

## Future Work

- **Production environment values** — `production.json` still has `CHANGE_ME` for `hostedZoneId` and `certificateArn`. Covered by existing task **0038** (env config). No new task needed.
- **Snapshot tests for CDK stacks** — repo has no test framework yet. Setting one up is its own scope (vitest/jest decision, devDeps, nx target, CI integration). Will spawn dedicated task.
- **CSP tuning** — to be added once SPA is deployed and load patterns are observable. Likely as part of the SPA deployment task or task 0036 (alarms).
- **WAF managed rules in COUNT mode first** — production should ramp via COUNT before BLOCK to avoid false positives. Should be addressed in production launch readiness (covered by task 0038 / 0039 / 0090).
- **CloudFront `BucketDeployment` for placeholder index.html** — currently the bucket is empty after deploy. Frontend deployment is in scope of task 0039 (CI/CD), not here.
