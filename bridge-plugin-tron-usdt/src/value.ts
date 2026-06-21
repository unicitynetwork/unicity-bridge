import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';

import { bytesEqual } from './hex.js';

/**
 * Reads the bridged-coin amount declared in a token's `data`, or null if the
 * token declares no value for `coinId`. Injected into the verifier so the
 * mint-reason check can confirm the token's declared value equals the locked
 * amount.
 *
 * In sphere-sdk this is backed by `decodeSpherePaymentData`; standalone/CLI use
 * {@link decodeBridgedValue} over the simple envelope below.
 */
export type BridgedAmountExtractor = (data: Uint8Array | null, coinId: Uint8Array) => bigint | null;

/**
 * Minimal self-contained value envelope used by the CLI/tests:
 * `CBOR [ coinId: bstr, amount: uint ]`.
 */
export function encodeBridgedValue(coinId: Uint8Array, amount: bigint): Uint8Array {
  return CborSerializer.encodeArray(
    CborSerializer.encodeByteString(coinId),
    CborSerializer.encodeUnsignedInteger(amount),
  );
}

export function decodeBridgedValue(data: Uint8Array | null, coinId: Uint8Array): bigint | null {
  if (!data) {
    return null;
  }
  let items: Uint8Array[];
  try {
    items = CborDeserializer.decodeArray(data, 2);
  } catch {
    return null;
  }
  const encodedCoinId = CborDeserializer.decodeByteString(items[0]);
  if (!bytesEqual(encodedCoinId, coinId)) {
    return null;
  }
  return CborDeserializer.decodeUnsignedInteger(items[1]);
}
