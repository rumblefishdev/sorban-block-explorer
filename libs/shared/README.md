# Shared

Cross-cutting utilities with no domain-specific dependencies.

This library is for low-level reusable code that is technically useful across
the workspace but is not part of the explorer business model and is not tied to
the UI.

## Put here

- Generic helpers with broad reuse
- Common utility types that are not business-domain concepts
- Reusable constants or small abstractions with no domain vocabulary
- Narrow cross-cutting helpers used by multiple libraries or apps

## Good examples

- Generic `Maybe<T>` or `Nullable<T>` style helper types
- String, date, array, or object utilities with no explorer-specific meaning
- Shared error/result helpers if they are domain-agnostic

## Do not put here

- Types like transaction, ledger, contract, or explorer filter models
- Frontend presentation types or reusable UI primitives
- API DTOs just because they are used in more than one place
- A growing dump of unrelated code

## Rule of thumb

If removing all Soroban or explorer terminology from the file still leaves the
code meaningful, `shared` may be the right place.

If the code starts using business language, move it to `domain` instead.
