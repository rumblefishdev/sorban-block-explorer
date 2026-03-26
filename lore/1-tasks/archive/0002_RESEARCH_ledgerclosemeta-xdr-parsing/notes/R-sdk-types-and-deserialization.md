---
title: 'Rust XDR crate types, deserialization, and transaction hash computation'
type: research
status: mature
spawned_from: null
spawns: []
tags: [rust, stellar-xdr, deserialization, hash]
links:
  - https://github.com/stellar/rs-stellar-xdr
  - https://github.com/stellar/go-stellar-sdk
  - https://developers.stellar.org/docs/networks/software-versions
  - https://stellar.org/blog/developers/stellar-x-ray-protocol-25-upgrade-guide
  - https://github.com/stellar/stellar-galexie
history:
  - date: 2026-03-26
    status: mature
    who: stkrolikiewicz
    note: 'Rewritten as Rust-first with stellar_xdr::curr crate'
---

# Rust XDR crate types, deserialization, and transaction hash computation

## Crate and Versions

- **Crate:** [`stellar-xdr`](https://github.com/stellar/rs-stellar-xdr) v25
- **Module:** `stellar_xdr::curr` (current protocol XDR definitions)
- **Protocol:** 25 ("X-Ray") — current Stellar mainnet
- **Reference impl:** [`rumblefishdev/stellar-indexer`](../sources/stellar-indexer-ledger-mod-rs.md) (Rust)

```toml
[dependencies]
stellar-xdr = { version = "25", default-features = true, features = ["curr"] }
zstd = "0.13"
hex = "0.4"
```

The crate is auto-generated from stellar-core's XDR definitions. XDR union types are Rust enums — `match` is exhaustive and compiler-enforced.

## LedgerCloseMeta Deserialization

### From Galexie S3 output (LedgerCloseMetaBatch)

Galexie writes `LedgerCloseMetaBatch` files compressed with zstd. Each file contains 1+ `LedgerCloseMeta` entries (see [task 0001](../../archive/0001_RESEARCH_galexie-captive-core-setup/README.md)).

```rust
use stellar_xdr::curr::{LedgerCloseMetaBatch, Limits, ReadXdr};

let compressed = std::fs::read("ledger.xdr.zst")?;
let decompressed = zstd::decode_all(compressed.as_slice())?;

let limits = Limits {
    depth: 1000,
    len: decompressed.len().max(10_000_000),
};
let batch = LedgerCloseMetaBatch::from_xdr(decompressed.as_slice(), limits)?;

for meta in batch.ledger_close_metas.iter() {
    process_ledger(meta);
}
```

`from_xdr()` returns `Result<T, stellar_xdr::curr::Error>` — no exceptions, compiler-enforced.

### LedgerCloseMeta Versions

Current mainnet uses **V2**. Exhaustive match:

```rust
match &lcm {
    LedgerCloseMeta::V0(v) => { /* pre-generalized tx set */ }
    LedgerCloseMeta::V1(v) => { /* generalized tx set */ }
    LedgerCloseMeta::V2(v) => { /* current: eviction + Soroban state size */ }
}
```

Adding V3 would cause a compile error in every `match` — impossible to miss.

## LedgerCloseMetaV2 Fields

```rust
pub struct LedgerCloseMetaV2 {
    pub ledger_header: LedgerHeaderHistoryEntry,
    pub tx_set: GeneralizedTransactionSet,
    pub tx_processing: Vec<TransactionResultMeta>,
    pub upgrades_processing: Vec<UpgradeEntryMeta>,
    pub scp_info: Vec<ScpHistoryEntry>,
    pub total_byte_size_of_live_soroban_state: i64,
    pub evicted_keys: Vec<LedgerKey>,
    pub ext: ExtensionPoint,
}
```

Direct struct field access — no method calls, no `.unwrap()`.

## Ledger Header Extraction

```rust
let header = &v.ledger_header.header;

let sequence: u32 = header.ledger_seq;
let close_time: u64 = header.scp_value.close_time.clone().into();
let protocol_version: u32 = header.ledger_version;
let base_fee: u32 = header.base_fee;
let tx_count: usize = v.tx_processing.len();
```

Ledger hash = SHA-256 of serialized `LedgerHeaderHistoryEntry`:

```rust
use stellar_xdr::curr::WriteXdr;
use sha2::{Sha256, Digest};

let header_xdr = v.ledger_header.to_xdr(limits)?;
let ledger_hash = hex::encode(Sha256::digest(&header_xdr));
```

## Transaction Envelope Extraction (TX Set)

Protocol 25 has **two-phase TX set**. From [stellar-indexer](../sources/stellar-indexer-ledger-mod-rs.md):

```rust
use stellar_xdr::curr::{TransactionPhase, TxSetComponent};

fn for_envelopes_in_phase<F>(phase: &TransactionPhase, f: &mut F)
where F: FnMut(&TransactionEnvelope)
{
    match phase {
        TransactionPhase::V0(components) => {
            for comp in components.iter() {
                let TxSetComponent::TxsetCompTxsMaybeDiscountedFee(txs_comp) = comp;
                for env in txs_comp.txs.iter() { f(env); }
            }
        }
        TransactionPhase::V1(parallel) => {
            for stage in parallel.execution_stages.iter() {
                for cluster in stage.0.iter() {
                    for env in cluster.0.iter() { f(env); }
                }
            }
        }
    }
}
```

V0 = classic transactions. V1 = parallel Soroban execution stages (Protocol 25+).

## Transaction Hash Computation

Built-in method on `TransactionEnvelope`:

```rust
let network_id: [u8; 32] = hex::decode(NETWORK_ID_HEX)?
    .try_into().map_err(|_| "invalid network id")?;

for_each_envelope(meta, |env| {
    if let Ok(tx_hash) = env.hash(network_id) {
        // tx_hash is [u8; 32]
        let hash_hex = hex::encode(tx_hash);
    }
});
```

No manual `TransactionSignaturePayload` construction — unlike the JS SDK.

Network passphrases → SHA-256:

- Mainnet: `7ac33997544e3175d266bd022439b22cdb16508c01163f26e5cb2a3e1045a979`
- Testnet: `cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472`

## Transaction Fields

```rust
use stellar_xdr::curr::{TransactionEnvelope, FeeBumpTransactionInnerTx, MuxedAccount};

fn envelope_sender_and_ops(env: &TransactionEnvelope) -> (String, u64, &[Operation]) {
    match env {
        TransactionEnvelope::TxV0(v0) => (
            MuxedAccount::from(&v0.tx.source_account_ed25519).to_string(),
            v0.tx.fee as u64,
            &v0.tx.operations,
        ),
        TransactionEnvelope::Tx(v1) => (
            v1.tx.source_account.to_string(),
            v1.tx.fee as u64,
            &v1.tx.operations,
        ),
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            (inner.tx.source_account.to_string(), inner.tx.fee as u64, &inner.tx.operations)
        }
    }
}

// Result from tx_processing
let proc = &v.tx_processing[i];
let tx_hash_hex = hex::encode(proc.result.transaction_hash.0);
let fee_charged: i64 = proc.result.result.fee_charged;
let result_name: &str = proc.result.result.result.name();
```

## TransactionMeta Versions (CRITICAL)

Protocol 25 introduced **TransactionMetaV4**. Events relocated:

```rust
match &proc.tx_apply_processing {
    TransactionMeta::V3(v3) => {
        if let Some(ref soroban) = v3.soroban_meta {
            // Events here
            for ev in soroban.events.iter() { /* ... */ }
        }
    }
    TransactionMeta::V4(v4) => {
        // Events at top-level (with stage info)
        for te in v4.events.iter() { /* te.event, te.stage */ }
        // Per-operation events
        for op_meta in v4.operations.iter() {
            for ev in op_meta.events.iter() { /* ... */ }
        }
    }
    _ => {}
}
```

Exhaustive `match` prevents silent breakage on protocol upgrades.

## Operation Types (27 total)

```rust
use stellar_xdr::curr::OperationBody;

match &op.body {
    OperationBody::CreateAccount(args) => { /* ... */ }
    OperationBody::Payment(args) => { /* args.destination, args.amount, args.asset */ }
    OperationBody::InvokeHostFunction(args) => { /* Soroban contract call */ }
    OperationBody::ExtendFootprintTtl(args) => { /* TTL extension */ }
    OperationBody::RestoreFootprint(args) => { /* state restoration */ }
    // ... 22 more classic types — compiler enforces exhaustive handling
}
```

## Performance

| Metric                              | Rust (estimated) | Node.js (measured) |
| ----------------------------------- | ---------------- | ------------------ |
| Heavy ledger parse (343 txs, 2.4MB) | ~5-10ms          | 76ms               |
| Cold start (Lambda)                 | 100-300ms        | 500-1500ms         |
| Memory overhead                     | ~20MB            | ~150MB             |
| Binary size                         | ~10-15MB         | ~50MB+             |

Lambda budget: ~5000ms per ledger. Rust gives ~500x headroom.
