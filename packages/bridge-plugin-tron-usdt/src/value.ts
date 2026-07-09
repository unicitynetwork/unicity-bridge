import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';
import { Asset } from '@unicitylabs/state-transition-sdk/lib/payment/asset/Asset.js';
import { AssetId } from '@unicitylabs/state-transition-sdk/lib/payment/asset/AssetId.js';
import { PaymentAssetCollection } from '@unicitylabs/state-transition-sdk/lib/payment/asset/PaymentAssetCollection.js';

import { bytesEqual } from './hex.js';

/**
 * Reads the bridged-coin amount declared in a token's `data`, or null if the
 * token declares no value for `coinId`. Injected into the verifier so the
 * mint-reason check can confirm the token's declared value equals the locked
 * amount.
 *
 * Production bridge tokens use bare SDK `PaymentAssetCollection` CBOR in
 * `MintTransaction.data`. Sphere's internal `SpherePaymentData` envelope
 * (CBOR tag 39050) is wallet-to-wallet only and must not appear on bridged
 * tokens.
 */
export type BridgedAmountExtractor = (data: Uint8Array | null, coinId: Uint8Array) => bigint | null;

/** Production bridge value data: bare SDK `PaymentAssetCollection` CBOR. */
export function encodeBridgePaymentData(coinId: Uint8Array, amount: bigint): Uint8Array {
  return PaymentAssetCollection.create(new Asset(new AssetId(coinId), amount)).toCBOR();
}

/** Decode production bridge value data; returns null for SpherePaymentData(39050) or other non-bridge formats. */
export function decodeBridgePaymentData(data: Uint8Array | null, coinId: Uint8Array): bigint | null {
  if (!data) return null;
  try {
    return PaymentAssetCollection.fromCBOR(data).get(new AssetId(coinId))?.value ?? null;
  } catch {
    return null;
  }
}

/**
 * Minimal self-contained value envelope used by the CLI/tests:
 * `CBOR [ coinId: bstr, amount: uint ]`.
 *
 * @deprecated Use {@link encodeBridgePaymentData}; this simple envelope is not
 * the production bridge token format.
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
