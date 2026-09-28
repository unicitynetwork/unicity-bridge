import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { decodeBridgePaymentData, fromHex } from '../src/index.js';

const vector = JSON.parse(readFileSync(new URL('../../../protocol/vectors/value/value-00.json', import.meta.url), 'utf8'));
const coinId = fromHex(vector.in.coin_id);

test('the value payloads the prover accepts declare their amount', () => {
  for (const payload of vector.valid) {
    assert.equal(decodeBridgePaymentData(fromHex(payload), coinId), BigInt(vector.in.amount));
  }
});

test('a value payload the prover rejects declares no value', () => {
  for (const invalid of vector.invalid) {
    assert.equal(decodeBridgePaymentData(fromHex(invalid.payload), coinId), null, invalid.why);
  }
});
