import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  TronUsdtLockJustification,
  TRON_USDT_LOCK_JUSTIFICATION_TAG,
} from '../src/index.js';
import { hexToBytes } from './helpers.js';

test('justification round-trips through CBOR', () => {
  const original = new TronUsdtLockJustification({
    chainId: 3448148188,
    lockContract: hexToBytes('00'.repeat(19) + 'ab'),
    assetContract: hexToBytes('00'.repeat(19) + 'cd'),
    txid: hexToBytes('11'.repeat(32)),
    logIndex: 2,
    amount: 1_234_567n,
    nonce: 42n,
  });

  const decoded = TronUsdtLockJustification.fromCBOR(original.toCBOR());
  assert.equal(decoded.data.chainId, 3448148188);
  assert.equal(decoded.data.logIndex, 2);
  assert.equal(decoded.data.amount, 1_234_567n);
  assert.equal(decoded.data.nonce, 42n);
  assert.deepEqual(decoded.data.txid, original.data.txid);
  assert.deepEqual(decoded.data.lockContract, original.data.lockContract);
});

test('decoding rejects a foreign CBOR tag', () => {
  // Canonical CBOR tag(6) wrapping an empty array — a valid but foreign tag.
  const bogus = new Uint8Array([0xc6, 0x80]);
  assert.throws(() => TronUsdtLockJustification.fromCBOR(bogus), /Invalid CBOR tag/);
});

test('the tag is the allocated bridge tag', () => {
  assert.equal(TronUsdtLockJustification.CBOR_TAG, TRON_USDT_LOCK_JUSTIFICATION_TAG);
  assert.equal(TRON_USDT_LOCK_JUSTIFICATION_TAG, 1330002n);
});
