import assert from 'node:assert/strict';
import { test } from 'node:test';

import { LOCK_EVENT_TOPIC0, toHex } from '../src/index.js';
import type { ConstantCallInput, LogFilter, SourceLogEntry } from '../src/source-chain.js';
import { findLock, loadBridges, SEPOLIA_USDC_BRIDGE, type LockSearch } from '../src/wallet/index.js';
import { makeLockLog } from './helpers.js';

const VAULT = SEPOLIA_USDC_BRIDGE.vault.slice(2).toLowerCase();
const FROM = 'ab'.repeat(20);
const OTHER_FROM = 'cd'.repeat(20);
const COMMITMENT = new Uint8Array(32).fill(7);
const TOKEN_ID = new Uint8Array(32).fill(3);
const OTHER_TOKEN_ID = new Uint8Array(32).fill(4);
const MINUTE = 60_000;
const NOW = 1_800_000_000_000;

/** The Sepolia bridge on a chain that starts at block 0, so the small fake tips below are after its deployment. */
function bridge(deployBlock = 0) {
  return loadBridges({ ...SEPOLIA_USDC_BRIDGE, deployBlock }, { rpc: { getTransactionInfo: async () => null, getNowBlockNumber: async () => 0n } })[0];
}

function lockAt(blockNumber: bigint, txid: string, tokenId: Uint8Array, from = FROM): SourceLogEntry {
  return { ...makeLockLog(VAULT, { nonce: 1n, fromEvmHex: from, amount: 5n, unicityTokenId: tokenId, recipientCommitment: COMMITMENT }), blockNumber, transactionHash: txid };
}

interface Chain {
  readonly tip: bigint;
  readonly locks?: readonly SourceLogEntry[];
  /** Which token ids the vault reports as locked; defaults to those of the mined locks. */
  readonly used?: readonly Uint8Array[];
  /** What the vault answers instead of a word, to stand in for a wrong address or a bare node. */
  readonly vaultAnswer?: string;
  /** The vault reports the token id locked only from this call on: a lock mined mid-search. */
  readonly usedFromCall?: number;
  readonly pending?: bigint;
  readonly latest?: bigint;
  readonly failing?: (filter: LogFilter, attempt: number) => Error | null;
}

function fakeRpc(chain: Chain) {
  const filters: LogFilter[] = [];
  const calls: ConstantCallInput[] = [];
  const locks = chain.locks ?? [];
  const used = (chain.used ?? locks.map((l) => Uint8Array.from(Buffer.from(l.data.slice(64, 128), 'hex')))).map((t) => toHex(t));
  const word = (b: boolean) => (b ? '1' : '0').padStart(64, '0');
  return {
    filters,
    calls,
    rpc: {
      getNowBlockNumber: async () => chain.tip,
      getTransactionInfo: async () => null,
      getLogs: async (filter: LogFilter) => {
        filters.push(filter);
        const failure = chain.failing?.(filter, filters.length);
        if (failure) throw failure;
        return locks.filter((e) => {
          const fromTopic = filter.topics[2];
          return e.blockNumber >= filter.fromBlock && e.blockNumber <= filter.toBlock && (fromTopic === null || e.topics[2] === fromTopic);
        });
      },
      constantCall: async (input: ConstantCallInput) => {
        calls.push(input);
        if (chain.vaultAnswer !== undefined) return chain.vaultAnswer;
        const known = used.includes(input.parameterHex ?? '') && calls.length >= (chain.usedFromCall ?? 1);
        return word(known);
      },
      getTransactionCount: async (_address: string, tag: 'latest' | 'pending') => (tag === 'pending' ? (chain.pending ?? 0n) : (chain.latest ?? 0n)),
    },
  };
}

function search(startedAgoMs: number): LockSearch {
  return { fromAddressHex: FROM, unicityTokenIdHex: toHex(TOKEN_ID), startedAtMs: NOW - startedAgoMs, nowMs: NOW };
}

test('asks the vault first whether the token id is locked, then finds the lock among the signer locks', async () => {
  const { rpc, filters, calls } = fakeRpc({ tip: 1_000n, locks: [lockAt(990n, 'aa'.repeat(32), OTHER_TOKEN_ID), lockAt(995n, 'bb'.repeat(32), TOKEN_ID)] });
  assert.deepEqual(await findLock(bridge(), rpc, search(MINUTE)), { outcome: 'found', txid: 'bb'.repeat(32) });
  assert.equal(calls[0].contractHex, VAULT);
  assert.equal(calls[0].functionSignature, 'tokenIdUsed(bytes32)');
  assert.equal(calls[0].parameterHex, toHex(TOKEN_ID));
  assert.equal(filters[0].address, VAULT);
  assert.deepEqual(filters[0].topics, [LOCK_EVENT_TOPIC0, null, FROM.padStart(64, '0')]);
});

test('answers absent, without reading any logs, when the vault has no lock with the token id and nothing from the account is pending', async () => {
  const { rpc, filters } = fakeRpc({ tip: 1_000n, locks: [lockAt(995n, 'aa'.repeat(32), OTHER_TOKEN_ID)], pending: 7n, latest: 7n });
  assert.deepEqual(await findLock(bridge(), rpc, search(MINUTE)), { outcome: 'absent' });
  assert.equal(filters.length, 0);
});

test('answers unknown while a transaction from the account is still pending', async () => {
  const { rpc } = fakeRpc({ tip: 1_000n, pending: 8n, latest: 7n });
  const result = await findLock(bridge(), rpc, search(MINUTE));
  assert.equal(result.outcome, 'unknown');
  assert.match((result as { why: string }).why, /pending/);
});

test('finds a lock the account sent from another address once the vault says the token id is locked', async () => {
  const { rpc, filters } = fakeRpc({ tip: 1_000n, locks: [lockAt(995n, 'cc'.repeat(32), TOKEN_ID, OTHER_FROM)] });
  assert.deepEqual(await findLock(bridge(), rpc, search(MINUTE)), { outcome: 'found', txid: 'cc'.repeat(32) });
  assert.deepEqual(filters.map((f) => f.topics[2]), [FROM.padStart(64, '0'), null]);
});

test('keeps looking further back when the vault says the lock exists but the first window has not got it', async () => {
  const twoDays = 2 * 24 * 60 * MINUTE;
  const { rpc, filters } = fakeRpc({ tip: 100_000n, locks: [lockAt(40_000n, 'dd'.repeat(32), TOKEN_ID)] });
  assert.deepEqual(await findLock(bridge(), rpc, search(twoDays)), { outcome: 'found', txid: 'dd'.repeat(32) });
  assert.equal(filters[0].fromBlock, 100_000n - 14_400n - 300n);
  assert.ok(filters.some((f) => f.fromBlock <= 40_000n && f.toBlock >= 40_000n));
});

test('answers unknown when the vault says the lock exists but the node has no log of it back to the first block', async () => {
  const { rpc } = fakeRpc({ tip: 5_000n, used: [TOKEN_ID] });
  const result = await findLock(bridge(), rpc, search(MINUTE));
  assert.equal(result.outcome, 'unknown');
  assert.match((result as { why: string }).why, /locked.*not found/);
});

test('walks a long range in windows of ten thousand blocks from where the deposit started, and stops at the first hit', async () => {
  const twoDays = 2 * 24 * 60 * MINUTE;
  const earliest = 100_000n - 14_400n - 300n;
  const { rpc, filters } = fakeRpc({ tip: 100_000n, locks: [lockAt(earliest + 50n, 'cc'.repeat(32), TOKEN_ID)] });
  assert.deepEqual(await findLock(bridge(), rpc, search(twoDays)), { outcome: 'found', txid: 'cc'.repeat(32) });
  assert.deepEqual(filters.map((f) => [f.fromBlock, f.toBlock]), [[earliest, earliest + 9_999n]]);
});

test('halves the window when the node refuses a range, and gives up only when it refuses the smallest', async () => {
  const capped = fakeRpc({ tip: 30_000n, locks: [lockAt(29_990n, 'dd'.repeat(32), TOKEN_ID)], failing: (f) => (f.toBlock - f.fromBlock >= 2_000n ? new Error('Ethereum RPC eth_getLogs failed: block range too large') : null) });
  assert.deepEqual(await findLock(bridge(), capped.rpc, search(2 * 24 * 60 * MINUTE)), { outcome: 'found', txid: 'dd'.repeat(32) });

  const hopeless = fakeRpc({ tip: 30_000n, used: [TOKEN_ID], failing: () => new Error('Ethereum RPC eth_getLogs failed: HTTP 500') });
  await assert.rejects(findLock(bridge(), hopeless.rpc, search(MINUTE)), /HTTP 500/);
});

test('retries once after a rate limit, whether reported as HTTP 429 or as the node error code', async () => {
  for (const message of ['Ethereum RPC eth_getLogs failed: HTTP 429', 'Ethereum RPC eth_getLogs failed: [-32005] project ID request rate exceeded', 'Ethereum RPC eth_getLogs failed: Too Many Requests']) {
    const limited = fakeRpc({ tip: 1_000n, locks: [lockAt(995n, 'ee'.repeat(32), TOKEN_ID)], failing: (_f, attempt) => (attempt === 1 ? new Error(message) : null) });
    assert.deepEqual(await findLock(bridge(), limited.rpc, search(MINUTE)), { outcome: 'found', txid: 'ee'.repeat(32) });
    assert.equal(limited.filters.length, 2, message);
  }
});

test('asks the vault again after finding nothing pending, so a lock mined between the two reads is not called absent', async () => {
  const { rpc, calls } = fakeRpc({ tip: 1_000n, locks: [lockAt(999n, 'ff'.repeat(32), TOKEN_ID)], usedFromCall: 2, pending: 3n, latest: 3n });
  assert.deepEqual(await findLock(bridge(), rpc, search(MINUTE)), { outcome: 'found', txid: 'ff'.repeat(32) });
  assert.equal(calls.length, 2);
});

test('refuses a vault answer that is not one word, which is what a wrong address or a bare node gives', async () => {
  for (const vaultAnswer of ['', '0x', 'ab'.repeat(33)]) {
    await assert.rejects(findLock(bridge(), fakeRpc({ tip: 1_000n, vaultAnswer }).rpc, search(MINUTE)), /tokenIdUsed/);
  }
});

test('never searches before the vault was deployed', async () => {
  const { rpc, filters } = fakeRpc({ tip: 100_000n, used: [TOKEN_ID] });
  const result = await findLock(bridge(90_000), rpc, search(MINUTE));
  assert.equal(result.outcome, 'unknown');
  assert.ok(filters.every((f) => f.fromBlock >= 90_000n), JSON.stringify(filters.map((f) => f.fromBlock.toString())));
});
