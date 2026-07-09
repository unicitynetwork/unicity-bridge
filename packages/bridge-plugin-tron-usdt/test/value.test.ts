import assert from 'node:assert/strict';
import { test } from 'node:test';

import { CborSerializer } from '@unicitylabs/state-transition-sdk/lib/serialization/cbor/CborSerializer.js';

import { fromHex } from '../src/hex.js';
import { decodeBridgePaymentData, encodeBridgePaymentData } from '../src/value.js';

const COIN_ID = fromHex('11'.repeat(32));

test('bridge payment data decodes bare SDK PaymentAssetCollection', () => {
  const data = encodeBridgePaymentData(COIN_ID, 7n);
  assert.equal(decodeBridgePaymentData(data, COIN_ID), 7n);
});

test('bridge payment data rejects SpherePaymentData tag 39050', () => {
  const sphereInternalData = CborSerializer.encodeTag(39050n, encodeBridgePaymentData(COIN_ID, 7n));
  assert.equal(decodeBridgePaymentData(sphereInternalData, COIN_ID), null);
});
