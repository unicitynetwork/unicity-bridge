/**
 * Explorer/address UI helpers (08 §8) — chain-keyed so the wallet UI never
 * hardcodes a Nile URL or the Tron address shape.
 */
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { TRON_MAINNET_CHAIN_ID, TRON_NILE_CHAIN_ID } from '../src/config.js';
import { explorerTxUrl, isValidTronAddress, tronPresentation } from '../src/wallet/explorer.js';

test('explorerTxUrl points at nile.tronscan for the Nile chainId', () => {
  assert.equal(
    explorerTxUrl(TRON_NILE_CHAIN_ID, 'abc123'),
    'https://nile.tronscan.org/#/transaction/abc123',
  );
});

test('explorerTxUrl points at mainnet tronscan otherwise', () => {
  assert.equal(
    explorerTxUrl(TRON_MAINNET_CHAIN_ID, 'abc123'),
    'https://tronscan.org/#/transaction/abc123',
  );
});

test('isValidTronAddress accepts a well-formed T… address', () => {
  assert.equal(isValidTronAddress('TMckEpYxv8QA7oL36FvFRR7Gg1bL5DHsbt'), true);
  assert.equal(isValidTronAddress('  TMckEpYxv8QA7oL36FvFRR7Gg1bL5DHsbt  '), true); // trims
});

test('isValidTronAddress rejects non-Tron / malformed input', () => {
  assert.equal(isValidTronAddress('0x1234'), false);
  assert.equal(isValidTronAddress('bc1qxyz'), false);
  assert.equal(isValidTronAddress('T123'), false);
  assert.equal(isValidTronAddress(''), false);
});

test('tronPresentation bundles explorer + address validation for a chainId (no chainId at call site)', () => {
  const pres = tronPresentation(TRON_NILE_CHAIN_ID);
  assert.equal(pres.explorerTxUrl('abc123'), 'https://nile.tronscan.org/#/transaction/abc123');
  assert.equal(pres.validateAddress('TMckEpYxv8QA7oL36FvFRR7Gg1bL5DHsbt'), true);
  assert.equal(pres.validateAddress('0xdeadbeef'), false);
});
