import { describe, it, expect } from 'vitest';
import { xdr } from '@stellar/stellar-base';
import { decodeEventTopics, decodeContractEvent } from './event-decoder.js';

describe('decodeEventTopics', () => {
  it('decodes array of ScVal topics', () => {
    const topics = [xdr.ScVal.scvSymbol('transfer'), xdr.ScVal.scvU32(100)];
    const result = decodeEventTopics(topics);
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({ type: 'symbol', value: 'transfer' });
    expect(result[1]).toEqual({ type: 'u32', value: 100 });
  });

  it('handles empty topics', () => {
    expect(decodeEventTopics([])).toHaveLength(0);
  });
});

describe('decodeContractEvent', () => {
  it('decodes a contract event', () => {
    const event = new xdr.ContractEvent({
      ext: xdr.ExtensionPoint.fromXDR(Buffer.alloc(4, 0)),
      contractId: Buffer.alloc(32, 0xab) as unknown as xdr.Hash,
      type: xdr.ContractEventType.contract(),
      body: new xdr.ContractEventBody(
        0,
        new xdr.ContractEventV0({
          topics: [xdr.ScVal.scvSymbol('transfer')],
          data: xdr.ScVal.scvU32(42),
        })
      ),
    });

    const result = decodeContractEvent(event);
    expect(result.eventType).toBe('contract');
    expect(result.contractId).toMatch(/^C[A-Z0-9]+$/);
    expect(result.topics).toHaveLength(1);
    expect(result.topics[0]).toEqual({ type: 'symbol', value: 'transfer' });
    expect(result.data).toEqual({ type: 'u32', value: 42 });
  });

  it('handles null contractId', () => {
    const event = new xdr.ContractEvent({
      ext: xdr.ExtensionPoint.fromXDR(Buffer.alloc(4, 0)),
      contractId: null,
      type: xdr.ContractEventType.system(),
      body: new xdr.ContractEventBody(
        0,
        new xdr.ContractEventV0({
          topics: [],
          data: xdr.ScVal.scvVoid(),
        })
      ),
    });

    const result = decodeContractEvent(event);
    expect(result.contractId).toBeNull();
    expect(result.eventType).toBe('system');
  });
});
