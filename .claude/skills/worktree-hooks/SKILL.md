---
name: worktree-hooks
description: Make husky pre-commit/pre-push hooks run in a git worktree — never bypass with --no-verify. Use before committing or pushing from a worktree, and whenever a hook fails with "Could not find Nx modules" / "Have you run npm install?" / lint-staged errors. The fix is to provision node_modules, not to skip the gate.
---

# /worktree-hooks — never bypass the commit gate in a worktree

Fresh git worktrees have **no `node_modules`**, so husky hooks blow up:

```
NX   Could not find Nx modules at "…/worktree".
Have you run npm/yarn install?
husky - pre-commit script failed (code 1)
```

The tempting shortcut — `git commit --no-verify` / `git push --no-verify` —
is **FORBIDDEN**. It pushes unformatted or broken code straight to `develop`
(hooks are the only gate before it lands). Provision `node_modules` instead.

## Hard rule

- **Never** pass `--no-verify` (or `-n`) to `git commit` / `git push` to get
  around a failing husky hook. Not for code, not "just for a docs change".
- If a hook fails because tooling is missing, **fix the environment**, then
  re-run the commit/push with hooks enabled.
- The only acceptable hook failures to act on are **real** ones (your diff is
  unformatted, lint/typecheck/clippy is red) — fix the diff, don't silence it.

## What the hooks gate (why bypass is dangerous)

- `.husky/pre-commit` → `npx lint-staged && npm run -s verify:staged`
  - `lint-staged`: `rustfmt` on `*.rs`, `nx format:write --files` (prettier) on
    everything else.
  - `verify:staged`: `tools/scripts/run-affected-checks.mjs staged` → lint /
    typecheck / build on **nx-affected** projects.
- `.husky/pre-push` → `cargo clippy --all-targets -- -D warnings` +
  `verify:push` (affected lint/typecheck/build).

Skipping these = prettier drift, type errors, and clippy warnings on `develop`.

## Fix: provision node_modules in the worktree

### One command

```bash
sh tools/scripts/worktree-node-modules.sh
```

Clones the main checkout's `node_modules` — ~20 s, ~30 MB of disk, and it
verifies the result before returning. Idempotent, and it repairs a worktree
that already carries the broken symlink shape below. Falls back to `npm ci`
when the lockfile differs from main's.

### Never symlink node_modules — this is the bug that keeps coming back

`ln -s "$MAIN/node_modules" node_modules` looks like the cheap answer and was
this skill's own advice until it was traced. It is wrong, and it fails in a way
that points at innocent files.

npm workspaces (`workspaces: ["libs/*", "infra", "web"]`) materialise each
workspace package as a symlink inside `node_modules/@rumblefish/`, **relative to
node_modules' own location**:

```
node_modules/@rumblefish/soroban-block-explorer-ui -> ../../libs/ui
```

Resolve that through a symlinked `node_modules` and `../../` lands in the MAIN
checkout. So the worktree's `web` compiles against **main's** `libs/ui` and
`libs/api-types` — whatever branch main happens to be parked on. The symptom is
~25 TypeScript errors in files your branch never touched, blocking every commit
including lore-only ones, with nothing wrong in your diff. The repo has no
`paths` mapping in `tsconfig.base.json`, so `node_modules` is the only
resolution route and there is no second opinion to catch it.

A real directory fixes it because the same relative symlink then resolves inside
the worktree. On APFS `cp -c` is a copy-on-write clone, so a real directory
costs neither the 1 GB nor the minutes an install would:

| Shape           | Disk   | Time    | Workspace packages resolve to  |
| --------------- | ------ | ------- | ------------------------------ |
| `ln -s` to main | 0      | instant | **MAIN's libs — wrong branch** |
| `npm ci`        | 1 GB   | minutes | worktree's libs                |
| `cp -Rpc` clone | ~30 MB | ~20 s   | worktree's libs                |

Check any worktree by hand:

```bash
cd node_modules/@rumblefish/soroban-block-explorer-ui && pwd -P
```

The path must start with the worktree's own root. If it starts with the main
checkout, the layout is the broken one and every typecheck in that worktree is
lying to you.

### 3. Commit / push normally — hooks now run

```bash
git commit -m "type(lore-NNNN): …"   # NO --no-verify
git push origin HEAD:develop         # NO --no-verify; pre-push clippy runs
```

If `pre-push`'s `cargo clippy` is slow (cold `target/`), that is the cost of the
gate — let it run. If it fails on code you did **not** touch, `develop`'s clippy
is already red: stop and report it (do **not** bypass to get around someone
else's breakage).

## If you already bypassed (retro-fix)

Something already went out with `--no-verify`? Reconstruct the gate on the
pushed files and push a fix commit if anything is flagged:

```bash
# prettier
npx nx format:check --files <file...>            # exit 0 = clean
# build/lint/typecheck impact
npx nx show projects --affected --files <file...>  # []  = nothing to check
# rust
git diff <base>..HEAD --name-only | grep -q '\.rs$' && \
  SQLX_OFFLINE=true cargo clippy --all-targets -- -D warnings
```

`[]` affected + `format:check` exit 0 ⇒ the artifact happens to be clean (common
for `lore/*.md`-only changes) — record that it was verified after the fact. Any
non-empty affected set or format failure ⇒ fix and push a follow-up commit
through the hooks (not another bypass).

## Note for other lore skills

`promote-task`, `pr`, `branch` and any flow that commits from a worktree inherit
this rule: set up `node_modules` first, then commit/push with hooks on.
