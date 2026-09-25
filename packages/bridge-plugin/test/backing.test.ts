import assert from 'node:assert/strict';
import { test } from 'node:test';

import { BridgeLockJustification, fromHex } from '../src/index.js';
import { loadBridges, lockBehind, mintedAgainst, SEPOLIA_USDC_BRIDGE, splitSourceJustification, type SplitSourceReader } from '../src/wallet/index.js';

const VAULT = '9c2bf4ed5b85130ffd14be8fa65c60f299fc9a2e';
const OTHER_VAULT = '00000000000000000000000000000000000000aa';
const USDC = '1c7d4b196cb0c7b01d743fbc6116a902379c7238';

function lock(lockContract = VAULT): Uint8Array {
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

const SPLIT_MARK = 0xff;

function split(source: Uint8Array): Uint8Array {
  return Uint8Array.from([SPLIT_MARK, ...source]);
}

const readSplit: SplitSourceReader = async (bytes) => {
  if (bytes[0] !== SPLIT_MARK) throw new Error('not a split');
  return bytes.slice(1);
};

test('a lock reason is the lock itself', async () => {
  const found = await lockBehind(lock(), readSplit);
  assert.equal(found?.data.nonce, 1n);
});

test('a split output leads back to the lock of the token it was split from', async () => {
  const found = await lockBehind(split(lock()), readSplit);
  assert.equal(found?.data.nonce, 1n);
});

test('a split of a split still reaches the lock', async () => {
  const found = await lockBehind(split(split(lock())), readSplit);
  assert.equal(found?.data.nonce, 1n);
});

test('a reason that is neither a lock nor a split has no lock behind it', async () => {
  assert.equal(await lockBehind(new Uint8Array([0x01, 0x02]), readSplit), null);
  assert.equal(await lockBehind(null, readSplit), null);
});

test('a split output of a token from this vault can be bridged out here', async () => {
  const [bridge] = loadBridges(SEPOLIA_USDC_BRIDGE);
  assert.equal(await mintedAgainst(bridge, split(lock()), readSplit), true);
  assert.equal(await mintedAgainst(bridge, split(lock(OTHER_VAULT)), readSplit), false);
});

test('the SDK reader answers null for bytes that are not a split reason', async () => {
  assert.equal(await splitSourceJustification(lock()), null);
  assert.equal(await splitSourceJustification(new Uint8Array([0x01, 0x02])), null);
});
