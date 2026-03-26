---
title: 'Error handling strategy and Lambda performance estimate (Rust)'
type: research
status: mature
spawned_from: null
spawns: []
tags: [rust, error-handling, performance, lambda]
links: []
history:
  - date: 2026-03-26
    status: mature
    who: stkrolikiewicz
    note: 'Rewritten as Rust-first with Result<T, Error> patterns'
---

# Error handling strategy and Lambda performance estimate (Rust)

## XDR Parse Errors

### Rust: `Result<T, stellar_xdr::curr::Error>`

All `from_xdr()` methods return `Result` — no exceptions, compiler-enforced.

```rust
use stellar_xdr::curr::{LedgerCloseMetaBatch, Limits, ReadXdr};

let limits = Limits { depth: 1000, len: data.len().max(10_000_000) };

match LedgerCloseMetaBatch::from_xdr(data.as_slice(), limits) {
    Ok(batch) => {
        for meta in batch.ledger_close_metas.iter() {
            process_ledger(meta)?;
        }
    }
    Err(e) => {
        // e is stellar_xdr::curr::Error
        // Variants: Invalid, Io, DepthLimitExceeded, LengthLimitExceeded, etc.
        log::error!("XDR decode error: {}", e);
        store_raw_xdr_with_error(data, &e.to_string())?;
    }
}
```

### Per-Transaction Error Handling

```rust
fn process_transaction(
    env: &TransactionEnvelope,
    proc: &TransactionResultMeta,
    ledger_seq: u32,
    limits: &Limits,
) -> Result<TransactionRecord, ProcessError> {
    let tx_hash = hex::encode(proc.result.transaction_hash.0);
    let fee_charged = proc.result.result.fee_charged;

    // These fields are always extractable (outer wrapper):
    let envelope_xdr = base64_encode(env.to_xdr(limits.clone())?);
    let result_xdr = base64_encode(proc.result.result.to_xdr(limits.clone())?);
    let result_meta_xdr = base64_encode(proc.tx_apply_processing.to_xdr(limits.clone())?);

    // Inner parsing may fail:
    let (source_account, operations, invocation_tree) = match parse_envelope_details(env) {
        Ok(details) => details,
        Err(e) => {
            // Store raw XDR, mark parse_error, keep visible
            return Ok(TransactionRecord {
                hash: tx_hash,
                ledger_sequence: ledger_seq,
                fee_charged,
                successful: false,
                envelope_xdr,
                result_xdr,
                result_meta_xdr,
                parse_error: Some(e.to_string()),
                ..Default::default()
            });
        }
    };

    Ok(TransactionRecord {
        hash: tx_hash,
        ledger_sequence: ledger_seq,
        source_account,
        fee_charged,
        successful: is_successful(&proc.result.result.result),
        result_code: proc.result.result.result.name().to_string(),
        envelope_xdr,
        result_xdr,
        result_meta_xdr,
        operation_tree: invocation_tree,
        parse_error: None,
        operations,
    })
}
```

**Key principle:** `tx_hash`, `fee_charged`, and raw XDR are always extractable from the outer wrapper even if inner parsing fails. The `result.transaction_hash` is in `TransactionResultMeta`, not inside the parsed content.

## Unknown Operation Types

Rust `match` on `OperationBody` is exhaustive. Unknown types can't silently pass:

```rust
match &op.body {
    OperationBody::CreateAccount(args) => parse_create_account(args),
    OperationBody::Payment(args) => parse_payment(args),
    OperationBody::InvokeHostFunction(args) => parse_invoke(args),
    // ... all 27 types
}
```

If a new protocol adds an operation type and we update `stellar-xdr` crate, the compiler will flag every `match` that doesn't handle the new variant. **No runtime "unknown operation" can occur** — it's a compile-time guarantee.

For forward compatibility without updating the crate, use a catch-all:

```rust
_ => {
    log::warn!("Unhandled operation type: {}", op.body.name());
    json!({ "type": "unhandled", "name": op.body.name(), "raw_xdr": base64_encode(op.to_xdr(limits)?) })
}
```

This should emit a CloudWatch alarm (see task 0014 AC) to trigger SDK update.

## Protocol Upgrade Handling

### Crate Version Compatibility

| Crate Version     | XDR Protocol | Notes                              |
| ----------------- | ------------ | ---------------------------------- |
| `stellar-xdr` v25 | Protocol 25  | Current mainnet, TransactionMetaV4 |
| `stellar-xdr` v23 | Protocol 23  | TransactionMetaV4 introduced       |

The crate version tracks the XDR schema version. Protocol upgrades require:

1. Stellar announces upgrade (weeks in advance)
2. Update `stellar-xdr` crate version in `Cargo.toml`
3. `cargo build` — compiler flags any new enum variants not handled
4. Add `match` arms for new types
5. Run integration tests with pre/post-upgrade XDR fixtures
6. Deploy via normal CI/CD pipeline

### Version Dispatch is Compile-Time Safe

```rust
// TransactionMeta::V3 vs V4 — Rust match is exhaustive
match meta {
    TransactionMeta::V3(v3) => { /* events in soroban_meta */ }
    TransactionMeta::V4(v4) => { /* events at top-level */ }
    _ => { /* V0/V1/V2 */ }
}
// If V5 is added in a new crate version → compile error here
```

Contrast with JS SDK: `meta.switch()` returns raw `4` with `undefined` name. A new version would silently fall through without warning.

## Raw Payload Retention Strategy

| Column            | Format      | Rust Encoding                                                           |
| ----------------- | ----------- | ----------------------------------------------------------------------- |
| `envelope_xdr`    | base64 text | `base64::engine::general_purpose::STANDARD.encode(env.to_xdr(limits)?)` |
| `result_xdr`      | base64 text | same pattern                                                            |
| `result_meta_xdr` | base64 text | same pattern                                                            |

**Why base64:** PostgreSQL `text` type with base64:

- Direct use in Horizon-compatible API responses
- Browser JS SDK accepts base64 for on-demand decode
- No encoding layer needed in the NestJS API

Size estimates from mainnet:

- Average envelope XDR: ~200-500 bytes
- Average result XDR: ~50-200 bytes
- Average result_meta_xdr: ~500-5000 bytes (Soroban state changes can be 50KB+)

## Performance Estimates

### Parse Times

| Metric                        | Rust (estimated) | Node.js (measured) |
| ----------------------------- | ---------------- | ------------------ |
| Heavy ledger (343 txs, 2.4MB) | ~5-10ms          | 76ms               |
| Light ledger (228 txs, 1.0MB) | ~2-5ms           | 27ms               |
| Per TX average                | ~0.02ms          | 0.22ms             |

Rust estimate based on typical XDR deserialization benchmarks for native binaries. Actual `stellar-indexer` benchmarks not yet measured — recommended before production deployment.

### Lambda Execution Budget

- Ledger close interval: ~5 seconds
- Rust parse + JSON conversion: ~10-20ms
- DB writes (batch insert via sqlx): ~100-300ms
- S3 download + zstd decompress: ~50-100ms
- **Total per ledger: ~200-400ms**
- **Headroom: 5000ms / 400ms = ~12x margin**

### Lambda Deployment

```
Target: provided.al2023 (custom runtime)
Binary: static Rust binary compiled with musl or glibc
Size: ~10-15MB (single zip file)
Memory: 256-512MB sufficient
Cold start: ~100-300ms (no runtime initialization)
Provisioned concurrency: NOT needed (unlike Node.js)
```

### DB Driver

Recommended: [`sqlx`](https://github.com/launchbadge/sqlx) with PostgreSQL

- Async, compile-time checked queries
- Connection pooling built-in (important for Lambda)
- `ON CONFLICT DO NOTHING` for idempotent writes

Alternative: [`diesel`](https://diesel.rs/) — synchronous, stronger type safety on schema, heavier compilation.

### Bottleneck Analysis

1. **DB writes** (~200ms) — the actual bottleneck. Batch inserts with `sqlx` are critical.
2. **S3 download** (~50-100ms) — pre-fetched by Lambda runtime for S3-triggered events.
3. **Zstd decompress** (~5-10ms) — negligible with `zstd` crate.
4. **XDR parse** (~5-10ms) — negligible in Rust.
5. **JSON serialization** (~5-10ms) — for JSONB columns (ScVal, operation details).
