import assert from 'node:assert/strict';
import { test } from 'node:test';

import { LOCK_EVENT_TOPIC0, toHex } from '../src/index.js';
import type { LogFilter, SourceLogEntry } from '../src/source-chain.js';
import { findLockTxid, loadBridges, SEPOLIA_USDC_BRIDGE, type LockSearch } from '../src/wallet/index.js';
import { makeLockLog } from './helpers.js';

const VAULT = SEPOLIA_USDC_BRIDGE.vault.slice(2).toLowerCase();
const FROM = 'ab'.repeat(20);
const COMMITMENT = new Uint8Array(32).fill(7);
const TOKEN_ID = new Uint8Array(32).fill(3);
const OTHER_TOKEN_ID = new Uint8Array(32).fill(4);
const MINUTE = 60_000;
const NOW = 1_800_000_000_000;

function bridge() {
  return loadBridges(SEPOLIA_USDC_BRIDGE, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } })[0];
}

function lockAt(blockNumber: bigint, txid: string, tokenId: Uint8Array): SourceLogEntry {
  return { ...makeLockLog(VAULT, { nonce: 1n, fromEvmHex: FROM, amount: 5n, unicityTokenId: tokenId, recipientCommitment: COMMITMENT }), blockNumber, transactionHash: txid };
}

function fakeRpc(tip: bigint, entries: readonly SourceLogEntry[], failing: (filter: LogFilter, attempt: number) => Error | null = () => null) {
  const filters: LogFilter[] = [];
  return {
    filters,
    rpc: {
      getNowBlockNumber: async () => tip,
      getTransactionInfo: async () => null,
      getLogs: async (filter: LogFilter) => {
        filters.push(filter);
        const failure = failing(filter, filters.length);
        if (failure) throw failure;
        return entries.filter((e) => e.blockNumber >= filter.fromBlock && e.blockNumber <= filter.toBlock);
      },
    },
  };
}

function search(startedAgoMs: number): LockSearch {
  return { fromAddressHex: FROM, unicityTokenIdHex: toHex(TOKEN_ID), startedAtMs: NOW - startedAgoMs, nowMs: NOW };
}

test('finds the lock whose event carries the deposit token id and ignores another deposit of the same signer, which shares its commitment', async () => {
  const { rpc, filters } = fakeRpc(1_000n, [lockAt(990n, 'aa'.repeat(32), OTHER_TOKEN_ID), lockAt(995n, 'bb'.repeat(32), TOKEN_ID)]);
  assert.equal(await findLockTxid(bridge(), rpc, search(MINUTE)), 'bb'.repeat(32));
  assert.equal(filters[0].address, VAULT);
  assert.deepEqual(filters[0].topics, [LOCK_EVENT_TOPIC0, null, FROM.padStart(64, '0')]);
});

test('answers null when no mined lock of that deposit is in the range', async () => {
  const { rpc } = fakeRpc(1_000n, [lockAt(995n, 'aa'.repeat(32), OTHER_TOKEN_ID)]);
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

test('walks a long range in windows of ten thousand blocks from where the deposit started, and stops at the first hit', async () => {
  const twoDays = 2 * 24 * 60 * MINUTE;
  const earliest = 100_000n - 14_400n - 300n;
  const { rpc, filters } = fakeRpc(100_000n, [lockAt(earliest + 50n, 'cc'.repeat(32), TOKEN_ID)]);
  assert.equal(await findLockTxid(bridge(), rpc, search(twoDays)), 'cc'.repeat(32));
  assert.deepEqual(filters.map((f) => [f.fromBlock, f.toBlock]), [[earliest, earliest + 9_999n]]);

  const miss = fakeRpc(100_000n, []);
  assert.equal(await findLockTxid(bridge(), miss.rpc, search(twoDays)), null);
  assert.equal(miss.filters[0].fromBlock, earliest);
  assert.ok(miss.filters.every((f) => f.toBlock - f.fromBlock < 10_000n));
  assert.equal(miss.filters.at(-1)!.toBlock, 100_000n);
});

test('halves the window when the node refuses a range, and gives up only when it refuses the smallest', async () => {
  const capped = fakeRpc(30_000n, [lockAt(29_990n, 'dd'.repeat(32), TOKEN_ID)], (f) => (f.toBlock - f.fromBlock >= 2_000n ? new Error('Ethereum RPC eth_getLogs failed: block range too large') : null));
  assert.equal(await findLockTxid(bridge(), capped.rpc, search(2 * 24 * 60 * MINUTE)), 'dd'.repeat(32));
  assert.ok(capped.filters.every((f, i) => i < 3 || f.toBlock - f.fromBlock < 2_000n));

  const hopeless = fakeRpc(30_000n, [], () => new Error('Ethereum RPC eth_getLogs failed: HTTP 500'));
  await assert.rejects(findLockTxid(bridge(), hopeless.rpc, search(MINUTE)), /HTTP 500/);
});

test('retries once after a rate limit', async () => {
  const limited = fakeRpc(1_000n, [lockAt(995n, 'ee'.repeat(32), TOKEN_ID)], (_f, attempt) => (attempt === 1 ? new Error('Ethereum RPC eth_getLogs failed: HTTP 429') : null));
  assert.equal(await findLockTxid(bridge(), limited.rpc, search(MINUTE)), 'ee'.repeat(32));
  assert.equal(limited.filters.length, 2);
});
