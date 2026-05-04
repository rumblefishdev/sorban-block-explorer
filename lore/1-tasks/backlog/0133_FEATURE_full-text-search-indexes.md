---
id: '0133'
title: 'DB: trigram index on assets.name for search-by-name (post-MVP parity)'
type: FEATURE
status: backlog
related_adr: ['0039']
related_tasks: ['0053', '0156']
depends_on: ['0053', '0156']
tags: [priority-low, effort-small, layer-db, post-mvp, audit-F22]
milestone: 2
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F22 (MEDIUM). Global search (task 0053) depends on this.'
  - date: '2026-05-04'
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-scoped after 0053 PR #155 review. Original scope (4 indexes for
      tokens/accounts/NFTs/contracts) was largely overlap with what 0053
      already ships (idx_assets_code_trgm, idx_nfts_name_trgm,
      idx_nfts_collection_trgm, idx_contracts_search). Real residual is
      a single missing index — `assets.name` trigram — needed for
      "human-readable token name" search per frontend-overview §6.15
      (line 552). Tech-design overview and SCF submission do not require
      this; treated as post-MVP parity feature versus stellar.expert /
      etherscan. Dropped: accounts.home_domain (no scope justification
      anywhere), nfts.search_vector (already exists per ADR 0039),
      soroban_contracts.search_vector extension (already covers
      contract_id per migration 0002). Priority lowered to low; effort
      down to small. Depends on 0053 merge (PR #155) and 0156 merge
      (Soroban token name extraction — without it, Soroban-native
      token names remain NULL and trigram returns nothing for them).
---

# DB: trigram index on assets.name for search-by-name (post-MVP parity)

## Summary

Add a `pg_trgm` GIN index on `assets.name` so `GET /search` can match human-readable token names (e.g. "Stellar Lumen", "USD Coin") in addition to the asset_code prefix already covered by `idx_assets_code_trgm`. Extend `22_get_search.sql` `asset_hits` CTE with a `name ILIKE` branch. Post-MVP parity feature; matches stellar.expert / etherscan UX.

## Context

PR #155 (task 0053) ships `GET /v1/search` with all six entity types (transaction, contract, asset, account, NFT, pool) using existing schema indexes. The asset CTE matches `asset_code ILIKE` only — `assets.name` is NOT in the WHERE clause. Frontend-overview §6.15 (line 552) lists "human-readable token names" as a search-input expectation; SCF submission and tech-design overview do not. This task closes that single gap.

Aktualny stan po analizie:

| Wanted                                                 | State                                         |
| ------------------------------------------------------ | --------------------------------------------- |
| `assets.name` substring search                         | **MISSING** — this task                       |
| `assets.asset_code` trigram                            | EXISTS (`idx_assets_code_trgm`)               |
| `nfts.name` trigram                                    | EXISTS (`idx_nfts_name_trgm`, ADR 0039)       |
| `nfts.collection_name` trigram                         | EXISTS (`idx_nfts_collection_trgm`, ADR 0039) |
| `soroban_contracts.search_vector` (name + contract_id) | EXISTS (migration 0002:58-66)                 |
| `accounts.home_domain`                                 | OUT OF SCOPE (no doc requires it)             |

## Implementation

Single migration:

```sql
CREATE INDEX idx_assets_name_trgm ON assets USING GIN (name gin_trgm_ops);
```

Extend `docs/architecture/database-schema/endpoint-queries/22_get_search.sql` `asset_hits` CTE:

```sql
asset_hits AS (
    SELECT 'asset'::text AS entity_type, ..., a.id::bigint AS surrogate_id
    FROM assets a
    WHERE $7 = TRUE
      AND (
              -- existing: classic / SAC / Soroban with asset_code
              (a.asset_code IS NOT NULL AND a.asset_code ILIKE '%' || $1 || '%')
              -- existing: native XLM
           OR (a.asset_type = 0 AND ($1 ILIKE 'xlm' OR $1 ILIKE 'native'))
              -- NEW: human-readable name (this task)
           OR (a.name      IS NOT NULL AND a.name      ILIKE '%' || $1 || '%')
          )
    LIMIT $4
)
```

Update `crates/api/src/search/queries.rs` accordingly (port the SQL change verbatim, per the 0053 pattern).

## Acceptance Criteria

- [ ] Migration creates `idx_assets_name_trgm` GIN trigram on `assets.name`
- [ ] `22_get_search.sql` `asset_hits` CTE matches `name ILIKE` in addition to `asset_code`
- [ ] `crates/api/src/search/queries.rs` ported verbatim from updated SQL
- [ ] Documentation updated per ADR 0032: `database-schema-overview.md §4.10` lists the new index; `backend-overview.md §6.3` Search notes name-match capability
- [ ] Unit / integration test: search query matching a name substring (e.g. "Lumen", "USD Coin") returns the corresponding asset row
- [ ] No regression on existing asset_code or native-XLM matching

## Future Work

If post-launch UX feedback shows users searching by issuer domain (e.g. "circle.com" to find USDC issuer), spawn a follow-up task adding `accounts.home_domain` lookup. Out of scope here.
