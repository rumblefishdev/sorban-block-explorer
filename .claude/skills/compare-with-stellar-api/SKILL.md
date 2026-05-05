# /compare-with-stellar-api — Verify a DB query's output against Horizon + stellar.expert + raw XDR

Take a path to a SQL query, run it against the local Docker Postgres, pick 5 representative rows (mix of edge cases + random), then dispatch 3 parallel subagents to cross-check those rows against:

1. **Horizon API** — Stellar's canonical REST API
2. **stellar.expert** — community block explorer
3. **Raw XDR (parsed independently)** — fetch envelope/result/result_meta XDR and decode it locally; the most authoritative source because it bypasses every display layer

Aggregate the findings into a report and **STOP**. The user runs the local frontend test themselves.

## When to use

After implementing or modifying a backend read query, before claiming the data is correct. Especially useful for `docs/architecture/database-schema/endpoint-queries/*.sql`, but works on any read query that returns identifiable Stellar entities (transaction, account, contract, asset, ledger, NFT, liquidity pool).

## Argument

`/compare-with-stellar-api <path-to-sql-file>` — required. If anything else (endpoint name, table name, empty), **STOP** and ask for a path.

## Step 1 — Read the file & resolve which statement to verify

Look for `-- @@ split @@` (multi-statement separator from `endpoint-queries/README.md`):

- **0 occurrences** → single statement. `selected_statement = 1` (the whole file).
- **1+ occurrences** → **STOP**. Enumerate the statements (print first comment / leading SQL keyword of each, label A, B, C, …) and ask which one to verify. Record the user's choice as `selected_statement = N`. Do not auto-pick.

Carry `selected_statement` into every subsequent step.

## Step 2 — Run the query against local Docker Postgres

Two modes — pick one based on the path. **Both must end up with `selected_statement`'s output as JSON-per-row** (`SELECT row_to_json(t) FROM (…) t`) so step 3 can parse it cleanly.

### Mode A — standard endpoint query

Path matches `docs/architecture/database-schema/endpoint-queries/<NN>_*.sql`.

The helper does discovery + substitution + multi-statement orchestration (including `\gset` chaining of intermediate values between statements). Use it for orchestration, then re-issue just the selected statement wrapped in `row_to_json` to get clean structured output.

```bash
# 1. orchestrate (and learn what params the helper picked from the script's `echo` lines):
docs/architecture/database-schema/endpoint-queries/run_endpoint.sh <NN> -x

# 2. re-run the selected statement with row_to_json wrap, reusing the same params:
docker compose exec -T postgres psql -U postgres -d soroban_block_explorer \
  -v ON_ERROR_STOP=1 -t -A \
  -c "SELECT row_to_json(t) FROM ( <selected_statement_with_substituted_params> ) t;"
```

For `\gset`-dependent statements (E3 B-F, E6 B, E11 B), capture the upstream values from the helper's first run, then substitute them into the wrapped re-run.

### Mode B — ad-hoc query

Path is anywhere else.

1. Split the file on `-- @@ split @@` and extract statement `selected_statement` (1-indexed).
2. Parse the file's `Inputs:` header (per task 0167 convention) for the `$N` placeholders the chosen statement uses. Each entry is `--   $N  :name  TYPE  semantics`.
3. For each `$N`, write a tiny discovery query against the local DB to pick a sample value. Use `NULL::<type>` for cursor / first-page params.
4. Run the substituted statement wrapped in `row_to_json`:

```bash
docker compose exec -T postgres psql -U postgres -d soroban_block_explorer \
  -v ON_ERROR_STOP=1 -t -A \
  -c "SELECT row_to_json(t) FROM ( <substituted_statement> ) t;"
```

`-t -A` gives one JSON object per line, no headers, no padding.

### STOP conditions for step 2

- Output has **0 lines** → STOP, report "empty result set; populate the table or check the query".
- Output has **1 line** → that's your only sample. Skip step 3 and use `n=1` rows in step 4.
- Otherwise aim for **≥20 lines** so step 3 has variance.

## Step 3 — Pick 5 sample rows

Goal: maximise edge-case coverage. **Do NOT take the first 5.** Parse the JSONL output from step 2 and pick:

- Nullable columns → at least one row where it's `null` and one where it's filled.
- Enum / type columns (`asset_type`, `event_type`, `op_type`, etc.) → coverage across distinct values.
- Boolean columns (`successful`, `has_soroban`, `is_sac`) → at least one of each value.
- Array / aggregate columns → prefer rows with non-trivial cardinality.
- Reserve **1-2 slots** for genuinely random rows (don't make the sample fully synthetic).

For each picked row, write a one-line description of _what it exemplifies_ (e.g. "row 3: failed Soroban tx with inner_tx_hash NULL and 4 distinct operation_types"). Carry these descriptions into step 4.

## Step 4 — Dispatch 3 parallel verifier subagents

**Send a single message with 3 Agent tool calls** so they run in parallel. All three receive the **same** rows + **same** field list — that's the entire point of central sampling.

### Build the field list (do this carefully, it's a common bug source)

For `endpoint-queries/` files: open `endpoint-queries/README.md` `## Endpoint response shapes`, find the section for the endpoint, find the table for `selected_statement`. **Include ONLY rows with `DB →` in the Source column.** Exclude:

- `Archive →` (XDR overlay added by API layer at runtime; not in the DB output — and the third subagent will independently verify these against parsed XDR anyway)
- `S3 →` (off-DB blob; not in the DB output)
- Synthesized fields like `cursor`, `position` (computed in API)

For ad-hoc queries: use the projected column names directly.

If you skip this filter, every Archive/S3 field will falsely report `MISMATCH` or `SOURCE_MISSING` and the report becomes noise.

### Determine the entity type

One of: `transaction`, `account`, `contract`, `asset`, `ledger`, `nft`, `liquidity_pool`. Infer from the query's primary table or its endpoint header.

### Subagent prompt template (use literally — fill placeholders, do not improvise structure)

The placeholder formats:

- `{ROWS_WITH_DESCRIPTIONS}`: 5 (or `n`) numbered markdown blocks. Each block: a `Row N: <description>` heading line, then a JSON code fence with the row object from step 2.
- `{FIELD_LIST}`: markdown bulleted list, one field per line, no extra prose.

Subagent prompt:

```
You are verifying Stellar entity data from our local Postgres against {SOURCE_NAME} ({SOURCE_BASE_URL}).

Entity type: {ENTITY_TYPE}
URL pattern hint for {SOURCE_NAME}: {URL_PATTERN_HINT}
(Derive the exact URL per row from the entity identifier in the row data.)

For each of the rows below, fetch the corresponding entity from {SOURCE_NAME} and compare every listed field. Use {FETCH_TOOL}. Report match/mismatch/source-missing per field per row.

Rows to verify (each with a one-line description of what it exemplifies):

{ROWS_WITH_DESCRIPTIONS}

Fields to verify (same list for every row):

{FIELD_LIST}

Output EXACTLY this format, replacing the angle-bracket placeholders with concrete values:

Row 1 (<row 1 description verbatim>):
  - <field_a>: MATCH
  - <field_b>: MISMATCH (DB=<db_value>, source=<source_value>)
  - <field_c>: SOURCE_MISSING (<one-line reason — e.g. 404, not indexed, archived>)
  - <field_d>: NOT_APPLICABLE (<reason — e.g. Horizon doesn't cover Soroban events>)
  - <field_e>: UNVERIFIABLE (<reason — page returned JS shell / no XDR available / etc.>)
Row 2 (<description>): …
…

Notes: <anything else — rate limits, partial fetches, suspected source bugs>

Hard rules:
- Do NOT guess. If you can't extract a field, mark UNVERIFIABLE or SOURCE_MISSING with a reason.
- Distinguish SOURCE_MISSING (entity not on this source — usually not a DB bug) from MISMATCH (entity present, value differs — likely a real bug).
- If rate-limited mid-run, finish what you can and note in "Notes:" how many rows you completed.
- Do NOT pick your own rows. Use exactly the rows provided.
```

### Per-source overrides for the template

| #   | SOURCE_NAME                  | SOURCE_BASE_URL                                                     | URL_PATTERN_HINT                                                                                                                                                                                                                              | FETCH_TOOL      |
| --- | ---------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| 1   | Horizon API                  | `https://horizon.stellar.org`                                       | `/transactions/<hash>`, `/accounts/<id>`, `/ledgers/<seq>`, `/assets?asset_code=<c>&asset_issuer=<i>`, `/liquidity_pools/<hex>`. **Does not cover Soroban contracts/events/invocations/NFTs** — for those, mark every field `NOT_APPLICABLE`. | WebFetch        |
| 2   | stellar.expert               | `https://stellar.expert/explorer/public`                            | `/tx/<hash>`, `/account/<id>`, `/asset/<code>-<issuer>`, `/contract/<id>`, `/liquidity-pool/<hex>`.                                                                                                                                           | WebFetch        |
| 3   | Raw XDR (independent decode) | n/a — fetch from Horizon `/transactions/<hash>` then decode locally | See "Subagent 3 specifics" below.                                                                                                                                                                                                             | WebFetch + Bash |

### Subagent 3 specifics (Raw XDR)

This subagent does **independent XDR decoding** so it doesn't trust any explorer's display layer.

**Scope:**

- Applicable to: `transaction`, plus any per-tx-derived field (`contract` events/invocations, ops, participants, signatures, memo) where the data lives inside a transaction's XDR.
- Not applicable: `account` current state, `asset` totals, `ledger` aggregates, `nft`, `liquidity_pool` reserves — there's no per-entity XDR for these. Mark every field `NOT_APPLICABLE` with reason "no XDR for this entity type".

**Procedure for in-scope entities:**

1. For each row, derive the parent transaction hash from the row data.
2. Fetch the XDR via `curl` (NOT WebFetch — WebFetch can silently corrupt long base64 strings; observed in practice as random characters inserted mid-string). Use `jq` to extract:
   ```bash
   curl -s "https://horizon.stellar.org/transactions/<hash>" \
     | jq -r '{envelope_xdr, result_xdr, result_meta_xdr, inner_transaction}'
   ```
   For fee-bumped rows, also fetch `/transactions/<inner_tx_hash_hex>` if you need the inner envelope separately (or drill into outer's `feeBump.tx.innerTx`).
3. Decode each XDR blob. Probe order (try each, use first that can handle the envelope type):

   - **py-stellar-sdk** (PREFERRED — covers both classic AND Soroban envelopes, ~100-200 ms per decode):
     - Probe: `python3 -c "import stellar_sdk"` succeeds, OR `~/.local/venvs/stellar-sdk/bin/python3 -c "import stellar_sdk"` succeeds (project may use a venv on PEP 668 systems).
     - Usage: pipe base64 via stdin to a small Python script using `stellar_sdk.xdr.TransactionEnvelope.from_xdr(...)` etc., extract fields, print.
   - **stellar-cli** (CLASSIC ENVELOPES ONLY — hard-fails on Soroban with `"xdr value max length exceeded"`, no flag raises the limit):
     - **Skip for any row where `has_soroban = true`** — go directly to py-stellar-sdk or UNVERIFIABLE.
     - Native: `which stellar` → `stellar xdr decode --type <Type> --input single-base64 --output json <base64>`
     - Docker: `docker image inspect stellar/stellar-cli:latest` (or `docker pull stellar/stellar-cli`) → `docker run --rm -i stellar/stellar-cli xdr decode --type <Type> --input single-base64 --output json <<< "<base64>"` (~200-500 ms per call for container startup).
   - **stellar-base** (Node): `node -e "require('stellar-base')"` succeeds — fallback if Python is also missing.
   - **If no decoder is available for the envelope type at hand**: mark every field of that row `UNVERIFIABLE` with reason `"no XDR decoder available; for full coverage (classic + Soroban) install py-stellar-sdk: 'pip install --break-system-packages stellar-sdk' (quick) or 'python3 -m venv ~/.local/venvs/stellar-sdk && ~/.local/venvs/stellar-sdk/bin/pip install stellar-sdk' (clean). stellar-cli alone covers classic envelopes only."`. Finish cleanly. Do NOT compile a Rust runner against `crates/xdr-parser` — too heavy for verification.

   Common `--type` values for stellar-cli: `TransactionEnvelope` for `envelope_xdr`, `TransactionResult` for `result_xdr`, `TransactionResultMeta` for `result_meta_xdr`. The CLI returns JSON; parse with `jq` to reach individual fields.

4. From the decoded XDR, extract the field values to compare:
   - `successful` ← `result_xdr → TransactionResultCode == txSUCCESS`
   - `operation_count` ← `envelope_xdr → operations.length`
   - `source_account` ← `envelope_xdr → sourceAccount` (G-StrKey)
   - `fee_charged` ← `result_xdr → feeCharged`
   - `memo_type`, `memo_content` ← `envelope_xdr → memo`
   - per-op `type`, `source_account`, `destination_account`, etc. ← `envelope_xdr → operations[i]`
   - per-event `topics`, `data` ← `result_meta_xdr → events[]`
   - per-invocation `function_name`, `args`, `return_value` ← `envelope_xdr → operations[i].body.invokeHostFunctionOp` + `result_meta_xdr`
5. Report per the standard output format. The whole point: this subagent's MATCH means "the XDR itself confirms the DB", which is much stronger than "Horizon's display agrees".

## Step 5 — Frontend-contract check

`grep` `docs/architecture/frontend/frontend-overview.md` for the route or endpoint name (e.g. `GET /transactions`, `/accounts/:id`, `/liquidity-pools/:id/chart`).

- If a section listing required fields for that view is found → compare against the columns the query projects. Note (a) frontend-required fields missing from the response, (b) projected columns not consumed by the frontend.
- If no matching section is found → write one line in the report: `Frontend contract check skipped — no <route> section in frontend-overview.md`. Do not synthesize.

## Step 6 — Aggregate and present

In chat:

1. **Sample rows** — one line each, recapping what each exemplifies.
2. **Cross-source matrix** — per row, per source (Horizon / stellar.expert / Raw XDR), condensed result. Highlight fields where the 3 sources **disagree with each other** — especially when XDR disagrees with both explorers (strong signal of an explorer bug, not a DB bug).
3. **Pure mismatches** — fields where DB disagrees with all sources that have the entity. **Mismatches confirmed by Raw XDR are the highest-confidence DB bugs** (XDR is ground truth).
4. **All-sources-missing rows** — DB has the row, all 3 sources don't. Could be legitimate DB-only data, or could indicate the row shouldn't exist.
5. **Frontend contract** — pass / gaps / skipped.
6. **Caveats** — rate limits, missing XDR decoder, Soroban entities Horizon couldn't cover, partial subagent runs.

## Step 7 — STOP

Do not commit, do not modify code, do not spawn lore tasks. Wait for the user to run their local manual test and signal the next step.

## Result terminology (referenced from the subagent template)

| Result           | Meaning                                                  | Action signal                                                                                   |
| ---------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `MATCH`          | DB and source agree on field value                       | nothing                                                                                         |
| `MISMATCH`       | source has row, field differs                            | **investigate** — DB or source bug; weight by source authority (XDR > Horizon > stellar.expert) |
| `SOURCE_MISSING` | source has no such entity                                | usually known divergence — note, don't fix                                                      |
| `NOT_APPLICABLE` | source by design doesn't cover this                      | structural, ignore                                                                              |
| `UNVERIFIABLE`   | could not extract value (JS shell, no XDR decoder, etc.) | can't judge                                                                                     |

## Anti-patterns

- Letting subagents pick their own rows — breaks the entire cross-comparison premise.
- Running subagents sequentially instead of in one parallel message.
- Passing Archive/S3 fields to subagents when sourcing the field list from `endpoint-queries/README.md` — every Archive field will falsely report MISMATCH.
- Conflating `SOURCE_MISSING` with `MISMATCH` in the report.
- Treating a Horizon/stellar.expert MISMATCH as gospel when Raw XDR says MATCH — explorers can be wrong; XDR is the ground truth.
- Auto-creating follow-up lore tasks for findings — present in chat; the user decides what becomes a task.
