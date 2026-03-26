#!/usr/bin/env node

/**
 * Research script for task 0002: Explore LedgerCloseMeta XDR structure.
 *
 * Downloads a real LedgerCloseMetaBatch from Stellar public data lake,
 * decompresses zstd, parses with @stellar/stellar-sdk XDR types,
 * and prints the structure for field-by-field mapping.
 *
 * Usage: node tools/scripts/explore-xdr.mjs [ledger-sequence]
 * Default: uses a recent Soroban-era ledger
 */

import {
  xdr,
  hash,
  StrKey,
  Address,
  nativeToScVal,
} from '@stellar/stellar-sdk';
import { execSync } from 'node:child_process';
import { writeFileSync, readFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const CACHE_DIR = join(__dirname, '..', '..', '.temp', 'xdr-research');

// --- Helpers ---

function log(section, ...args) {
  console.log(`\n${'='.repeat(60)}`);
  console.log(`  ${section}`);
  console.log('='.repeat(60));
  args.forEach((a) => console.log(a));
}

function indent(obj, depth = 2) {
  return JSON.stringify(obj, null, depth);
}

function scValToReadable(scVal) {
  try {
    switch (scVal.switch().name) {
      case 'scvBool':
        return { type: 'bool', value: scVal.b() };
      case 'scvVoid':
        return { type: 'void' };
      case 'scvU32':
        return { type: 'u32', value: scVal.u32() };
      case 'scvI32':
        return { type: 'i32', value: scVal.i32() };
      case 'scvU64':
        return { type: 'u64', value: scVal.u64().toString() };
      case 'scvI64':
        return { type: 'i64', value: scVal.i64().toString() };
      case 'scvU128':
        return {
          type: 'u128',
          value: {
            hi: scVal.u128().hi().toString(),
            lo: scVal.u128().lo().toString(),
          },
        };
      case 'scvI128':
        return {
          type: 'i128',
          value: {
            hi: scVal.i128().hi().toString(),
            lo: scVal.i128().lo().toString(),
          },
        };
      case 'scvU256':
        return { type: 'u256', value: 'u256' };
      case 'scvI256':
        return { type: 'i256', value: 'i256' };
      case 'scvBytes':
        return { type: 'bytes', value: scVal.bytes().toString('hex') };
      case 'scvString':
        return { type: 'string', value: scVal.str().toString() };
      case 'scvSymbol':
        return { type: 'symbol', value: scVal.sym().toString() };
      case 'scvAddress': {
        const addr = scVal.address();
        if (addr.switch().name === 'scAddressTypeAccount') {
          return {
            type: 'address',
            subtype: 'account',
            value: StrKey.encodeEd25519PublicKey(addr.accountId().ed25519()),
          };
        }
        return {
          type: 'address',
          subtype: 'contract',
          value: StrKey.encodeContract(addr.contractId()),
        };
      }
      case 'scvVec':
        return { type: 'vec', value: scVal.vec().map(scValToReadable) };
      case 'scvMap':
        return {
          type: 'map',
          value: scVal.map().map((entry) => ({
            key: scValToReadable(entry.key()),
            val: scValToReadable(entry.val()),
          })),
        };
      default:
        return { type: scVal.switch().name, value: '<not decoded>' };
    }
  } catch (e) {
    return { type: 'error', message: e.message };
  }
}

// --- Download ---

async function downloadLedgerBatch(seq) {
  mkdirSync(CACHE_DIR, { recursive: true });
  const cached = join(CACHE_DIR, `ledger-${seq}.xdr`);

  if (existsSync(cached)) {
    console.log(`Using cached: ${cached}`);
    return readFileSync(cached);
  }

  // Stellar public data lake (SDF) — uses ledgers_per_file=1, files_per_partition=64000
  // Key format: {hex(0xFFFFFFFF - partition_start)}--{part_start}-{part_end}/{hex(0xFFFFFFFF - file_start)}--{file_start}.xdr.zst
  const partitionSize = 64000;
  const partStart = Math.floor(seq / partitionSize) * partitionSize;
  const partEnd = partStart + partitionSize - 1;
  const partHex = (0xffffffff - partStart).toString(16).padStart(8, '0');
  const fileHex = (0xffffffff - seq).toString(16).padStart(8, '0');
  const key = `${partHex}--${partStart}-${partEnd}/${fileHex}--${seq}.xdr.zst`;
  const url = `https://storage.googleapis.com/stellar-ledger-data-public/ledgers/${key}`;

  console.log(`Downloading: ${url}`);
  const zstFile = join(CACHE_DIR, `ledger-${seq}.xdr.zst`);

  execSync(`curl -sL -o "${zstFile}" "${url}"`);

  // Decompress zstd
  try {
    execSync(`zstd -d -f "${zstFile}" -o "${cached}"`);
  } catch {
    // Try with brew-installed zstd
    execSync(`/opt/homebrew/bin/zstd -d -f "${zstFile}" -o "${cached}"`);
  }

  return readFileSync(cached);
}

// --- Parse ---

function parseBatch(rawXdr) {
  // Try LedgerCloseMetaBatch first (Galexie output)
  try {
    const batch = xdr.LedgerCloseMetaBatch.fromXDR(rawXdr);
    console.log(
      `Parsed as LedgerCloseMetaBatch: ${
        batch.ledgerCloseMetas().length
      } ledger(s)`
    );
    console.log(`Start ledger: ${batch.startSequence()}`);
    console.log(`End ledger: ${batch.endSequence()}`);
    return batch.ledgerCloseMetas();
  } catch (e) {
    console.log(
      `Not a LedgerCloseMetaBatch (${e.message}), trying single LedgerCloseMeta...`
    );
  }

  // Fallback: single LedgerCloseMeta
  const meta = xdr.LedgerCloseMeta.fromXDR(rawXdr);
  console.log('Parsed as single LedgerCloseMeta');
  return [meta];
}

function exploreLedgerCloseMeta(lcm) {
  const version = lcm.switch().value;
  console.log(`LedgerCloseMeta version: v${version}`);

  // Get the versioned content
  let v;
  switch (version) {
    case 0:
      v = lcm.v0();
      break;
    case 1:
      v = lcm.v1();
      break;
    case 2:
      v = lcm.v2();
      break;
    default:
      console.log(`Unknown version: ${version}`);
      return;
  }

  // --- Ledger Header ---
  const ledgerHeader = v.ledgerHeader().header();
  log(
    'LEDGER HEADER',
    indent({
      sequence: ledgerHeader.ledgerSeq(),
      closeTime: ledgerHeader.scpValue().closeTime().toString(),
      protocolVersion: ledgerHeader.ledgerVersion(),
      baseFee: ledgerHeader.baseFee(),
      totalCoins: ledgerHeader.totalCoins().toString(),
      feePool: ledgerHeader.feePool().toString(),
      txSetResultHash: ledgerHeader.txSetResultHash().toString('hex'),
      previousLedgerHash: ledgerHeader.previousLedgerHash().toString('hex'),
      bucketListHash: ledgerHeader.bucketListHash().toString('hex'),
    })
  );

  // Ledger hash (hash of the header XDR)
  const headerXdr = v.ledgerHeader().toXDR();
  const ledgerHash = hash(headerXdr).toString('hex');
  console.log(`Ledger hash (SHA-256 of header XDR): ${ledgerHash}`);

  // --- Transactions ---
  let txProcessing;
  if (version === 0) {
    txProcessing = v.txProcessing();
  } else {
    txProcessing = v.txProcessing();
  }

  log('TRANSACTIONS', `Count: ${txProcessing.length}`);

  for (let txIdx = 0; txIdx < Math.min(txProcessing.length, 3); txIdx++) {
    const txp = txProcessing[txIdx];
    const result = txp.result();
    const txResultPair = result.result();
    const txHash = txResultPair.transactionHash().toString('hex');
    const txResult = txResultPair.result();

    console.log(`\n--- TX ${txIdx}: ${txHash.slice(0, 16)}... ---`);

    // Transaction envelope
    let envelope;
    if (version >= 1) {
      // v1/v2 have generalized tx set — envelope comes from txSet
      // But txProcessing still has the envelope info via different path
      // Let's check what's available
      const txApplyProcessing = txp;
      console.log(
        `  txApplyProcessing keys: ${Object.getOwnPropertyNames(
          Object.getPrototypeOf(txApplyProcessing)
        )
          .filter((k) => typeof txApplyProcessing[k] === 'function')
          .join(', ')}`
      );
    }

    // Result
    console.log(`  feeCharged: ${txResult.feeCharged().toString()}`);
    console.log(`  resultCode: ${txResult.result().switch().name}`);
    const successful =
      txResult.result().switch().name === 'txSuccess' ||
      txResult.result().switch().name === 'txFeeBumpInnerSuccess';
    console.log(`  successful: ${successful}`);

    // Operations from result
    if (successful) {
      try {
        const opResults = txResult.result().results();
        console.log(`  operations: ${opResults.length}`);
        for (let opIdx = 0; opIdx < Math.min(opResults.length, 3); opIdx++) {
          const opResult = opResults[opIdx];
          console.log(`    op[${opIdx}] type: ${opResult.tr().switch().name}`);
        }
      } catch (e) {
        console.log(`  operations: error reading - ${e.message}`);
      }
    }

    // Transaction meta (changes, events)
    const txMeta = txp.txApplyProcessing();
    const metaVersion = txMeta.switch().value;
    console.log(`  txMeta version: v${metaVersion}`);

    if (metaVersion === 3) {
      const v3 = txMeta.v3();

      // Soroban meta
      const sorobanMeta = v3.sorobanMeta();
      if (sorobanMeta) {
        console.log(`  SOROBAN META PRESENT`);

        // Events
        const events = sorobanMeta.events();
        console.log(`  Soroban events: ${events.length}`);
        for (let evIdx = 0; evIdx < Math.min(events.length, 3); evIdx++) {
          const ev = events[evIdx];
          console.log(`    event[${evIdx}]:`);
          console.log(`      type: ${ev.type().name}`);
          console.log(
            `      contractId: ${
              ev.contractId() ? StrKey.encodeContract(ev.contractId()) : 'null'
            }`
          );
          console.log(`      topics: ${ev.body().v0().topics().length}`);
          for (const topic of ev.body().v0().topics()) {
            console.log(`        topic: ${indent(scValToReadable(topic))}`);
          }
          console.log(
            `      data: ${indent(scValToReadable(ev.body().v0().data()))}`
          );
        }

        // Return value
        const returnVal = sorobanMeta.returnValue();
        if (returnVal) {
          console.log(`  returnValue: ${indent(scValToReadable(returnVal))}`);
        }

        // Diagnostic events
        const diagEvents = sorobanMeta.diagnosticEvents();
        console.log(`  diagnosticEvents: ${diagEvents.length}`);
      }

      // Ledger entry changes
      const operations = v3.operations();
      console.log(`  operationMetas: ${operations.length}`);
      for (let opIdx = 0; opIdx < Math.min(operations.length, 2); opIdx++) {
        const opMeta = operations[opIdx];
        const changes = opMeta.changes();
        console.log(`    op[${opIdx}] changes: ${changes.length}`);
        for (let chIdx = 0; chIdx < Math.min(changes.length, 3); chIdx++) {
          const change = changes[chIdx];
          console.log(`      change[${chIdx}] type: ${change.switch().name}`);
          try {
            let entry;
            switch (change.switch().name) {
              case 'ledgerEntryCreated':
                entry = change.created().data();
                break;
              case 'ledgerEntryUpdated':
                entry = change.updated().data();
                break;
              case 'ledgerEntryRemoved':
                entry = change.removed();
                break;
              case 'ledgerEntryState':
                entry = change.state().data();
                break;
            }
            if (entry && entry.switch) {
              console.log(`        entryType: ${entry.switch().name}`);
            }
          } catch (e) {
            console.log(`        error: ${e.message}`);
          }
        }
      }
    }
  }

  // --- Transaction envelopes (from tx set) ---
  if (version >= 1) {
    log('TX SET (envelopes)');
    try {
      // v1/v2 have generalized tx set
      const txSet = version === 2 ? v.txSet() : v.txSet();
      const generalized = txSet.v1TxSet();
      const phases = generalized.phases();
      console.log(`  phases: ${phases.length}`);
      let envIdx = 0;
      for (let phIdx = 0; phIdx < phases.length; phIdx++) {
        const phase = phases[phIdx];
        // v0 components
        const components = phase.v0Components();
        for (const comp of components) {
          const txs = comp.txsMaybeDiscountedFee().txs();
          for (const env of txs) {
            if (envIdx < 3) {
              const envType = env.switch().name;
              console.log(`  env[${envIdx}] type: ${envType}`);

              // Get source account
              let sourceAccount;
              if (envType === 'envelopeTypeTx') {
                sourceAccount = env.v1().tx().sourceAccount();
              } else if (envType === 'envelopeTypeTxV0') {
                sourceAccount = env.v0().tx().sourceAccountEd25519();
              } else if (envType === 'envelopeTypeTxFeeBump') {
                sourceAccount = env
                  .feeBump()
                  .tx()
                  .innerTx()
                  .v1()
                  .tx()
                  .sourceAccount();
              }

              if (sourceAccount && sourceAccount.switch) {
                const key =
                  sourceAccount.switch().name === 'publicKeyTypeEd25519'
                    ? StrKey.encodeEd25519PublicKey(sourceAccount.ed25519())
                    : sourceAccount.med25519
                    ? 'muxed'
                    : 'unknown';
                console.log(`    source: ${key}`);
              }

              // Get operations
              let ops;
              if (envType === 'envelopeTypeTx') {
                ops = env.v1().tx().operations();
              } else if (envType === 'envelopeTypeTxV0') {
                ops = env.v0().tx().operations();
              }

              if (ops) {
                console.log(`    operations: ${ops.length}`);
                for (let i = 0; i < Math.min(ops.length, 2); i++) {
                  console.log(
                    `      op[${i}] type: ${ops[i].body().switch().name}`
                  );

                  // INVOKE_HOST_FUNCTION details
                  if (ops[i].body().switch().name === 'invokeHostFunction') {
                    const ihf = ops[i].body().invokeHostFunctionOp();
                    const hostFn = ihf.hostFunction();
                    console.log(
                      `        hostFunction type: ${hostFn.switch().name}`
                    );

                    if (
                      hostFn.switch().name === 'hostFunctionTypeInvokeContract'
                    ) {
                      const invokeArgs = hostFn.invokeContract();
                      console.log(
                        `        contractAddress: ${indent(
                          scValToReadable(
                            xdr.ScVal.scvAddress(invokeArgs.contractAddress())
                          )
                        )}`
                      );
                      console.log(
                        `        functionName: ${invokeArgs
                          .functionName()
                          .toString()}`
                      );
                      console.log(`        args: ${invokeArgs.args().length}`);
                      for (const arg of invokeArgs.args().slice(0, 3)) {
                        console.log(
                          `          arg: ${indent(scValToReadable(arg))}`
                        );
                      }
                    }
                  }
                }
              }

              // Compute tx hash
              const envelopeXdrBytes = env.toXDR();
              // Hash is computed over the "signature base" not raw envelope
              // For v1 tx: hash(networkId + ENVELOPE_TYPE_TX + tx)
              console.log(
                `    envelopeXdr length: ${envelopeXdrBytes.length} bytes`
              );
            }
            envIdx++;
          }
        }
      }
      console.log(`  total envelopes: ${envIdx}`);
    } catch (e) {
      console.log(`  Error reading tx set: ${e.message}`);
    }
  }
}

// --- Main ---

const targetLedger = parseInt(process.argv[2]) || 54000000; // Recent Soroban-era ledger
console.log(`Target ledger: ${targetLedger}`);

try {
  const rawXdr = await downloadLedgerBatch(targetLedger);
  console.log(`Raw XDR size: ${rawXdr.length} bytes`);

  const metas = parseBatch(rawXdr);
  for (const lcm of metas) {
    exploreLedgerCloseMeta(lcm);
  }
} catch (e) {
  console.error(`Error: ${e.message}`);
  console.error(e.stack);
}
