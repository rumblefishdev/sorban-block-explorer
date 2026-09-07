---
id: '0532'
title: 'Worktree provisioning symlinks node_modules and skips husky — typechecks the wrong branch, and no pre-commit gate'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0528']
tags: ['tooling', 'dx', 'worktree', 'ci', 'effort-small']
links: []
history:
  - date: '2026-09-01'
    status: backlog
    who: karolkow
    note: >
      Both defects hit during 0528 and cost real time. A fresh worktree gets
      `node_modules` symlinked to the main checkout, so `@rumblefish/*`
      workspace packages resolve into whatever branch the MAIN checkout happens
      to sit on — the web typecheck then validates this branch's code against
      another branch's API types. Separately `.husky/_` is not created, so the
      pre-commit gate silently does not run: a file with unclosed Rust
      delimiters committed cleanly.
---

# Worktree provisioning shares node_modules and skips husky

## Summary

Creating a worktree provisions it in a way that breaks two guarantees silently:
the wrong branch's packages get typechecked, and the pre-commit gate does not
run at all. Both fail **quietly** — no error, no warning, just a wrong answer.

## Defect 1 — `node_modules` is a symlink to the main checkout

A fresh worktree gets `node_modules` symlinked to the main checkout's. The
`@rumblefish/*` entries inside it are relative symlinks (`-> ../../libs/<pkg>`),
so after following the outer symlink they resolve to the **main checkout's**
`libs/`, not the worktree's.

Consequence: the worktree's web code is typechecked against whatever branch the
main checkout is sitting on.

Observed during 0528: the main checkout was on a liquidity-pool branch where
`asset_a?: null | PoolAssetLeg`. A branch whose diff contained **zero TypeScript
files** produced 20+ `PoolAssetLeg | null | undefined` type errors and failed
`pre-push`. A clean `develop` worktree — which happens to have a real
`node_modules` directory — reported 0 errors on the same code.

The failure mode is the dangerous kind: it points at files the change never
touched, so the obvious reading is "my change broke the frontend".

Workaround that fixed it: `npm install` inside the worktree, giving it a real
`node_modules` (48 s). Confirmed afterwards that `@rumblefish/api-types`
resolved inside the worktree.

CI is unaffected — it does a plain branch checkout.

## Defect 2 — `.husky/_` is not created, so hooks never fire

`core.hooksPath` is `.husky/_`, and that directory does not exist in a fresh
worktree. Git finds no hook and proceeds.

Proven, not inferred: a file containing `fn broken( {` was staged and committed
**successfully**. After `npx husky` in the same worktree, the identical commit
was rejected with `rustfmt … unclosed delimiter` / `husky - pre-commit script
failed (code 1)`.

So a commit made in a fresh worktree carries no formatting, lint, typecheck or
test gate, and nothing says so.

## Why this matters beyond convenience

The repo's rule is that hooks are never bypassed with `--no-verify`. That rule
assumes the hooks run. Here they were absent while appearing present — the
`.husky/pre-commit` script file exists and reads correctly, so an inspection of
the repo suggests the gate is armed when it is not.

## Implementation

- Provision a real `node_modules` per worktree (an `npm install` in the
  provisioning step), or point the `@rumblefish/*` links at the worktree's own
  `libs/`. Do not leave the shared symlink — it silently couples every worktree
  to the main checkout's branch.
- Run `npx husky` as part of provisioning so `.husky/_` exists.
- Consider a cheap guard that fails loudly when `.husky/_` is missing, so an
  ungated commit is impossible rather than merely unlikely.

## Acceptance Criteria

- [ ] A newly created worktree typechecks its **own** `libs/`, proven with the
      main checkout parked on a different branch
- [ ] A newly created worktree rejects a deliberately malformed staged file
- [ ] Existing worktrees have a documented one-liner to repair both
- [ ] The worktree-hygiene guidance mentions both, with the symptom, since the
      typecheck symptom points at innocent files

## Notes

- Repair for an existing worktree: `npm install` then `npx husky`, both run
  inside the worktree.
- Related lesson worth keeping: a green or red local gate is only as trustworthy
  as its wiring. Both defects were found by probing the gate with a known-bad
  input, not by reading configuration.
