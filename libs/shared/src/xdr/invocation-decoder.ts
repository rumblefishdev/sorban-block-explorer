import { xdr, Address } from '@stellar/stellar-base';
import { decodeScVal } from './scval-decoder.js';
import type { DecodedScVal } from './scval-decoder.js';

export interface InvocationNode {
  type: 'contract' | 'wasm' | 'sac';
  contractId: string | null;
  functionName: string | null;
  args: readonly DecodedScVal[];
  subInvocations: readonly InvocationNode[];
}

/**
 * Decode a SorobanAuthorizedInvocation into a tree of InvocationNodes.
 */
function decodeAuthorizedInvocation(
  invocation: xdr.SorobanAuthorizedInvocation
): InvocationNode {
  const fn = invocation.function();
  const subs = invocation.subInvocations().map(decodeAuthorizedInvocation);

  const fnType = fn.switch();
  if (
    fnType.value ===
    xdr.SorobanAuthorizedFunctionType.sorobanAuthorizedFunctionTypeContractFn()
      .value
  ) {
    const contractFn = fn.contractFn();
    const contractId = Address.fromScAddress(
      contractFn.contractAddress()
    ).toString();
    const functionName =
      typeof contractFn.functionName() === 'string'
        ? (contractFn.functionName() as string)
        : (contractFn.functionName() as Buffer).toString('utf8');
    const args = contractFn.args().map(decodeScVal);

    return {
      type: 'contract',
      contractId,
      functionName,
      args,
      subInvocations: subs,
    };
  }

  // CreateContractHostFn or other types
  return {
    type: 'wasm',
    contractId: null,
    functionName: null,
    args: [],
    subInvocations: subs,
  };
}

/**
 * Decode invocation tree from the transaction envelope's InvokeHostFunction operation.
 * Extracts the auth invocation hierarchy from SorobanAuthorizationEntry entries.
 */
export function decodeInvocationTree(envelopeXdr: string): InvocationNode[] {
  const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');

  let ops: xdr.Operation[];
  const type = envelope.switch();
  if (type.value === xdr.EnvelopeType.envelopeTypeTxV0().value) {
    ops = envelope.v0().tx().operations();
  } else if (type.value === xdr.EnvelopeType.envelopeTypeTx().value) {
    ops = envelope.v1().tx().operations();
  } else if (type.value === xdr.EnvelopeType.envelopeTypeTxFeeBump().value) {
    ops = envelope.feeBump().tx().innerTx().v1().tx().operations();
  } else {
    return [];
  }

  const nodes: InvocationNode[] = [];

  for (const op of ops) {
    const body = op.body();
    if (body.switch().value !== xdr.OperationType.invokeHostFunction().value) {
      continue;
    }

    const hostFn = body.invokeHostFunctionOp();
    const auth = hostFn.auth();

    for (const entry of auth) {
      nodes.push(decodeAuthorizedInvocation(entry.rootInvocation()));
    }
  }

  return nodes;
}
