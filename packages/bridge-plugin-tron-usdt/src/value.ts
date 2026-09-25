import { CborDeserializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborDeserializer.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';
import { Asset } from '@unicitylabs/state-transition-sdk/lib/payment/asset/Asset.js';
import { AssetId } from '@unicitylabs/state-transition-sdk/lib/payment/asset/AssetId.js';
import { PaymentAssetCollection } from '@unicitylabs/state-transition-sdk/lib/payment/asset/PaymentAssetCollection.js';

import { bytesEqual } from './hex.js';

export const WALLET_VALUE_TAG = 39050n;
export const WALLET_VALUE_VERSION = 1n;

/**
 * Reads the bridged-coin amount declared in a token's `data`, or null if the
 * token declares no value for `coinId`. Injected into the verifier so the
 * mint-reason check can confirm the token's declared value equals the locked
 * amount.
 */
export type BridgedAmountExtractor = (data: Uint8Array | null, coinId: Uint8Array) => bigint | null;

/** Encode a single-asset value payload in the wallet format, no memo. */
export function encodeBridgePaymentData(coinId: Uint8Array, amount: bigint): Uint8Array {
  const assets = PaymentAssetCollection.create(new Asset(new AssetId(coinId), amount));
  return CborSerializer.encodeTag(
    WALLET_VALUE_TAG,
    CborSerializer.encodeArray(
      CborSerializer.encodeUnsignedInteger(WALLET_VALUE_VERSION),
      assets.toCBOR(),
      CborSerializer.encodeNullable(null, CborSerializer.encodeByteString),
    ),
  );
}

/**
 * Decode the wallet-format payload and return the amount it declares for
 * `coinId`; null when the bytes are not that format, the version is unknown, or
 * the coin is absent.
 */
export function decodeBridgePaymentData(data: Uint8Array | null, coinId: Uint8Array): bigint | null {
  if (!data) return null;
  try {
    const tag = CborDeserializer.decodeTag(data);
    if (tag.tag !== WALLET_VALUE_TAG) return null;
    const fields = CborDeserializer.decodeArray(tag.data, 3);
    if (CborDeserializer.decodeUnsignedInteger(fields[0]) !== WALLET_VALUE_VERSION) return null;
    return PaymentAssetCollection.fromCBOR(fields[1]).get(new AssetId(coinId))?.value ?? null;
  } catch {
    return null;
  }
}

/**
 * Minimal self-contained value envelope used by the CLI/tests only:
 * `CBOR [ coinId: bstr, amount: uint ]`. Never appears on a real bridged token.
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
