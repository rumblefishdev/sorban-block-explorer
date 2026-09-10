---
id: '0548'
title: 'Protocol 28 (Adapter) readiness: Galexie 28.0.1 pin + stellar-xdr 27→28 before the 2026-09-16 pubnet vote'
type: OPS
status: active
related_adr: []
related_tasks: ['0367', '0368']
tags: [indexer, xdr, infra, galexie, protocol-28, priority-high]
links:
  - 'https://stellar.org/blog/developers/adapter-protocol-28-upgrade-guide'
  - 'https://stellar.org/blog/developers/introducing-adapter-protocol-28-on-stellar'
  - 'https://hub.docker.com/r/stellar/stellar-galexie/tags'
history:
  - date: 2026-09-08
    status: active
    who: karolkow
    note: >
      Created 8 days before the mainnet vote, from the SDF upgrade
      announcement. First PLANNED execution of the protocol-upgrade bump —
      0367/0368 were both reactive (16 h silent ingestion stall, then a
      7.5k-message DLQ). 0367's own future-work list called for exactly this.
---

# Protocol 28 (Adapter) readiness

## Summary

Pubnet votes to protocol 28 on **2026-09-16 17:00 UTC**. Two independent things
break at that instant if untouched: the digest-pinned Galexie image still ships a
protocol-27 captive core (stops writing to S3 → ingestion starves), and the
workspace still pins `stellar-xdr = "27"`, which cannot decode the new XDR union
arms (parse error → DLQ). Both failures already happened once, on the 26→27
upgrade — this task is the same work done _before_ the vote instead of after.

## Context

Repeat of the 2026-07-08/09 incident pair:

- **0367** — Galexie ran a pre-27 core through the protocol-27 vote. Captive core
  kept following SCP but could not apply new-protocol ledgers
  (`History: Skipping catchup: incompatible core version`), stalled mid-sync and
  wrote nothing to S3. Ran ~16 h before a human noticed. Galexie's version tracks
  the protocol, so **28.0.x == protocol 28**.
- **0368** — the indexer binary linked `stellar-xdr 26`, so the first proto-27
  ledger (63401875) failed `HandlerError::Parse`; the reconcile aborted before
  commit and the SQS doorbell redelivered until it dead-lettered (~7.5k messages).

What is different this time: we have advance notice, so both halves are planned
work rather than an outage. 0367's _Future Work_ listed "protocol-upgrade watch
process … so the Galexie/core bump is a planned ~20-min task before each pubnet
upgrade, not an outage" — this task is that process firing for the first time.

### Protocol 28 content (three CAPs, two touch us)

- **CAP-83 — empty transaction set consensus.** New `STELLAR_VALUE_EMPTY_TX_SET`
  arm on the `StellarValue.ext` union inside `LedgerHeader.scpValue`, with
  `txSetHash` all-zero. **No new `LedgerCloseMeta` version** — the container stays
  V2. We only read `scp_value.close_time` (`crates/xdr-parser/src/ledger.rs:18`),
  so there is no logic change; the exposure is purely that stellar-xdr 27 cannot
  decode an unknown union arm, and `deserialize_batch`
  (`crates/xdr-parser/src/lib.rs:111`) fails the **whole batch**, not one ledger.
- **CAP-85 — external contract executables.** New
  `CONTRACT_EXECUTABLE_EXTERNAL_REF` arm on `ContractExecutable`: a contract's
  code can now be a reference to an executable owned by _another_ contract, so an
  admin can upgrade a whole fleet atomically. Breaks the assumption that a
  contract instance carries its own `wasm_hash` — see Future Work.
- **CAP-86 — sparse map host functions.** Host functions only, no XDR change.
  No impact here.

### Blast radius in this workspace

`stellar-xdr` is pinned once, at `Cargo.toml:40`, and used by 7 crates
(xdr-parser, api, db-clickhouse, indexer, enrichment-shared, backfill-runner,
audit-harness). The new `ContractExecutable` arm lands on three **exhaustive**
matches in production code, so the bump fails at compile time rather than
silently rendering the wrong thing:

- `crates/xdr-parser/src/scval.rs:77`
- `crates/xdr-parser/src/operation.rs:835`
- `crates/xdr-parser/src/invocation.rs:558`

(`crates/xdr-parser/src/token_metadata.rs:87` compares with `==` against
`StellarAsset` and is unaffected.)

Unlike 26→27, no module reshuffle is expected: the `curr`/`next` split was
already removed in 27, so this should be a pin bump plus the three match arms.

Not affected: Soroban RPC is SDF-public (`mainnet.sorobanrpc.com`,
`DEFAULT_SOROBAN_RPC_URLS`, `crates/enrichment-shared/src/nft_token_uri/client.rs:43`) — SDF upgrades it. No
`@stellar/*` JS dependency exists in the frontend. We run no validators, so the
2026-09-09 validator-arming deadline does not apply.

## Implementation Plan

### Step 1 — Galexie image (infra; must land before 2026-09-16 17:00 UTC)

`stellar/stellar-galexie:28.0.1` was published 2026-08-27 (28.0.0 on 2026-08-14).
Mirror it into ECR, read the **landed ECR digest back**, and pin that:

- `galexieImageTag` in `infra/envs/production.json:24`
  (currently `sha256:91eae7af…3c82c8` = the Galexie 27.0.0 mirror from 0367)
- GitHub env `GALEXIE_IMAGE_DIGEST` (production + staging) → the **Docker Hub**
  source digest

Per 0367: the ECR digest differs from the Hub digest because `docker push`
re-serialises the manifest, and `galexieImageTag` is resolved via
`fromEcrRepository`, so it must hold the ECR one. Read it back with
`aws ecr batch-get-image … imageId.imageDigest`, never reuse the Hub digest.

Deploy is the user's (`aws-admin` shell). Budget ~20 min of warmup on the
restart: a fresh Fargate task has empty ephemeral storage and re-downloads and
applies ~16 GB of BucketList state before it resumes exporting.

### Step 2 — stellar-xdr 27 → 28 (code)

1. `Cargo.toml:40` → `stellar-xdr = { version = "28" }` (28.0.0 is on crates.io,
   published 2026-07-30).
2. Handle `ContractExecutable::ExternalRef` at the three sites above. Render it
   honestly — an explicit `external_ref` type carrying the owner reference, never
   a fallback that makes it look like a plain `wasm` executable
   (a plausible-but-wrong render is worse than a loud failure).
3. `cargo build --workspace --all-targets` + `cargo test -p xdr-parser` to catch
   any field-level struct change beyond the new arms.
4. Regenerate api-types — `Cargo.{toml,lock}` change trips the
   `API types freshness` CI gate: `npx nx run @rumblefish/api-types:generate`.

### Step 3 — verify decode before the vote

Testnet is **already on protocol 28** — `getLatestLedger` against
`soroban-testnet.stellar.org` returned `protocolVersion: 28` at ledger 4566692
(checked 2026-09-08; mainnet returned 27 at ledger 64329242 the same second). So
proto-28 ledgers exist to test against today. Decode one as a positive control
rather than waiting for mainnet to prove it.

### Step 4 — deploy + watch

Deploy compute after the vote window; watch `production-galexie-ingestion-lag`
(the alarm 0367 fixed: SQS `NumberOfMessagesSent`, 5-min window,
`treatMissingData: BREACHING`) and the DLQ depth across 17:00 UTC.

## Acceptance Criteria

- [ ] `galexieImageTag` pinned to the Galexie 28.0.1 ECR digest, read back from
      ECR (not copied from Docker Hub)
- [ ] GitHub env `GALEXIE_IMAGE_DIGEST` updated (production + staging)
- [ ] Galexie 28.0.1 live in prod, S3 exports flowing, before 2026-09-16 17:00 UTC
- [ ] Workspace `stellar-xdr` = 28; `cargo build --workspace --all-targets` green
- [ ] `ContractExecutable::ExternalRef` handled at all three render sites, with a
      test per site; no fallback that mimics `wasm`
- [ ] A testnet proto-28 ledger decodes clean through `deserialize_batch`
- [ ] Post-vote: indexer decodes mainnet proto-28 ledgers, DLQ stays empty,
      ingestion-lag alarm quiet
- [ ] **Docs updated** — TBD at PR time; expected `N/A` (no change to schema,
      endpoints, pipeline steps or topology as described in
      `docs/architecture/**`), same reasoning as 0367/0368
- [ ] **API types regenerated** — required, `Cargo.{toml,lock}` change

## Future Work

Not auto-created as backlog tasks — pending confirmation:

- **CAP-85 semantics vs `wasm_hash` and the upgradeable badge.** External-ref
  executables mean a contract's code identity lives in another contract. That
  undercuts the stale-`wasm_hash` fix (0320/0326) and the upgradeable badge
  (0327), whose premise — "does this contract import the upgrade host function" —
  is wrong for a fleet member whose mutability sits with the owner. Also gives
  `contract_deployments` a third executable kind with no direct hash. Not a
  Sep-16 blocker: it only bites once mainnet contracts actually use external refs.
- **Sibling `prices` repo.** `prices-production-ledger-processor` builds from a
  separate repo and needs the same stellar-xdr bump. 0368 left the 26→27 bump
  there open as an external follow-up — confirm whether that ever landed before
  assuming 28 is the only gap.
- Carried over, still open from 0367: ledger-advance healthcheck (replace
  `pgrep -x stellar-core`), and persistent BucketList state (EFS) to cut the
  ~20-min restart warmup.

## Notes

- Key dates: Core stable 2026-08-13 · testnet vote 2026-08-27 17:00 UTC ·
  validator arming 2026-09-09 17:00 UTC (not ours) · **mainnet vote 2026-09-16
  17:00 UTC**.
- Source announcement came from the team Slack on 2026-09-08, linking the two SDF
  posts in `links` above.
