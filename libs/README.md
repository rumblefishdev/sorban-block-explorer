# Libs

Libraries in this workspace contain reusable code that should not live inside a
single application from `apps/*`.

Use this directory when code is meant to be shared, versioned, built, and
validated as its own unit.

## Quick placement guide

- Put domain models, business terms, and explorer concepts in `libs/domain`.
- Put generic cross-cutting helpers in `libs/shared`.
- Put frontend-only presentation primitives in `libs/ui`.

## Decision rule

Ask what the code describes:

- If it describes the business domain, it belongs in `domain`.
- If it is generic technical infrastructure with no UI and no business meaning,
  it belongs in `shared`.
- If it exists to render, style, compose, or support the frontend experience, it
  belongs in `ui`.

## Shared types used by frontend and backend

Default to `libs/domain` when the types describe the same real business object
on both sides, for example blocks, ledgers, transactions, filters, or explorer
state.

Do not place frontend/backend shared types in `libs/ui`.

Do not use `libs/shared` as a catch-all for business types just because both
frontend and backend need them.

If request/response DTOs or transport contracts become large enough to deserve
their own boundary, create a dedicated library such as `libs/contracts` instead
of overloading `shared`.

## What not to do

- Do not place app-specific logic here if only one app will use it.
- Do not mix UI concerns into `domain` or `shared`.
- Do not mix business meaning into `shared` when `domain` is a better fit.
