# UI

Frontend-oriented presentation layer primitives.

This library contains code that exists for the frontend experience: rendering,
presentation, composition, and view-facing types.

## Put here

- Reusable UI components
- Presentation-only types used by components or routes
- Frontend composition helpers
- Design-oriented primitives, display models, and navigation structures

## Good examples

- `NavigationItem`
- Component props and view models
- Reusable layout and visual primitives

## Do not put here

- Backend-facing types shared with the API
- Core business entities such as ledgers or transactions
- Generic non-UI utilities that belong in `shared`
- Application bootstrap or page-specific code that is not reusable

## Rule of thumb

If the code would make no sense outside the frontend, it belongs here.

If a type is shared by frontend and backend, it usually belongs in `domain`, not
in `ui`.
