import assert from 'node:assert/strict';
import { test } from 'node:test';

import { fromHex } from '../src/index.js';

test('fromHex reads 0x-prefixed and bare hex', () => {
  assert.deepEqual(fromHex('0x0aFf'), new Uint8Array([0x0a, 0xff]));
  assert.deepEqual(fromHex('0aff'), new Uint8Array([0x0a, 0xff]));
});

test('fromHex refuses a pair with a non-hex digit instead of reading it as zero', () => {
  assert.throws(() => fromHex('0g'), /Invalid hex/);
  assert.throws(() => fromHex('aa1z'), /Invalid hex/);
});
