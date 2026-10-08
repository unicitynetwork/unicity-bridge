import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { ConstantCallInput } from '../src/index.js';
import { loadBridges, queryBalance, SEPOLIA_USDC_BRIDGE } from '../src/wallet/index.js';

const OWNER = '0x2B00d708fc777F174A248B9bE01c8E8379d69Caf';
const idle = () => ({ rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } });

test('queryBalance reads balanceOf(address) off the asset for the owner', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, idle());
  const calls: ConstantCallInput[] = [];
  const rpc = { constantCall: async (input: ConstantCallInput) => (calls.push(input), (3_000_000).toString(16).padStart(64, '0')) };
  assert.equal(await queryBalance(rpc, { assetAddress: SEPOLIA_USDC_BRIDGE.asset, owner: OWNER }), 3_000_000n);
  assert.equal(calls[0].contractHex, bridge.plugin.resolvedConfig.assetContractHex);
  assert.equal(calls[0].ownerHex, OWNER.slice(2).toLowerCase());
  assert.equal(calls[0].functionSignature, 'balanceOf(address)');
  assert.equal(calls[0].parameterHex, OWNER.slice(2).toLowerCase().padStart(64, '0'));
});

test('queryBalance refuses an answer that is not one word, which is what a wrong asset address or chain gives', async () => {
  for (const word of ['', '0x', 'ab'.repeat(33)]) {
    await assert.rejects(queryBalance({ constantCall: async () => word }, { assetAddress: SEPOLIA_USDC_BRIDGE.asset, owner: OWNER }), /balanceOf/);
  }
});
