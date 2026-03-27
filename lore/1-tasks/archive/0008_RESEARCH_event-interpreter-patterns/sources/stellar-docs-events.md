# Stellar Docs: Events Data Structures

**Source:** https://developers.stellar.org/docs/learn/fundamentals/stellar-data-structures/events
**Fetched:** 2026-03-27

---

Events are the mechanism that applications off-chain can use to monitor movement of value of any Stellar operation, as well as custom events in contracts on-chain.

## How are events emitted?

`ContractEvents` appear in Stellar Core's `TransactionMeta`. The placement depends on the `TransactionMeta` version. For Soroban transactions, `TransactionMetaV3` contains a `sorobanMeta` field with `SorobanTransactionMeta` that includes both `events` (custom contract events) and `diagnosticEvents`. Note that `events` only populate when transactions succeed.

`TransactionMetaV4` is more sophisticated, supporting events for Soroban, classic operations, fees, and refunds. The top-level `events` vector handles transaction-level events, currently containing `fee` events for initial charges and refunds. Operation-specific events appear under `OperationMetaV2`.

### ContractEvent

Event topics can contain mixed types. Events also include a data object supporting any value or type, including custom types defined by contracts using `#[contracttype]`:

```
struct ContractEvent
{
    ExtensionPoint ext;
    ContractID* contractID;
    ContractEventType type;

    union switch (int v)
    {
    case 0:
        struct
        {
            SCVal topics<>;
            SCVal data;
        } v0;
    }
    body;
};
```

### TransactionMetaV3

```
struct SorobanTransactionMeta
{
    SorobanTransactionMetaExt ext;

    ContractEvent events<>;
    SCVal returnValue;

    DiagnosticEvent diagnosticEvents<>;
};

struct TransactionMetaV3
{
    ExtensionPoint ext;

    LedgerEntryChanges txChangesBefore;
    OperationMeta operations<>;
    LedgerEntryChanges txChangesAfter;
    SorobanTransactionMeta* sorobanMeta;
};
```

### TransactionMetaV4

```
struct OperationMetaV2
{
    ExtensionPoint ext;
    LedgerEntryChanges changes;
    ContractEvent events<>;
};

enum TransactionEventStage {
    TRANSACTION_EVENT_STAGE_BEFORE_ALL_TXS = 0,
    TRANSACTION_EVENT_STAGE_AFTER_TX = 1,
    TRANSACTION_EVENT_STAGE_AFTER_ALL_TXS = 2
};

struct TransactionEvent {
    TransactionEventStage stage;
    ContractEvent event;
};

struct TransactionMetaV4
{
    ExtensionPoint ext;

    LedgerEntryChanges txChangesBefore;
    OperationMetaV2 operations<>;
    LedgerEntryChanges txChangesAfter;
    SorobanTransactionMetaV2* sorobanMeta;

    TransactionEvent events<>;
    DiagnosticEvent diagnosticEvents<>;
};
```

### Event types

Three `ContractEventType`s exist:

1. `CONTRACT` events are emitted by contracts that use the `contract_event` host function to convey state changes.
2. `SYSTEM` events are emitted by the host. Currently, only one system event exists: emitted when `update_current_contract_wasm` is called.
3. `DIAGNOSTIC` events are meant for debugging and will not be emitted unless the host instance explicitly enables it.

## What are diagnosticEvents?

The `diagnosticEvents` field remains empty by default unless the stellar-core instance has `ENABLE_SOROBAN_DIAGNOSTIC_EVENTS=true` configured. When enabled, this list includes failed contract call events, host errors, contract call stack traces, and logs from `log_from_linear_memory`. These events have `type == DIAGNOSTIC`. For Soroban invocations, the list also contains non-diagnostic contract events.

### fn_call

The `fn_call` diagnostic event fires when a contract is called and contains these topics:

1. The symbol `"fn_call"`
2. The contract ID being called
3. A symbol with the function name

Data includes the function arguments vector.

### fn_return

The `fn_return` diagnostic event fires when a contract call completes and contains these topics:

1. The symbol `"fn_return"`
2. A symbol with the function name

Data contains the returned value.

### When should diagnostic events be enabled?

Regular `ContractEvents` should convey information about state changes. `diagnosticEvents` on the other hand contain events that are not useful for most users, but may be helpful in debugging issues or building the contract call stack.

Due to the fact that a node with diagnostic events enabled will be executing code paths that diverge from a regular node, it is highly encouraged to only use this feature on watcher nodes (nodes where `NODE_IS_VALIDATOR=false` is set).

## Tracking the movement of value

Starting in protocol 23, classic operations can emit `transfer`, `mint`, `burn`, `clawback`, `fee`, and `set_authorized` events so that the movement of assets and trustline updates can be tracked using a single stream of data. These events require `EMIT_CLASSIC_EVENTS=true`. Setting `BACKFILL_STELLAR_ASSET_EVENTS=true` emits events for any ledger regardless of protocol version.

## Reading events

Use the `getEvents` RPC endpoint to fetch and filter events by type, contract, and topic.

Events are ephemeral: RPC providers typically only keep short chunks (less than a week) of history around.

The TypeScript SDK example demonstrates fetching `transfer` events from the XLM contract:

```javascript
import {
  humanizeEvents,
  nativeToScVal,
  scValToNative,
  Address,
  Networks,
  Asset,
  xdr,
} from '@stellar/stellar-sdk';
import { Server } from '@stellar/stellar-sdk/rpc';

const s = new Server('https://soroban-testnet.stellar.org');

async function main() {
  const response = await s.getLatestLedger();
  const xlmFilter = {
    type: 'contract',
    contractIds: [Asset.native().contractId(Networks.TESTNET)],
    topics: [
      [
        nativeToScVal('transfer', { type: 'symbol' }).toXDR('base64'),
        '*',
        '*',
        '*',
      ],
    ],
  };
  let page = await s.getEvents({
    startLedger: response.sequence - 120,
    filters: [xlmFilter],
    limit: 10,
  });

  while (true) {
    if (!page.events.length) {
      await new Promise((r) => setTimeout(r, 2000));
    } else {
      console.log(cereal(simpleEventLog(page.events)));
      console.log(cereal(fullEventLog(page.events)));
    }

    page = await s.getEvents({
      filters: [xlmFilter],
      cursor: page.cursor,
      limit: 10,
    });
  }
}

function simpleEventLog(events) {
  return events.map((event) => {
    return {
      topics: event.topic.map((t) => scValToNative(t)),
      value: scValToNative(event.value),
    };
  });
}

function fullEventLog(events) {
  return humanizeEvents(
    events.map((event) => {
      return new xdr.ContractEvent({
        contractId: event.contractId.address().toBuffer(),
        type: xdr.ContractEventType.contract(),
        body: new xdr.ContractEventBody(
          0,
          new xdr.ContractEventV0({
            topics: event.topic,
            data: event.value,
          })
        ),
      });
    })
  );
}

function cereal(data) {
  return JSON.stringify(
    data,
    (k, v) => (typeof v === 'bigint' ? v.toString() : v),
    2
  );
}

main().catch((e) => console.error(e));
```

The RPC API supports alternate XDR encoding formats like JSON for human-readable command-line event viewing by passing `xdrFormat: "json"` as an additional parameter.
