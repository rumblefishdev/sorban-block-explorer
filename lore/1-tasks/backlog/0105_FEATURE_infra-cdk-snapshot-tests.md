---
id: '0105'
title: 'Infra: CDK snapshot + assertion tests'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0035']
tags: [priority-medium, effort-small, layer-infra, phase-future]
links: []
history:
  - date: 2026-04-07
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0035 future work — no test framework exists in repo for infra/.'
---

# Infra: CDK snapshot + assertion tests

## Summary

Set up a test framework in `infra/` and add snapshot + fine-grained assertion tests for the CDK stacks. There is currently no test framework in the repo for the infrastructure code, so changes to stacks land without regression coverage.

## Context

Spawned from task 0035 (CDK delivery stack). The senior-review pass on 0035 flagged "no tests" as a real gap, but introducing a first test framework is its own scope: framework choice (vitest vs jest), devDependency installation, nx target wiring, CI integration. Out of scope for 0035, tracked here.

The `infra/` package is currently the only Nx project without a `test` target. Snapshot tests for CDK are the standard pattern: serialize the synthesized CloudFormation template and assert it has not changed unintentionally between commits.

## Implementation Plan

1. **Choose framework** — vitest (lighter, ESM-native, faster for TS) or jest (larger ecosystem). Vitest is the modern default.
2. **Add devDeps to `infra/package.json`** — vitest, `@vitest/snapshot` (built-in), maybe `aws-cdk-lib/assertions` for fine-grained.
3. **Add nx `test` target** to `infra` project, wired via `nx.json` or local `project.json`.
4. **Snapshot test for each stack** — instantiate the stack with a stub `EnvironmentConfig`, synth, snapshot the resulting template via `Template.fromStack(stack).toJSON()`.
5. **Fine-grained assertions** for safety-critical properties:
   - `DeliveryStack`: WAF scope is CLOUDFRONT, basic auth gated by `enableBasicAuth`, security headers policy includes HSTS
   - `ApiGatewayStack`: WAF scope is REGIONAL, WAF gated by `enableWaf`
   - `WafWebAcl` construct: rule count, rate limit applied, log group resource policy present
6. **CI integration** — add `nx test infra` to existing GitHub Actions workflow (task 0039 already activates CI).
7. **Smoke test for `validateConfig()`** — assert that `CHANGE_ME` placeholders throw, valid configs pass.

## Acceptance Criteria

- [ ] `infra/` has a working `nx test` target
- [ ] Snapshot test for `DeliveryStack`, `ApiGatewayStack`, and at least one other stack (e.g. `NetworkStack`)
- [ ] Fine-grained assertions for `enableWaf` and `enableBasicAuth` flag behavior
- [ ] `validateConfig()` has unit tests for both happy path and `CHANGE_ME` failure path
- [ ] Tests run in CI on every PR touching `infra/`
- [ ] Snapshots committed to git

## Notes

- Snapshot tests catch unintended changes to synthesized CloudFormation. They are noisy in the first iteration (every change to a stack updates the snapshot) but invaluable for regression detection on infra refactors like the WAF construct extraction in 0035.
- Avoid testing CDK internals — test what we own (rule counts, scope, conditional resources from feature flags).
