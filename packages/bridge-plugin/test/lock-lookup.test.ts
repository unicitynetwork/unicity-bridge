import assert from 'node:assert/strict';
import { test } from 'node:test';

import { LOCK_EVENT_TOPIC0, toHex } from '../src/index.js';
import type { LogFilter, SourceLogEntry } from '../src/source-chain.js';
import { findLockTxid, loadBridges, SEPOLIA_USDC_BRIDGE, type LockSearch } from '../src/wallet/index.js';
import { makeLockLog } from './helpers.js';

const VAULT = SEPOLIA_USDC_BRIDGE.vault.slice(2).toLowerCase();
const FROM = 'ab'.repeat(20);
const COMMITMENT = new Uint8Array(32).fill(7);
const OTHER_COMMITMENT = new Uint8Array(32).fill(9);
const MINUTE = 60_000;
const NOW = 1_800_000_000_000;

function bridge() {
  return loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } })[0];
}

function lockAt(blockNumber: bigint, txid: string, commitment: Uint8Array): SourceLogEntry {
  return { ...makeLockLog(VAULT, { nonce: 1n, fromEvmHex: FROM, amount: 5n, unicityTokenId: new Uint8Array(32), recipientCommitment: commitment }), blockNumber, transactionHash: txid };
}

function fakeRpc(tip: bigint, entries: readonly SourceLogEntry[]) {
  const filters: LogFilter[] = [];
  return {
    filters,
    rpc: {
      getNowBlockNumber: async () => tip,
      getTransactionInfo: async () => null,
      getLogs: async (filter: LogFilter) => {
        filters.push(filter);
        return entries.filter((e) => e.blockNumber >= filter.fromBlock && e.blockNumber <= filter.toBlock);
      },
    },
  };
}

function search(startedAgoMs: number): LockSearch {
  return { fromAddressHex: FROM, recipientCommitmentHex: toHex(COMMITMENT), startedAtMs: NOW - startedAgoMs, nowMs: NOW };
}

test('finds the lock whose event carries the deposit commitment and ignores another deposit of the same signer', async () => {
  const { rpc, filters } = fakeRpc(1_000n, [lockAt(990n, 'aa'.repeat(32), OTHER_COMMITMENT), lockAt(995n, 'bb'.repeat(32), COMMITMENT)]);
  assert.equal(await findLockTxid(bridge(), rpc, search(MINUTE)), 'bb'.repeat(32));
  assert.equal(filters[0].address, VAULT);
  assert.deepEqual(filters[0].topics, [LOCK_EVENT_TOPIC0, null, FROM.padStart(64, '0')]);
});

test('answers null when no lock of that deposit is in the range', async () => {
  const { rpc } = fakeRpc(1_000n, [lockAt(995n, 'aa'.repeat(32), OTHER_COMMITMENT)]);
  assert.equal(await findLockTxid(bridge(), rpc, search(MINUTE)), null);
});

test('searches only the blocks since the deposit started, with some slack, and not before the chain began', async () => {
  const { rpc, filters } = fakeRpc(100_000n, []);
  await findLockTxid(bridge(), rpc, search(MINUTE));
  assert.equal(filters.length, 1);
  assert.equal(filters[0].toBlock, 100_000n);
  assert.equal(filters[0].fromBlock, 100_000n - 5n - 300n);

  const young = fakeRpc(50n, []);
  await findLockTxid(bridge(), young.rpc, search(MINUTE));
  assert.equal(young.filters[0].fromBlock, 0n);
});

test('walks a long range in windows of ten thousand blocks, newest first, and stops at the first hit', async () => {
  const twoDays = 2 * 24 * 60 * MINUTE;
  const { rpc, filters } = fakeRpc(100_000n, [lockAt(93_000n, 'cc'.repeat(32), COMMITMENT)]);
  assert.equal(await findLockTxid(bridge(), rpc, search(twoDays)), 'cc'.repeat(32));
  assert.deepEqual(filters.map((f) => [f.fromBlock, f.toBlock]), [[90_001n, 100_000n]]);

  const miss = fakeRpc(100_000n, []);
  assert.equal(await findLockTxid(bridge(), miss.rpc, search(twoDays)), null);
  const first = miss.filters[0];
  const last = miss.filters.at(-1)!;
  assert.equal(first.toBlock, 100_000n);
  assert.ok(miss.filters.every((f) => f.toBlock - f.fromBlock < 10_000n));
  assert.equal(last.fromBlock, 100_000n - 14_400n - 300n);
});
