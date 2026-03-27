import { describe, it, expect } from 'vitest';
import {
  xdr,
  Keypair,
  Networks,
  TransactionBuilder,
  Account,
  Operation,
  Address,
  Asset,
  nativeToScVal,
} from '@stellar/stellar-base';
import { decodeInvocationTree } from './invocation-decoder.js';

// The SDK types use Opaque[] for Hash/ContractId but runtime is Buffer.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const asHash = (buf: Buffer): any => buf;

function buildInvokeHostFunctionEnvelope(
  auth: xdr.SorobanAuthorizationEntry[]
): string {
  const keypair = Keypair.random();
  const account = new Account(keypair.publicKey(), '100');
  const contractAddress = xdr.ScAddress.scAddressTypeContract(
    asHash(Buffer.alloc(32, 0x01))
  );

  const tx = new TransactionBuilder(account, {
    fee: '100',
    networkPassphrase: Networks.TESTNET,
  })
    .addOperation(
      Operation.invokeHostFunction({
        func: xdr.HostFunction.hostFunctionTypeInvokeContract(
          new xdr.InvokeContractArgs({
            contractAddress,
            functionName: 'test_fn',
            args: [],
          })
        ),
        auth,
      })
    )
    .setTimeout(30)
    .build();

  return tx.toEnvelope().toXDR('base64');
}

describe('decodeInvocationTree', () => {
  it('returns empty for non-invokeHostFunction transactions', () => {
    const keypair = Keypair.random();
    const account = new Account(keypair.publicKey(), '100');
    const tx = new TransactionBuilder(account, {
      fee: '100',
      networkPassphrase: Networks.TESTNET,
    })
      .addOperation(
        Operation.payment({
          destination: Keypair.random().publicKey(),
          asset: Asset.native(),
          amount: '10',
        })
      )
      .setTimeout(30)
      .build();

    const result = decodeInvocationTree(tx.toEnvelope().toXDR('base64'));
    expect(result).toHaveLength(0);
  });

  it('returns empty for invokeHostFunction with no auth', () => {
    const envelope = buildInvokeHostFunctionEnvelope([]);
    const result = decodeInvocationTree(envelope);
    expect(result).toHaveLength(0);
  });

  it('decodes a simple contract invocation from auth entries', () => {
    const contractBuf = Buffer.alloc(32, 0x02);
    const contractAddress = xdr.ScAddress.scAddressTypeContract(
      asHash(contractBuf)
    );

    const authEntry = new xdr.SorobanAuthorizationEntry({
      credentials: xdr.SorobanCredentials.sorobanCredentialsSourceAccount(),
      rootInvocation: new xdr.SorobanAuthorizedInvocation({
        function:
          xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
            new xdr.InvokeContractArgs({
              contractAddress,
              functionName: 'transfer',
              args: [nativeToScVal(42, { type: 'u32' })],
            })
          ),
        subInvocations: [],
      }),
    });

    const envelope = buildInvokeHostFunctionEnvelope([authEntry]);
    const result = decodeInvocationTree(envelope);

    expect(result).toHaveLength(1);
    expect(result[0]?.type).toBe('contract');
    expect(result[0]?.contractId).toBe(
      Address.contract(contractBuf).toString()
    );
    expect(result[0]?.functionName).toBe('transfer');
    expect(result[0]?.args).toHaveLength(1);
    expect(result[0]?.args[0]).toEqual({ type: 'u32', value: 42 });
    expect(result[0]?.subInvocations).toHaveLength(0);
  });

  it('decodes nested sub-invocations', () => {
    const outerBuf = Buffer.alloc(32, 0x03);
    const innerBuf = Buffer.alloc(32, 0x04);

    const authEntry = new xdr.SorobanAuthorizationEntry({
      credentials: xdr.SorobanCredentials.sorobanCredentialsSourceAccount(),
      rootInvocation: new xdr.SorobanAuthorizedInvocation({
        function:
          xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
            new xdr.InvokeContractArgs({
              contractAddress: xdr.ScAddress.scAddressTypeContract(
                asHash(outerBuf)
              ),
              functionName: 'swap',
              args: [],
            })
          ),
        subInvocations: [
          new xdr.SorobanAuthorizedInvocation({
            function:
              xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
                new xdr.InvokeContractArgs({
                  contractAddress: xdr.ScAddress.scAddressTypeContract(
                    asHash(innerBuf)
                  ),
                  functionName: 'transfer',
                  args: [],
                })
              ),
            subInvocations: [],
          }),
        ],
      }),
    });

    const envelope = buildInvokeHostFunctionEnvelope([authEntry]);
    const result = decodeInvocationTree(envelope);

    expect(result).toHaveLength(1);
    expect(result[0]?.functionName).toBe('swap');
    expect(result[0]?.subInvocations).toHaveLength(1);
    expect(result[0]?.subInvocations[0]?.functionName).toBe('transfer');
    expect(result[0]?.subInvocations[0]?.contractId).toBe(
      Address.contract(innerBuf).toString()
    );
  });
});
