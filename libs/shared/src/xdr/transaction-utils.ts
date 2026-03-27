import { xdr, Transaction, FeeBumpTransaction } from '@stellar/stellar-base';

/**
 * Compute the canonical transaction hash from an envelope XDR string.
 * Returns a 64-character lowercase hex string.
 */
export function computeTransactionHash(
  envelopeXdr: string,
  networkPassphrase: string
): string {
  const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
  const type = envelope.switch();

  if (type.value === xdr.EnvelopeType.envelopeTypeTxFeeBump().value) {
    const tx = new FeeBumpTransaction(envelope, networkPassphrase);
    return tx.hash().toString('hex');
  }

  const tx = new Transaction(envelope, networkPassphrase);
  return tx.hash().toString('hex');
}

export interface ExtractedMemo {
  memoType: string;
  memo: string | null;
}

/**
 * Extract memo type and value from a TransactionEnvelope.
 */
export function extractMemo(envelope: xdr.TransactionEnvelope): ExtractedMemo {
  let memo: xdr.Memo;
  const type = envelope.switch();

  if (type.value === xdr.EnvelopeType.envelopeTypeTxV0().value) {
    memo = envelope.v0().tx().memo();
  } else if (type.value === xdr.EnvelopeType.envelopeTypeTx().value) {
    memo = envelope.v1().tx().memo();
  } else if (type.value === xdr.EnvelopeType.envelopeTypeTxFeeBump().value) {
    const inner = envelope.feeBump().tx().innerTx().v1().tx();
    memo = inner.memo();
  } else {
    return { memoType: 'none', memo: null };
  }

  return decodeMemo(memo);
}

function decodeMemo(memo: xdr.Memo): ExtractedMemo {
  const type = memo.switch();

  switch (type.value) {
    case xdr.MemoType.memoNone().value:
      return { memoType: 'none', memo: null };

    case xdr.MemoType.memoText().value: {
      const text = memo.text();
      const value = typeof text === 'string' ? text : text.toString('utf8');
      return { memoType: 'text', memo: value };
    }

    case xdr.MemoType.memoId().value:
      return { memoType: 'id', memo: memo.id().toString() };

    case xdr.MemoType.memoHash().value:
      return { memoType: 'hash', memo: memo.hash().toString('hex') };

    case xdr.MemoType.memoReturn().value:
      return { memoType: 'return', memo: memo.retHash().toString('hex') };

    default:
      return { memoType: 'none', memo: null };
  }
}
