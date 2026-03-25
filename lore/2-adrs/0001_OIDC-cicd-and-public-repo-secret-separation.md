---
id: '0001'
title: 'OIDC-based CI/CD authentication and config/secret separation for public repository'
status: accepted
deciders: [stkrolikiewicz]
related_tasks: ['0075', '0076', '0078', '0072']
related_adrs: []
tags: [infrastructure, security, ci-cd]
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: accepted
    who: stkrolikiewicz
    note: 'ADR created post-factum from infrastructure-overview.md sections 9.3-9.4'
---

# ADR 0001: OIDC-based CI/CD authentication and config/secret separation for public repository

**Related:**

- [Task 0075: CDK environment-specific configuration](../1-tasks/backlog/0075_FEATURE_cdk-environment-config.md)
- [Task 0076: CI/CD pipeline](../1-tasks/backlog/0076_FEATURE_cicd-github-actions.md)
- [Task 0078: CDK IAM roles](../1-tasks/backlog/0078_FEATURE_cdk-iam-ecr-nat.md)
- [Task 0072: CDK CloudFront/WAF](../1-tasks/backlog/0072_FEATURE_cdk-cloudfront-waf-route53.md)

---

## Context

The soroban-block-explorer repository is public on GitHub. Infrastructure is deployed to AWS using CDK via GitHub Actions. A public repository creates a unique threat surface: any committed secret (database password, AWS access key, staging password) is immediately and permanently exposed. Even if rotated quickly, leaked credentials in git history can be exploited before rotation completes.

The infrastructure requires credentials for:

- CI/CD deployment to AWS (staging and production)
- Database access (RDS PostgreSQL)
- Staging web frontend password protection
- Potential future integration keys (non-browser API consumers)

---

## Decision

1. **CI/CD authentication uses GitHub Actions OIDC federation** to assume short-lived AWS IAM roles at deploy time. No long-lived AWS access keys are stored in GitHub secrets.

2. **Staging and production deployments use separate AWS IAM roles** with environment-scoped permissions and GitHub Environment protection rules.

3. **Non-secret configuration is committed to the repository** under `infra/aws-cdk/config/*` — this includes environment names, regions, instance classes, retention periods, scaling thresholds, domain names, and secret reference names (ARNs).

4. **Secret values live exclusively in AWS Secrets Manager or SSM Parameter Store SecureString.** CDK code consumes only secret references (ARNs, parameter names), never actual secret values.

5. **The repository never contains** `.env.prod`, `.env.staging`, or similar files with real secrets. No secret values appear in CDK context files, TypeScript constants, or GitHub workflow YAML.

6. **Runtime workloads (Lambda, ECS) access only the specific secrets they need** through IAM policies scoped to individual secret ARNs.

---

## Rationale

OIDC eliminates the highest-risk credential class (long-lived AWS keys) entirely. Short-lived tokens issued per workflow run cannot be reused if the repository is forked or workflow logs are leaked. Combined with per-environment role separation, a compromised staging workflow cannot affect production.

Committing non-secret config (instance sizes, regions, domains) keeps infrastructure reproducible and reviewable via PR. Keeping secrets external makes the repository safe to clone, fork, and inspect without granting any operational access.

This model also supports the project's explicit goal of open-source redeployability: a third party can fork the repo, point config at their own AWS account and Secrets Manager entries, and deploy the full stack.

---

## Alternatives Considered

### Alternative 1: Private repository

**Description:** Make the repository private, allowing secrets to be committed with reduced exposure risk.

**Pros:**

- Simpler credential management
- No config/secret split needed

**Cons:**

- Violates open-source redeployability goal
- Private repos can still leak (employee access, CI logs, forks)
- Does not eliminate the risk, only reduces the blast radius

**Decision:** REJECTED — conflicts with project's open-source mandate

### Alternative 2: Long-lived IAM access keys in GitHub Secrets

**Description:** Store AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY as GitHub repository secrets.

**Pros:**

- Simpler GitHub Actions setup
- No OIDC provider configuration needed

**Cons:**

- Keys are permanent until rotated; leaked keys grant persistent access
- Single set of keys cannot enforce environment separation
- GitHub secrets are accessible to all repository collaborators and workflow runs

**Decision:** REJECTED — unacceptable risk for a public repository

### Alternative 3: Sealed Secrets or SOPS-encrypted files in repository

**Description:** Commit encrypted secret files using Sealed Secrets, SOPS, or similar tools.

**Pros:**

- Secrets version-controlled alongside infrastructure
- Decryption keys held externally

**Cons:**

- Adds operational complexity (key management, rotation)
- Encrypted blobs in git history are permanent; key compromise exposes all historical secrets
- Does not address CI/CD authentication to AWS

**Decision:** REJECTED — adds complexity without solving the CI/CD auth problem; Secrets Manager is simpler for this use case

---

## Consequences

### Positive

- Public repository is safe to clone, fork, and inspect without operational risk
- OIDC tokens are short-lived and scoped to specific workflow runs
- Environment separation enforced at IAM level, not just GitHub UI
- Third parties can redeploy by providing their own secrets
- Config changes are PR-reviewable; secret rotation is independent of code deployment

### Negative

- OIDC provider setup adds initial CDK complexity (one-time)
- Developers must use Secrets Manager console or CLI for secret values (no local .env file)
- Config/secret boundary must be maintained by discipline; accidental secret commits remain possible without CI validation

---

## References

- Infrastructure overview: `docs/architecture/infrastructure/infrastructure-overview.md` sections 9.3-9.4
- GitHub OIDC documentation: https://docs.github.com/en/actions/security-for-github-actions/security-hardening-your-deployments/configuring-openid-connect-in-amazon-web-services
