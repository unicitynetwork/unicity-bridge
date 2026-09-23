import assert from 'node:assert/strict';
import { test } from 'node:test';

import { encodeCallData, selectorHex, type ConstantCallInput } from '../src/index.js';
import { loadBridges, owedTo, SEPOLIA_USDC_BRIDGE, withdrawCall } from '../src/wallet/index.js';

const VAULT = '9c2bf4ed5b85130ffd14be8fa65c60f299fc9a2e';
const DEST = '0x2B00d708fc777F174A248B9bE01c8E8379d69Caf';
const idle = () => ({ rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } });

test('owedTo reads owed(address) off the vault for the destination', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, idle());
  const calls: ConstantCallInput[] = [];
  const rpc = { constantCall: async (input: ConstantCallInput) => (calls.push(input), (1_000_000).toString(16).padStart(64, '0')) };
  assert.equal(await owedTo(bridge, rpc, DEST), 1_000_000n);
  assert.equal(calls[0].contractHex, VAULT);
  assert.equal(calls[0].functionSignature, 'owed(address)');
  assert.equal(calls[0].parameterHex, DEST.slice(2).toLowerCase().padStart(64, '0'));
});

test('withdrawCall targets the vault with no arguments', () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, idle());
  const call = withdrawCall(bridge);
  assert.deepEqual(call, { contractHex: VAULT, functionSignature: 'withdraw()', parameters: [] });
  assert.equal(encodeCallData(call), selectorHex('withdraw()'));
  assert.equal(selectorHex('withdraw()'), '3ccfd60b');
});
