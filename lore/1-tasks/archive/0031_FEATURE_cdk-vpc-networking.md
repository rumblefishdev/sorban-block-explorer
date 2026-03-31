---
id: '0031'
title: 'CDK: VPC, subnets, security groups, VPC endpoints'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0006']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: active
    who: fmazur
    note: 'Activated task for implementation'
  - date: 2026-03-30
    status: completed
    who: fmazur
    note: >
      Implemented NetworkStack with VPC, 2 subnets, 3 security groups,
      S3 Gateway VPC endpoint. 8 new files, CDK synth produces 25 AWS
      resources. Full EnvironmentConfig foundation and Nx CDK targets.
---

# CDK: VPC, subnets, security groups, VPC endpoints

## Summary

Define the foundational AWS networking infrastructure using CDK: a VPC with public/private subnet split, security groups for inter-component access, and a VPC endpoint for S3. The network is designed for single-AZ launch in us-east-1a with a clear path to Multi-AZ expansion. All compute (Lambda, ECS Fargate) and storage (RDS) components depend on this networking layer.

## Status: Completed

## Acceptance Criteria

- [x] VPC is created with appropriate CIDR block in us-east-1
- [x] Public subnet exists in us-east-1a with Internet Gateway route
- [x] Private subnet exists in us-east-1a with NAT Gateway route
- [x] Lambda Security Group allows outbound to RDS Proxy and S3 VPC endpoint
- [x] RDS Security Group allows inbound from Lambda SG on port 5432
- [x] ECS Security Group allows outbound to S3 VPC endpoint and NAT Gateway
- [x] S3 Gateway VPC endpoint is configured on the private subnet route table
- [x] Lambda functions can be VPC-attached in the private subnet (ENI configuration)
- [x] No security group allows unrestricted inbound from 0.0.0.0/0
- [x] CDK code is structured for Multi-AZ expansion without architectural changes
- [x] All networking resources are tagged consistently for cost tracking
- [x] Initial deployment in us-east-1a; VPC structured for Multi-AZ expansion without VPC replacement
- [x] No public internet path exists to reach RDS or ECS Fargate ingestion components directly
- [x] Multi-AZ expansion trigger documented in CDK code: expand when SLA requirement exceeds 99.9%

## Implementation Notes

### Files created

| File                              | Purpose                                       |
| --------------------------------- | --------------------------------------------- |
| `src/lib/stacks/network-stack.ts` | NetworkStack — VPC, subnets, SGs, S3 endpoint |
| `src/lib/config/types.ts`         | EnvironmentConfig interface                   |
| `src/lib/config/staging.ts`       | Staging environment values                    |
| `src/lib/config/production.ts`    | Production environment values                 |
| `src/lib/config/index.ts`         | Config resolver (CDK_ENV or --context env=)   |
| `src/bin/app.ts`                  | CDK app entry point                           |
| `cdk.json`                        | CDK configuration with full feature flags     |
| `project.json`                    | Nx targets: synth, diff, deploy, bootstrap    |

### Files modified

| File                  | Change                                                    |
| --------------------- | --------------------------------------------------------- |
| `package.json`        | Added aws-cdk-lib, constructs, aws-cdk (dev)              |
| `src/index.ts`        | Re-exports NetworkStack, EnvironmentConfig, resolveConfig |
| `eslint.config.mjs`   | Ignore cdk.out/ directory                                 |
| `.gitignore` (root)   | Added cdk.out, cdk.context.json                           |
| `package.json` (root) | Added infra:bootstrap/diff/synth/deploy scripts           |
| `README.md` (root)    | Added Infrastructure section                              |
| `README.md` (infra)   | Full CDK documentation                                    |

### CDK synth output (25 AWS resources)

VPC, 2 subnets, 2 route tables, IGW, NAT GW, EIP, 3 security groups,
SG ingress/egress rules, S3 VPC endpoint, CDK metadata, custom resource
for restricting default SG.

## Design Decisions

### From Plan

1. **Single-AZ in us-east-1a with Multi-AZ readiness**: /16 CIDR, /20 subnets
   give room for expansion. CDK Vpc construct handles new AZs automatically.

2. **Least-privilege security groups**: `allowAllOutbound: false` on all SGs.
   Explicit egress rules only. SG-to-SG references instead of CIDR blocks.

3. **S3 Gateway VPC endpoint (free)**: Route-table level, no hourly cost.
   Reduces NAT Gateway data transfer charges.

### Emerged

4. **NAT Gateway included in this task**: Task notes say NAT belongs to task
   0040, but acceptance criteria requires "private subnet with NAT Gateway
   route". CDK Vpc construct manages NAT atomically — separating it is an
   anti-pattern. Task 0040 can focus on ECR only.

5. **EnvironmentConfig scoped to NetworkStack fields only**: Research task
   0006 designed a 25-field interface. Implemented with 4 fields (YAGNI) —
   each subsequent stack task extends the interface with its own fields.
   Avoids placeholder values and false confidence.

6. **`availabilityZones: readonly string[]` instead of `string`**: Makes
   Multi-AZ expansion a config-only change (add array entry). No type
   changes needed.

7. **`natGateways: config.availabilityZones.length`**: Derives NAT count
   from AZ count automatically. Multi-AZ = one NAT per AZ for HA.

8. **`restrictDefaultSecurityGroup: true`**: CIS Benchmark 5.4. Strips
   default VPC security group of all rules via Custom Resource.

9. **`cdk.context.json` gitignored**: Research decision #15 says commit it
   for deterministic synth. But it contains AWS account ID in cache key,
   and ADR-0001 says no hardcoded account IDs (open-source repo).
   Each deployer/CI generates their own.

10. **Full feature flags from `cdk init`**: Initial implementation had 8
    hand-picked flags. Replaced with complete set (~80) from current CDK
    version for security and compatibility.

11. **ECS egress port 11625**: Stellar overlay protocol for peer connections.
    Not in original task spec but required for Galexie to connect to
    Stellar network peers.

## Issues Encountered

- **CDK `maxAzs` + `availabilityZones` conflict**: Cannot use both params
  in Vpc construct. CDK throws `VpcSupportsAvailabilityZonesMax`. Fix:
  use only `availabilityZones`.

- **ESLint linting `cdk.out/`**: CDK synth output contains minified JS that
  fails lint rules. Fix: added `cdk.out/**` to eslint ignores.

- **Account ID leak in `cdk.context.json`**: Auto-generated context cache
  contains AWS account ID as part of the lookup key. Fix: gitignored the file.

## Future Work

- Task 0040 handles ECR and related compute resources (NAT Gateway already
  provisioned here)
- VPC Flow Logs can be added for debugging (not required at launch)
