---
url: 'https://github.com/corymhall/cdk-diff-action'
title: 'cdk-diff-action: GitHub Action to comment on PRs with CDK stack diff'
fetched_date: 2026-03-26
task: '0006'
---

# CDK Diff Action

GitHub action to comment on PRs with the stack diff.

## Features

- Create a single comment per CDK stage
- Updates the same comment on each commit, reducing clutter
- Calls out any destructive changes to resources
- Fail workflow if there are destructive changes
- Summary of stack changes with expandable details
- Allow destructive changes for certain resource types

## Usage

The action handles performing the diff and commenting on the PR. It requires credentials to AWS and the synthesized CDK cloud assembly (`cdk.out`). Minimal example:

```yaml
name: diff
on:
  pull_request:
    branches:
      - main
jobs:
  Synth:
    name: Synthesize
    permissions:
      contents: read
      pull-requests: write
      id-token: write
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4
      - name: Setup Node
        uses: actions/setup-node@v3
        with:
          node-version: 20
      - name: Install dependencies
        run: yarn install --frozen-lockfile
      - name: Synth
        run: npx cdk synth
      - name: Authenticate Via OIDC Role
        uses: aws-actions/configure-aws-credentials@v4
        with:
          aws-region: us-east-2
          role-duration-seconds: 1800
          role-skip-session-tagging: true
          role-to-assume: arn:aws:iam::1234567891012:role/cdk_github_actions
          role-session-name: github
      - name: Diff
        uses: corymhall/cdk-diff-action@v2
        with:
          githubToken: ${{ secrets.GITHUB_TOKEN }}
```

This action supports semver versioning:

```yaml
uses: corymhall/cdk-diff-action@v1      # latest v1.x.x
uses: corymhall/cdk-diff-action@v1.1    # latest v1.1.x
```

## Configuration Options

### Allow Destroy Types

Allow certain resource types to be destroyed without failing the build:

```yaml
- name: Diff
  uses: corymhall/cdk-diff-action@v2
  with:
    allowedDestroyTypes: |
      AWS::ECS::TaskDefinition
      AWS::CloudWatch::Dashboard
    githubToken: ${{ secrets.GITHUB_TOKEN }}
```

### Disable Diff for Specific Stages

Use `stackSelectorPatterns` with glob patterns to filter which stacks to diff. To exclude stacks use an exclude pattern (e.g. `!SomeStage/SampleStack`):

```yaml
- name: Diff
  uses: corymhall/cdk-diff-action@v2
  with:
    stackSelectorPatterns: |
      !Stage1/*
      !Stage2/*
    githubToken: ${{ secrets.GITHUB_TOKEN }}
```

### Don't Fail for Destructive Changes in Certain Stages

Show the diff for stages, but do not fail the build on destructive changes:

```yaml
- name: Diff
  uses: corymhall/cdk-diff-action@v2
  with:
    noFailOnDestructiveChanges: |
      Stage1
      Stage2
    githubToken: ${{ secrets.GITHUB_TOKEN }}
```

### Disable Workflow Failure Entirely

Never fail the workflow even if there are destructive changes:

```yaml
- name: Diff
  uses: corymhall/cdk-diff-action@v2
  with:
    failOnDestructiveChanges: false
    githubToken: ${{ secrets.GITHUB_TOKEN }}
```
