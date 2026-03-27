import { xdr, StrKey } from '@stellar/stellar-base';
import { decodeScVal } from './scval-decoder.js';
import type { DecodedScVal } from './scval-decoder.js';

/**
 * Decode ScVal[] topics array from a Soroban event.
 */
export function decodeEventTopics(
  topics: xdr.ScVal[]
): readonly DecodedScVal[] {
  return topics.map(decodeScVal);
}

export type DecodedEventType = 'contract' | 'system' | 'diagnostic';

export interface DecodedContractEvent {
  contractId: string | null;
  eventType: DecodedEventType;
  topics: readonly DecodedScVal[];
  data: DecodedScVal;
}

/**
 * Decode a full ContractEvent into structured form.
 */
export function decodeContractEvent(
  event: xdr.ContractEvent
): DecodedContractEvent {
  const contractIdRaw = event.contractId();
  const contractId = contractIdRaw
    ? StrKey.encodeContract(Buffer.from(contractIdRaw as unknown as Buffer))
    : null;

  const eventTypeVal = event.type().value;
  let eventType: DecodedEventType;
  if (eventTypeVal === xdr.ContractEventType.contract().value) {
    eventType = 'contract';
  } else if (eventTypeVal === xdr.ContractEventType.system().value) {
    eventType = 'system';
  } else {
    eventType = 'diagnostic';
  }

  const body = event.body();
  const bodyV0 = body.v0();
  const topics = decodeEventTopics(bodyV0.topics());
  const data = decodeScVal(bodyV0.data());

  return { contractId, eventType, topics, data };
}
