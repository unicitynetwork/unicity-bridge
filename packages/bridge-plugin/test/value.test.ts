import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Asset } from '@unicitylabs/state-transition-sdk/lib/payment/asset/Asset.js';
import { AssetId } from '@unicitylabs/state-transition-sdk/lib/payment/asset/AssetId.js';
import { PaymentAssetCollection } from '@unicitylabs/state-transition-sdk/lib/payment/asset/PaymentAssetCollection.js';
import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';

import { fromHex, toHex } from '../src/hex.js';
import { decodeBridgePaymentData, encodeBridgePaymentData, WALLET_VALUE_TAG } from '../src/value.js';

const COIN_ID = new Uint8Array(32).fill(0xab);
const OTHER_COIN = new Uint8Array(32).fill(0xcd);

const SPHERE_NO_MEMO = 'd9988a830181825820' + 'ab'.repeat(32) + '430f4240f6';
const SPHERE_WITH_MEMO = 'd9988a830181825820' + 'ab'.repeat(32) + '412a420102';

test('encodes byte-for-byte what sphere-sdk encodes for the same single-asset value', () => {
  assert.equal(toHex(encodeBridgePaymentData(COIN_ID, 1_000_000n)), SPHERE_NO_MEMO);
});

test('reads back what sphere-sdk encodes; a memo does not disturb the amount', () => {
  assert.equal(decodeBridgePaymentData(fromHex(SPHERE_NO_MEMO), COIN_ID), 1_000_000n);
  assert.equal(decodeBridgePaymentData(fromHex(SPHERE_WITH_MEMO), COIN_ID), 42n);
});

test('round trip; a coin the payload does not name reads as null', () => {
  const data = encodeBridgePaymentData(COIN_ID, 7n);
  assert.equal(decodeBridgePaymentData(data, COIN_ID), 7n);
  assert.equal(decodeBridgePaymentData(data, OTHER_COIN), null);
  assert.equal(decodeBridgePaymentData(null, COIN_ID), null);
});

test('a bare PaymentAssetCollection (the old bridge dialect) is NOT a value: null', () => {
  const bare = PaymentAssetCollection.create(new Asset(new AssetId(COIN_ID), 7n)).toCBOR();
  assert.equal(decodeBridgePaymentData(bare, COIN_ID), null);
});

test('an unknown payload version or a foreign tag reads as null, never as an amount', () => {
  const assets = PaymentAssetCollection.create(new Asset(new AssetId(COIN_ID), 7n)).toCBOR();
  const v2 = CborSerializer.encodeTag(
    WALLET_VALUE_TAG,
    CborSerializer.encodeArray(CborSerializer.encodeUnsignedInteger(2n), assets, CborSerializer.encodeNullable(null, CborSerializer.encodeByteString)),
  );
  assert.equal(decodeBridgePaymentData(v2, COIN_ID), null);
  const foreign = CborSerializer.encodeTag(
    39048n,
    CborSerializer.encodeArray(CborSerializer.encodeUnsignedInteger(1n), assets, CborSerializer.encodeNullable(null, CborSerializer.encodeByteString)),
  );
  assert.equal(decodeBridgePaymentData(foreign, COIN_ID), null);
  assert.equal(decodeBridgePaymentData(new Uint8Array([0xff]), COIN_ID), null);
});
