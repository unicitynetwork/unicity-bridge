/**
 * Tron wallet providers (08 Phase 3 — the picker seam). TronLink is always offered;
 * WalletConnect appears only when the app injects a signer factory. Each provider
 * `create`s a `TronSigner` the bridge-in flow drives uniformly.
 */
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { availableTronWallets, type TronSigner } from '../src/wallet/index.js';

const NILE = 3448148188;
const fakeSigner = (): TronSigner => ({
  connect: async () => '',
  getAddress: async () => '',
  getNetwork: async () => NILE,
  sendCall: async () => '',
});

test('availableTronWallets offers TronLink always, WalletConnect only when configured', () => {
  assert.deepEqual(availableTronWallets().map((p) => p.id), ['tronlink']);

  const withWc = availableTronWallets({ walletConnect: { signerFactory: () => fakeSigner() } });
  assert.deepEqual(withWc.map((p) => p.id), ['tronlink', 'walletconnect']);
});

test('a provider.create builds a TronSigner for the given chainId', () => {
  const tronlink = availableTronWallets()[0];
  const signer = tronlink.create(NILE);
  assert.equal(typeof signer.connect, 'function');
  assert.equal(typeof signer.getNetwork, 'function');
  assert.equal(typeof signer.sendCall, 'function');
});

test('the WalletConnect provider delegates create() to the injected factory', () => {
  const marker = fakeSigner();
  const [, wc] = availableTronWallets({ walletConnect: { signerFactory: () => marker } });
  assert.equal(wc.create(NILE), marker);
});
