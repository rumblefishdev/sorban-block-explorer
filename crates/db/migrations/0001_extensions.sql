-- ADR 0027 — initial schema, step 1/7: extensions
-- pg_trgm is required for GIN trigram indexes on tokens.asset_code and nfts.name.

CREATE EXTENSION IF NOT EXISTS pg_trgm;
