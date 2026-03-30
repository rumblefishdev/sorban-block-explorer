---
id: '0090'
title: 'Security audit: OWASP Top 10, IAM least-privilege, infrastructure hardening'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-medium, layer-testing]
milestone: 3
links:
  - docs/architecture/technical-design-general-overview.md
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — D3 scope coverage (task 0085)'
---

# Security audit: OWASP Top 10, IAM least-privilege, infrastructure hardening

## Summary

Perform security audit covering OWASP Top 10 for the API, IAM least-privilege review, and infrastructure hardening verification. Produce signed-off security checklist required by D3 acceptance criteria.

## Status: Backlog

**Current state:** Not started.

## Context

D3 (§7.4) requires "Security audit checklist (OWASP Top 10, IAM least-privilege, no public RDS endpoint)." The effort breakdown (§7.1F) allocates 3 days. D3 acceptance criteria include: "no wildcard IAM, WAF/throttling active, RDS has no public endpoint, production RDS backups/PITR/deletion protection enabled, RDS and S3 encrypted with KMS-backed keys, all secrets in Secrets Manager, all API inputs validated."

## Implementation Plan

### Step 1: OWASP Top 10 review

Audit all API endpoints against OWASP Top 10: injection, broken auth, sensitive data exposure, XXE, broken access control, security misconfiguration, XSS, insecure deserialization, insufficient logging, SSRF.

### Step 2: IAM least-privilege review

Verify all IAM roles follow least-privilege: no wildcard policies, Lambda roles scoped to required resources, ECS task roles minimal.

### Step 3: Infrastructure hardening verification

Verify: RDS no public endpoint, RDS backups/PITR/deletion protection enabled, RDS and S3 encrypted with KMS, WAF active on API Gateway, all secrets in Secrets Manager, no hardcoded credentials.

### Step 4: Produce security checklist

Document findings, remediations, and sign-off.

## Acceptance Criteria

- [ ] OWASP Top 10 review completed for all API endpoints
- [ ] No wildcard IAM policies in production
- [ ] WAF/throttling active on public ingress
- [ ] RDS has no public endpoint
- [ ] Production RDS: backups, PITR, deletion protection enabled
- [ ] RDS and S3 encrypted with KMS-backed keys
- [ ] All secrets in Secrets Manager
- [ ] All API inputs validated
- [ ] Security checklist signed off
