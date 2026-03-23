# Domain

Shared domain types and business logic for explorer data.

This library is the default home for code that describes the explorer domain in
an application-agnostic way.

## Put here

- Core domain types used by multiple apps.
- Business entities and value objects, for example ledgers, transactions,
  contracts, balances, cursors, and filters.
- Pure domain helpers that operate on those types.
- Shared frontend/backend types when they represent the same business concept on
  both sides.

## Good examples

- `LedgerPointer`
- `TransactionPointer`
- Pagination or query objects tied to explorer data
- Domain enums and discriminated unions

## Do not put here

- React components, hooks, styling, or any presentation-oriented types
- Generic utilities with no business meaning
- Transport-only DTOs if they become a separate API contract layer
- App wiring, framework setup, or runtime integration code

## Rule of thumb

If a type name still makes sense when discussed with no reference to frontend,
backend, HTTP, or a framework, it probably belongs here.
