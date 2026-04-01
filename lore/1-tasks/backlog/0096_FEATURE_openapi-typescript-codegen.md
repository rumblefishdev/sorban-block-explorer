---
id: '0096'
title: 'OpenAPI → TypeScript codegen: shared types between Rust API and React frontend'
type: FEATURE
status: backlog
related_adr: ['0005']
related_tasks: ['0092', '0094']
tags:
  [priority-high, effort-small, layer-frontend, layer-backend, typescript, rust]
links:
  - https://github.com/hey-api/openapi-ts
  - https://docs.rs/utoipa/5.4.0/utoipa/derive.ToSchema.html
history:
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from 0092 research. Single source of truth for API types: Rust → OpenAPI → TypeScript.'
---

# OpenAPI → TypeScript codegen: shared types between Rust API and React frontend

## Summary

Generate TypeScript types, fetch client, and TanStack Query hooks from the Rust API's OpenAPI 3.1 spec. Single source of truth: Rust structs with `#[derive(ToSchema)]`. Frontend never writes API types manually.

## Context

Rust API (utoipa) generates OpenAPI 3.1 JSON. React frontend (TanStack Query) needs typed API access. Without codegen, types are duplicated across languages and drift over time.

Research task 0092 confirmed utoipa 5.4 generates OpenAPI 3.1 specs. This task wires codegen into the monorepo.

## Implementation

### 1. Extract OpenAPI spec from Rust (build-time, no server needed)

Add a secondary binary to the `api` crate (not a separate crate):

```rust
// crates/api/src/bin/extract_openapi.rs
fn main() {
    let spec = api::ApiDoc::openapi();
    println!("{}", spec.to_pretty_json().unwrap());
}
```

Run: `cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json`

### 2. Create `libs/api-types/` Nx library

```
libs/api-types/
├── src/
│   ├── openapi.json          # extracted spec (committed)
│   ├── generated/            # codegen output (committed)
│   │   ├── types.ts
│   │   ├── client.ts         # typed fetch client
│   │   └── hooks.ts          # TanStack Query hooks
│   └── index.ts              # re-exports
├── openapi-ts.config.ts      # @hey-api/openapi-ts config
└── project.json              # Nx targets
```

Committed generated files = frontend devs don't need Rust toolchain.

### 3. Install and configure `@hey-api/openapi-ts`

```bash
npm install -D @hey-api/openapi-ts
```

Config (`openapi-ts.config.ts`):

```typescript
export default {
  input: 'src/openapi.json',
  output: 'src/generated',
  plugins: [
    '@hey-api/types', // TypeScript types
    '@hey-api/client-fetch', // typed fetch client
    '@hey-api/tanstack-query', // TanStack Query hooks
  ],
};
```

### 4. Nx targets

```jsonc
{
  "targets": {
    "extract-openapi": {
      "command": "cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json"
    },
    "generate": {
      "command": "npx openapi-ts",
      "dependsOn": ["extract-openapi"]
    }
  }
}
```

### 5. CI validation

Run codegen + `git diff --exit-code libs/api-types/src/generated/` — fail if committed types are stale.

## Acceptance Criteria

- [ ] `crates/api/src/bin/extract_openapi.rs` extracts OpenAPI JSON without booting server
- [ ] `libs/api-types/` Nx library with generated types, client, and hooks
- [ ] `@hey-api/openapi-ts` configured with types + fetch client + TanStack Query plugins
- [ ] `nx run api-types:generate` produces up-to-date TypeScript from spec
- [ ] `web/` can import types: `import { Ledger, PaginatedResponse } from '@rumblefish/api-types'`
- [ ] CI step validates generated types are committed and up-to-date

## Notes

- Depends on task 0094 (Cargo workspace) — `extract_openapi` binary lives in `crates/api/`
- `@hey-api/openapi-ts` is v0.x but very actively maintained (2M+ weekly downloads, released today)
- Runner-up option: `openapi-typescript` + `openapi-fetch` + `openapi-react-query` (three packages, leaner output, more wiring)
- Existing `libs/domain/` TS types will be gradually replaced by generated types from this package
