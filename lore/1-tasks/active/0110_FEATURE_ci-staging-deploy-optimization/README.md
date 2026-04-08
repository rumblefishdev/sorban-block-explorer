---
id: '0110'
title: 'CI: staging deploy optimization — region var, caching, tag-gating'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0039', '0038', '0103']
tags: [ci, cd, cdk, github-actions, staging, priority-medium, effort-medium]
links:
  - .github/workflows/deploy-staging.yml
history:
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Task created — bundles 3 independent improvements to staging deploy workflow'
  - date: '2026-04-08'
    status: active
    who: stkrolikiewicz
    note: 'Promoted from backlog to active.'
  - date: '2026-04-08'
    status: active
    who: stkrolikiewicz
    note: 'Converted to directory format; detailed plan split into notes/.'
---

# CI: staging deploy optimization

**Scope: staging only.** Three independent improvements to
`.github/workflows/deploy-staging.yml`, delivered as **4 separate PRs**
(one prerequisite + three substantive). A parallel task **0103** covers the
same improvements for production; staging acts as pilot.

## Subtasks (= PRs)

| #    | Title                               | Risk          | Notes                                                               |
| ---- | ----------------------------------- | ------------- | ------------------------------------------------------------------- |
| PR 0 | Add `workflow_dispatch` trigger     | minimal       | Prerequisite for Phase 0 measurement + pre-merge testing            |
| PR 1 | Region as `vars.AWS_REGION`         | low           | One file, strict scope                                              |
| PR 2 | Deploy caching (measurement-driven) | medium        | Phase 0 baseline mandatory; stale-binary risk → SHA256 verification |
| PR 3 | Tag-gated deploy trigger            | high (social) | ADR + team sign-off required                                        |

Detailed steps, acceptance criteria, and scope limits per PR →
**[notes/G-subtask-breakdown.md](notes/G-subtask-breakdown.md)**

## Caching strategy (PR 2 deep dive)

Rust / cargo-lambda / Node / Nx caching is non-trivial and has several traps.
Full analysis with ROI ranking, test matrix, and stale-binary mitigations →
**[notes/G-caching-strategy.md](notes/G-caching-strategy.md)**

## Quality gates & process

Pre-merge testing, rollback, stop-loss, scope locks, ADR process,
regression guards, post-merge validation, cost tracking, monitoring →
**[notes/G-quality-gates.md](notes/G-quality-gates.md)**

## Ground rules

- **Measure before optimizing.** Phase 0 baseline mandatory for PR 2.
- **Do NOT cache `cdk.out`** — account-specific, correctness risk.
- **Each subtask = one PR**, independently revertable.
- **Tag-gating is a process decision**, not just a workflow change — requires ADR.
- **Scope is locked per PR** (see notes/G-subtask-breakdown.md).

## Related tasks

- **0039** — parent task, created `deploy-staging.yml` (archived).
- **0038** — CDK environment config module. Hand-off note in [notes/G-quality-gates.md](notes/G-quality-gates.md) under "Hand-off for 0038".
- **0103** — production deploy workflow. Scope extended to mirror these three improvements; 0110 lands first as pilot.

## Out of scope

- Production deploy workflow (→ 0103)
- Preview environments per PR
- Multi-region deploy
- Nx Cloud / remote cache
- Replacing CDK

## Top-level acceptance criteria

- [ ] PR 0 merged — `workflow_dispatch` available
- [ ] PR 1 merged — no literal `us-east-1` in `deploy-staging.yml`; sourced from `vars.AWS_REGION`
- [ ] PR 2 merged — measurable deploy-time reduction justified by Phase 0 baseline; cache validation test matrix passed; SHA256 Lambda verification step added
- [ ] PR 3 merged OR moved to blocked/canceled with documented reason — staging trigger behavior matches ADR
- [ ] Regression guard in CI prevents reintroduction of `us-east-1` literal
- [ ] Worklog entries for each PR (facts + emerged decisions)
- [ ] Post-merge validation period observed (see quality gates)

## Open questions (resolved during work, logged in worklog)

- [ ] Does `infra/bin/staging.ts` hardcode region? (PR 1)
- [ ] Current mean deploy time and frequency? (PR 2 / PR 3 ROI gate)
- [ ] Team consensus on staging purpose? (PR 3 ADR)
- [ ] Are `nx.json` `inputs`/`outputs` correctly declared for CDK build? (PR 2 prerequisite)
- [ ] Which Rust crates actually compile in this workflow? (PR 2)
