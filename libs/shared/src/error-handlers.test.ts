import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  handleXdrParseError,
  handleUnknownOperation,
  handleScValDecodeError,
  handleContractMetadataError,
} from './error-handlers.js';

beforeEach(() => {
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

function parseLog(): Record<string, unknown> {
  const calls = vi.mocked(console.error).mock.calls;
  return JSON.parse((calls[0] as string[])[0] as string) as Record<
    string,
    unknown
  >;
}

describe('handleXdrParseError', () => {
  it('returns partial record with parseError flag and preserves raw XDR', () => {
    const result = handleXdrParseError(new Error('Invalid XDR'), {
      transactionHash: 'abc123',
      ledgerSequence: 100,
      decodeStep: 'fromXDR',
      rawXdr: 'AAAA==',
    });

    expect(result.parseError).toBe(true);
    expect(result.error.errorType).toBe('XdrParseError');
    expect(result.error.message).toBe('Invalid XDR');
    expect(result.error.rawXdr).toBe('AAAA==');
    expect(result.error.decodeStep).toBe('fromXDR');
    expect(result.error.context.transactionHash).toBe('abc123');
    expect(result.error.context.ledgerSequence).toBe(100);
    expect(result.error.timestamp).toBeTruthy();
  });

  it('captures error stack trace', () => {
    const result = handleXdrParseError(new Error('fail'), {
      decodeStep: 'fromXDR',
      rawXdr: 'raw',
    });

    expect(result.error.stack).toContain('Error: fail');
  });

  it('emits structured JSON log', () => {
    handleXdrParseError(new Error('bad'), {
      decodeStep: 'fromXDR',
      rawXdr: 'AAAA==',
      transactionHash: 'tx1',
    });

    const log = parseLog();
    expect(log['level']).toBe('error');
    expect(log['event']).toBe('XdrParseError');
    expect(log['transactionHash']).toBe('tx1');
    expect(log['decodeStep']).toBe('fromXDR');
    expect(log['rawXdr']).toBeUndefined();
  });

  it('does not throw', () => {
    expect(() =>
      handleXdrParseError(new Error('fail'), {
        decodeStep: 'step',
        rawXdr: 'raw',
      })
    ).not.toThrow();
  });
});

describe('handleUnknownOperation', () => {
  it('returns unknown operation result with raw XDR preserved', () => {
    const result = handleUnknownOperation(999, 'BBBB==', {
      transactionHash: 'tx1',
      ledgerSequence: 200,
    });

    expect(result.operationType).toBe('unknown');
    expect(result.rawXdr).toBe('BBBB==');
    expect(result.error.errorType).toBe('UnknownOperationType');
    expect(result.error.operationType).toBe(999);
    expect(result.error.message).toContain('999');
  });

  it('emits structured JSON log suitable for CloudWatch alarm', () => {
    handleUnknownOperation(42, 'CCCC==', { ledgerSequence: 5 });

    const log = parseLog();
    expect(log['level']).toBe('error');
    expect(log['event']).toBe('UnknownOperationType');
    expect(log['operationType']).toBe(42);
    expect(log['ledgerSequence']).toBe(5);
  });

  it('does not throw', () => {
    expect(() => handleUnknownOperation(0, '', {})).not.toThrow();
  });
});

describe('handleScValDecodeError', () => {
  it('returns unparsed result preserving raw value and field context', () => {
    const result = handleScValDecodeError(
      new Error('bad scval'),
      'functionArgs',
      'raw-bytes-hex',
      { transactionHash: 'tx2', parentId: 'inv-1' }
    );

    expect(result.unparsed).toBe(true);
    expect(result.rawValue).toBe('raw-bytes-hex');
    expect(result.error.errorType).toBe('ScValDecodeError');
    expect(result.error.fieldContext).toBe('functionArgs');
    expect(result.error.parentId).toBe('inv-1');
  });

  it('captures error stack trace', () => {
    const result = handleScValDecodeError(new Error('bad'), 'topics', 'raw', {
      parentId: 'x',
    });

    expect(result.error.stack).toContain('Error: bad');
  });

  it('emits structured JSON log', () => {
    handleScValDecodeError(new Error('bad'), 'topics', 'raw', {
      contractId: 'C1',
      parentId: 'inv-1',
    });

    const log = parseLog();
    expect(log['level']).toBe('error');
    expect(log['event']).toBe('ScValDecodeError');
    expect(log['contractId']).toBe('C1');
    expect(log['parentId']).toBe('inv-1');
    expect(log['rawValue']).toBeUndefined();
  });

  it('does not throw', () => {
    expect(() =>
      handleScValDecodeError(new Error('fail'), 'topics', 'raw', {
        parentId: 'x',
      })
    ).not.toThrow();
  });
});

describe('handleContractMetadataError', () => {
  it('returns metadataMissing result preserving contract ID and WASM hash', () => {
    const result = handleContractMetadataError(new Error('wasm fail'), {
      contractId: 'C_ABC',
      wasmHash: 'hash123',
      extractionStep: 'interface_parse',
    });

    expect(result.metadataMissing).toBe(true);
    expect(result.error.errorType).toBe('ContractMetadataError');
    expect(result.error.contractId).toBe('C_ABC');
    expect(result.error.wasmHash).toBe('hash123');
    expect(result.error.extractionStep).toBe('interface_parse');
  });

  it('captures error stack trace', () => {
    const result = handleContractMetadataError(new Error('wasm'), {
      contractId: 'C1',
      wasmHash: 'h',
      extractionStep: 'step',
    });

    expect(result.error.stack).toContain('Error: wasm');
  });

  it('emits structured JSON log', () => {
    handleContractMetadataError(new Error('fail'), {
      contractId: 'C_X',
      wasmHash: 'h',
      extractionStep: 'step',
    });

    const log = parseLog();
    expect(log['level']).toBe('error');
    expect(log['event']).toBe('ContractMetadataError');
    expect(log['contractId']).toBe('C_X');
  });

  it('does not throw', () => {
    expect(() =>
      handleContractMetadataError(new Error('fail'), {
        contractId: 'C_X',
        wasmHash: 'h',
        extractionStep: 'step',
      })
    ).not.toThrow();
  });
});
