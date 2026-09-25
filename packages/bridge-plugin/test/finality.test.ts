import assert from 'node:assert/strict';
import { test } from 'node:test';

import { BridgeLockJustification, chainFamily, fromHex, LOCK_EVENT_TOPIC0, type SourceTxInfo } from '../src/index.js';
import { loadBridges, lockFinality, SEPOLIA_USDC_BRIDGE } from '../src/wallet/index.js';

const VAULT = '9c2bf4ed5b85130ffd14be8fa65c60f299fc9a2e';
const USDC = '1c7d4b196cb0c7b01d743fbc6116a902379c7238';

function rpcAt(tip: bigint, tx: SourceTxInfo | null) {
  return { getTransactionInfo: async () => tx, getNowBlockNumber: async () => tip };
}

function justification(lockContract = VAULT): Uint8Array {
  return new BridgeLockJustification({
    chainId: 11155111,
    lockContract: fromHex(lockContract),
    assetContract: fromHex(USDC),
    txid: fromHex('ab'.repeat(32)),
    logIndex: 0,
    amount: 1_000_000n,
    nonce: 1n,
  }).toCBOR();
}

const lockAt = (block: bigint): SourceTxInfo => ({ blockNumber: block, success: true, logs: [{ address: VAULT, topics: [LOCK_EVENT_TOPIC0], data: '' }] });

test('the Sepolia manifest asks for a testnet finality of 12 blocks, and each family knows its block time', () => {
  assert.equal(SEPOLIA_USDC_BRIDGE.confirmations, 12);
  assert.equal(chainFamily('eip155').blockSeconds, 12);
  assert.equal(chainFamily('tron').blockSeconds, 3);
});

test('a lock still short of the manifest finality is settling, with the time left', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: rpcAt(1005n, lockAt(1000n)) });
  const f = await lockFinality(bridge, justification());
  assert.deepEqual(f, { confirmations: 5n, required: 12, final: false, secondsLeft: 7 * 12 });
});

test('a lock at or past the manifest finality is final', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: rpcAt(1012n, lockAt(1000n)) });
  const f = await lockFinality(bridge, justification());
  assert.equal(f?.final, true);
  assert.equal(f?.secondsLeft, 0);
});

test('a lock the node does not know yet is settling for the whole finality window', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: rpcAt(1000n, null) });
  const f = await lockFinality(bridge, justification());
  assert.deepEqual(f, { confirmations: 0n, required: 12, final: false, secondsLeft: 12 * 12 });
});

test('a justification for another vault, or no justification, is not this bridge\'s concern', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: rpcAt(1000n, null) });
  assert.equal(await lockFinality(bridge, justification('ab'.repeat(20))), null);
  assert.equal(await lockFinality(bridge, null), null);
  assert.equal(await lockFinality(bridge, new Uint8Array([1, 2, 3])), null);
});
