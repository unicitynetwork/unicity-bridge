import assert from 'node:assert/strict';
import { test } from 'node:test';

import { mintBridgedToken, type BridgePayments } from '@unicitylabs/bridge-core';
import { VerificationStatus } from '@unicitylabs/state-transition-sdk/lib/verification/VerificationStatus.js';

import { MockTronRpc } from '../src/cli/scenario.js';
import { createTronUsdtBridgePlugin, decodeBridgePaymentData, toHex, TRON_USDT_LOCK_JUSTIFICATION_TAG } from '../src/index.js';
import { bridgeTokenPlugin, createTronSourceAdapter, loadBridges, NILE_USDT_BRIDGE } from '../src/wallet/index.js';
import { buildScenario, CONFIG } from './helpers.js';

const deps = () => ({ rpc: new MockTronRpc(null, 0n) });
const noNestedTokens = (): void => {};

test('bridgeTokenPlugin: one plugin per bridged asset, carrying the STRICT verifier under the asset tag', () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const plugin = bridgeTokenPlugin(loaded!);

  assert.equal(plugin.id, `bridge:${NILE_USDT_BRIDGE.chainRef}:usdt`);
  assert.equal(plugin.mintJustificationVerifiers.length, 1);
  assert.equal(plugin.mintJustificationVerifiers[0]!.tag, TRON_USDT_LOCK_JUSTIFICATION_TAG);
  assert.equal(plugin.mintJustificationVerifiers[0], loaded!.plugin.verifier);
});

test('the adapter mint request carries the wallet-format value, the lock reason, and a 0-confirmation self verifier', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const wallet = { getAddress: async () => 'TAddr', sendCall: async () => 'txid' };
  const rpc = { allowance: async () => 0n } as never;
  const adapter = createTronSourceAdapter(loaded!, wallet, rpc, deps());

  const req = adapter.buildMintRequest({
    saltHex: 'a5'.repeat(32),
    amount: 1_000_000n,
    commit: { nonce: 19n, blockNumber: 71_070_991n, logIndex: 0 },
    commitTxid: 'e7'.repeat(32),
  });

  assert.equal(decodeBridgePaymentData(req.mintData, loaded!.plugin.resolvedConfig.coinId), 1_000_000n);
  assert.equal(toHex(req.tokenType), loaded!.plugin.tokenTypeHex);
  assert.equal(toHex(req.salt), 'a5'.repeat(32));
  assert.equal(req.coinIdHex, loaded!.plugin.coinIdHex);
  assert.equal(req.mintJustificationVerifiers.length, 1);
  assert.equal(req.mintJustificationVerifiers[0]!.tag, TRON_USDT_LOCK_JUSTIFICATION_TAG);
  assert.notEqual(req.mintJustificationVerifiers[0], loaded!.plugin.verifier, 'the self verifier is a separate, weaker instance');
});

test('the self verifier accepts a lock that is in a block but not final; the strict one does not', async () => {
  const s = await buildScenario({ tip: 101n });
  const strict = await s.plugin.verifier.verify(s.certifiedTx, noNestedTokens);
  assert.equal(strict.status, VerificationStatus.FAIL);
  assert.match(strict.message, /Insufficient confirmations: 1 < 20/);

  const self = createTronUsdtBridgePlugin({ ...CONFIG, confirmations: 0 }, { rpc: s.rpc }).verifier;
  const own = await self.verify(s.certifiedTx, noNestedTokens);
  assert.equal(own.status, VerificationStatus.OK, own.message);
});

test('mintBridgedToken maps the adapter request onto the wallet custom mint 1:1', async () => {
  const [loaded] = loadBridges(NILE_USDT_BRIDGE, deps());
  const wallet = { getAddress: async () => 'TAddr', sendCall: async () => 'txid' };
  const adapter = createTronSourceAdapter(loaded!, wallet, { allowance: async () => 0n } as never, deps());
  const req = adapter.buildMintRequest({
    saltHex: 'a5'.repeat(32),
    amount: 5n,
    commit: { nonce: 1n, blockNumber: 2n, logIndex: 0 },
    commitTxid: 'e7'.repeat(32),
  });
  const calls: unknown[] = [];
  const payments: BridgePayments = {
    mintCustom: async (r) => {
      calls.push(r);
      return { success: true, tokenId: 'ab'.repeat(32) };
    },
    burn: async () => {
      throw new Error('not used');
    },
    pendingBurns: async () => [],
    acknowledgeBurn: async () => {},
  };

  const result = await mintBridgedToken(payments, req);

  assert.deepEqual(result, { success: true, tokenId: 'ab'.repeat(32) });
  assert.deepEqual(calls, [
    {
      tokenType: req.tokenType,
      salt: req.salt,
      data: req.mintData,
      justification: req.genesisReason,
      assets: [{ coinId: req.coinIdHex, amount: 5n }],
      mintJustificationVerifiers: req.mintJustificationVerifiers,
    },
  ]);
});
