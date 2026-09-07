## Session Gate

**Before any work, verify:**

| Check | File | If Missing |
|-------|------|------------|
| **Who** | `lore/0-session/current-user.md` | Auto-generated from `LORE_SESSION_CURRENT_USER` env |
| **What** | `lore/0-session/current-task.md` | Pick from `lore/0-session/next-tasks.md`, use MCP tool `lore_set-task` |

## File Size & Test Placement

- Production module ≤ ~800 lines. Split BY TOPIC (one concern per file),
  never by layer ("all queries of the module" is how god files grow).
- Tests always live in a sibling file, never inline: Rust `foo_tests.rs`
  via `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`, TS `foo.test.ts(x)`.
- Verification-only code (oracles, corpus checks) belongs in the crate's
  `tests/` directory, not in the production module it verifies.
- Touching a file that exceeds the limit? Extract at least its tests in the
  same PR. New files must not be born over the limit.
- The existing stock shrinks incrementally — task 0525 tracks the backlog —
  never in a big-bang refactor.

## File Deletion Policy

**Using `rm` is FORBIDDEN.** To delete files, move them to the MAIN
checkout's `.trash/` instead — resolve it rather than typing a relative path,
which lands in whatever worktree you happen to be in:
```bash
mv <file> "$(git worktree list --porcelain | awk '/^worktree /{print $2; exit}')/.trash/"
```

`.trash/` is gitignored, so it holds the only copy of anything that was never
committed. Move files to the **main checkout's** `.trash/`, never a worktree's:
removing a merged or abandoned worktree deletes its trash with it, and the
policy is then satisfied in letter while the file is gone.

## Task-Gated Development

**Writing code without an active task is FORBIDDEN.**

## API Types Codegen — regenerate before commit

If a PR touches **any of these paths**, the OpenAPI spec changes and the
TypeScript types in `libs/api-types/src/{openapi.json,generated/}` MUST
be regenerated before commit (CI gate `API types freshness`):

- `crates/api/**` (handlers, DTOs, openapi schemas, routes)
- `Cargo.toml` / `Cargo.lock`
- `libs/api-types/**`

Command:

```bash
npx nx run @rumblefish/api-types:generate
```

This runs `cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json`
followed by `openapi-ts` codegen. Stage the resulting changes (`openapi.json` +
`generated/*`) in the same commit as the API change.

CI runs `nx run @rumblefish/api-types:check-generated` (a `git diff --exit-code`
on those paths). Skipping the regen → red `API types freshness` check.

## Evergreen Architecture Docs

Per [ADR 0032](./lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md),
every PR that changes the shape of the system MUST also update the relevant
files under `docs/architecture/**` in the same PR. "Shape of the system" means
anything describable in those docs: schema (column types, tables, partitioning,
constraints), API endpoints (added/removed/renamed), ingestion pipeline steps,
infrastructure topology, XDR parsing responsibilities, frontend data contracts.

The ADR template (`lore/2-adrs/_template.md`) and task template
(`lore/1-tasks/_template.md`) both carry a "Docs updated" checklist — use it.
Mark each doc file either "updated" or `N/A — reason`; never leave it blank.

Pure policy / process / tooling changes (e.g. CI configuration) that do not
affect the described architecture are legitimate `N/A` cases.

## Operational Guides

Single entry points for **running** the system (as opposed to `docs/architecture/**`,
which describes it). Read the matching one before operating — each links the
per-layer deep-dives (`infra/README.md`, `infra-hetzner/README.md`,
`docs/runbooks/**`) rather than duplicating them.

| Guide | Read before… | Non-obvious constraint it encodes |
|-------|--------------|-----------------------------------|
| [`docs/deployment.md`](./docs/deployment.md) | shipping anything | Production is the only environment and **every deploy is manual** from a laptop. The `staging` CI path is dead — `make deploy-staging` / `npm run infra:*:staging` do not exist. |
| [`docs/backfills.md`](./docs/backfills.md) | any backfill / re-parse | **`repair-tier1` is mandatory** after a parallel or `--reindex` run — RMT cannot express MIN semantics, so 12 Tier-1 columns corrupt silently. Some subcommands require the indexer stopped (`EXCHANGE TABLES` is the dividing line). |
| [`docs/backups.md`](./docs/backups.md) | backups / restore | After a restore the Lambdas **will not re-deliver** the rolled-back range — the gap must be re-ingested with `backfill-runner`. |

These encode durable rules. Keep transient state (which task is open, which PR is
pending) out of them — it rots and makes the guide untrustworthy.

## GitHub Issues

Issues close at **deploy**, never at merge — merged code is not shipped code.

Reference them as `Refs #123` in PR titles, bodies and commits. **Never
`Closes` / `Fixes` / `Resolves`**: those auto-close the issue when
`develop → master` merges, while the fix is still undeployed.

Triage, post-merge comments and the post-deploy sweep: `/issues`. It never
posts to GitHub — it drafts, a human sends.

## Context

@lore/0-session/current-user.md
@lore/0-session/current-task.md
@lore/0-session/next-tasks.md
@lore/CLAUDE.md

<!-- nx configuration start-->
<!-- Leave the start & end comments to automatically receive updates. -->

# General Guidelines for working with Nx

- For navigating/exploring the workspace, invoke the `nx-workspace` skill first - it has patterns for querying projects, targets, and dependencies
- When running tasks (for example build, lint, test, e2e, etc.), always prefer running the task through `nx` (i.e. `nx run`, `nx run-many`, `nx affected`) instead of using the underlying tooling directly
- Prefix nx commands with the workspace's package manager (e.g., `pnpm nx build`, `npm exec nx test`) - avoids using globally installed CLI
- You have access to the Nx MCP server and its tools, use them to help the user
- For Nx plugin best practices, check `node_modules/@nx/<plugin>/PLUGIN.md`. Not all plugins have this file - proceed without it if unavailable.
- NEVER guess CLI flags - always check nx_docs or `--help` first when unsure

## Scaffolding & Generators

- For scaffolding tasks (creating apps, libs, project structure, setup), ALWAYS invoke the `nx-generate` skill FIRST before exploring or calling MCP tools

## When to use nx_docs

- USE for: advanced config options, unfamiliar flags, migration guides, plugin configuration, edge cases
- DON'T USE for: basic generator syntax (`nx g @nx/react:app`), standard commands, things you already know
- The `nx-generate` skill handles generator discovery internally - don't call nx_docs just to look up generator syntax


<!-- nx configuration end-->