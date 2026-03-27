---
url: 'https://github.com/aripalo/aws-cdk-github-oidc'
title: 'aws-cdk-github-oidc: CDK constructs for GitHub Actions OIDC with AWS IAM'
fetched_date: 2026-03-26
task: '0006'
---

# AWS CDK Github OpenID Connect

AWS CDK constructs that define:

- GitHub Actions as OpenID Connect Identity Provider into AWS IAM
- IAM Roles that can be assumed by GitHub Actions workflows

These constructs enable you to strengthen deployment security by eliminating the requirement for long-term access keys in GitHub Actions. Instead, you can leverage OpenID Connect for authentication between GitHub Action workflows and AWS IAM.

## Getting Started

```bash
npm i -D aws-cdk-github-oidc
```

### OpenID Connect Identity Provider Trust for AWS IAM

To establish a new GitHub OIDC provider configuration in AWS IAM:

```typescript
import { GithubActionsIdentityProvider } from 'aws-cdk-github-oidc';

const provider = new GithubActionsIdentityProvider(scope, 'GithubProvider');
```

This construct creates an OIDC provider trust configuration with:

- Issuer URL: `https://token.actions.githubusercontent.com`
- Audiences: `['sts.amazonaws.com']`

### Referencing an Existing GitHub OIDC Provider

Since only one GitHub OIDC provider can exist per AWS Account, use `fromAccount` to reference an existing provider:

```typescript
import { GithubActionsIdentityProvider } from 'aws-cdk-github-oidc';

const provider = GithubActionsIdentityProvider.fromAccount(
  scope,
  'GithubProvider'
);
```

### Defining a Role for GitHub Actions to Assume

```typescript
import { GithubActionsRole } from 'aws-cdk-github-oidc';

const uploadRole = new GithubActionsRole(scope, 'UploadRole', {
  provider: provider, // reference into the OIDC provider
  owner: 'octo-org', // repository owner (organization or user) name
  repo: 'octo-repo', // repository name (without the owner name)
  filter: 'ref:refs/tags/v*', // JWT sub suffix filter, defaults to '*'
});

// use it like any other role, for example grant S3 bucket write access:
myBucket.grantWrite(uploadRole);
```

You can pass any `iam.RoleProps` except `assumedBy` (defined by the construct):

```typescript
const deployRole = new GithubActionsRole(scope, 'DeployRole', {
  provider: provider,
  owner: 'octo-org',
  repo: 'octo-repo',
  roleName: 'MyDeployRole',
  description: 'This role deploys stuff to AWS',
  maxSessionDuration: cdk.Duration.hours(2),
});

deployRole.addManagedPolicy(
  iam.ManagedPolicy.fromAwsManagedPolicyName('AdministratorAccess')
);
```

### Subject Filter

By default, `filter` is `'*'`, allowing any workflow from the given repository. To further restrict access:

| `filter` value                 | Description                              |
| ------------------------------ | ---------------------------------------- |
| `'ref:refs/tags/v*'`           | Allow only tags with prefix `v`          |
| `'ref:refs/heads/demo-branch'` | Allow only from branch `demo-branch`     |
| `'pull_request'`               | Allow only from pull request             |
| `'environment:Production'`     | Allow only from `Production` environment |

### GitHub Actions Workflow Example

Use [aws-actions/configure-aws-credentials](https://github.com/aws-actions/configure-aws-credentials) to assume a role:

```yaml
jobs:
  deploy:
    name: Upload to Amazon S3
    runs-on: ubuntu-latest
    permissions:
      id-token: write # needed to interact with GitHub's OIDC Token endpoint
      contents: read
    steps:
      - name: Checkout
        uses: actions/checkout@v2

      - name: Configure AWS credentials
        uses: aws-actions/configure-aws-credentials@v4
        with:
          role-to-assume: arn:aws:iam::123456789012:role/MyUploadRole
          aws-region: us-east-1

      - name: Sync files to S3
        run: |
          aws s3 sync . s3://my-example-bucket
```
