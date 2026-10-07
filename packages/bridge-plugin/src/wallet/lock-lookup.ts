import { decodeLockEvent, LOCK_EVENT_TOPIC0 } from '../lock-event.js';
import { toHex } from '../hex.js';
import type { LogFilter, LogReader, SourceChainRpc, SourceLogEntry } from '../source-chain.js';
import type { LoadedBridge } from './manifest.js';

export interface LockSearch {
  /** The account that signed the deposit, 20-byte hex. */
  readonly fromAddressHex: string;
  /** The deposit's token id, 32-byte hex: derived from its salt, so it names this deposit alone. */
  readonly unicityTokenIdHex: string;
  readonly startedAtMs: number;
  readonly nowMs: number;
}

const ETHEREUM_BLOCK_MS = 12_000;
const BLOCKS_OF_SLACK = 300n;
const WINDOW = 10_000n;
const SMALLEST_WINDOW = 1_000n;
const RATE_LIMIT_PAUSE_MS = 2_000;

/**
 * The transaction that locked a deposit, found from the vault's `Lock` events for the signer
 * since the deposit started, or `null` when no mined block in that range carries its token id:
 * a lock still in the mempool, or on blocks a lagging node has not seen, also answers `null`.
 * A read of the chain, nothing is sent. From the blocks around the start onwards, in windows a
 * public node serves: a lock that went out did so within minutes of the deposit, so the first
 * window usually has it. A node that refuses a window gets it halved, down to a floor; a rate
 * limit is retried once after a pause.
 */
export async function findLockTxid(bridge: LoadedBridge, rpc: LogReader & SourceChainRpc, search: LockSearch): Promise<string | null> {
  const tip = await rpc.getNowBlockNumber();
  const since = BigInt(Math.ceil(Math.max(0, search.nowMs - search.startedAtMs) / ETHEREUM_BLOCK_MS));
  const earliest = later(tip - since - BLOCKS_OF_SLACK, 0n);
  const tokenId = search.unicityTokenIdHex.toLowerCase();
  const filter = {
    address: bridge.plugin.resolvedConfig.lockContractHex,
    topics: [LOCK_EVENT_TOPIC0, null, search.fromAddressHex.toLowerCase().padStart(64, '0')],
  };
  let window = WINDOW;
  for (let fromBlock = earliest; fromBlock <= tip; ) {
    const toBlock = earlier(fromBlock + window - 1n, tip);
    const logs = await readLogs(rpc, { ...filter, fromBlock, toBlock }).catch((err: unknown) => {
      if (window <= SMALLEST_WINDOW) throw err;
      window /= 2n;
      return null;
    });
    if (logs === null) continue;
    const hit = logs.find((log) => toHex(decodeLockEvent(log)?.unicityTokenId ?? new Uint8Array()) === tokenId);
    if (hit) return hit.transactionHash;
    fromBlock = toBlock + 1n;
  }
  return null;
}

async function readLogs(rpc: LogReader, filter: LogFilter): Promise<SourceLogEntry[]> {
  try {
    return await rpc.getLogs(filter);
  } catch (err) {
    if (!/\b429\b/.test(String((err as Error)?.message))) throw err;
    await new Promise((resolve) => setTimeout(resolve, RATE_LIMIT_PAUSE_MS));
    return rpc.getLogs(filter);
  }
}

function later(a: bigint, b: bigint): bigint {
  return a > b ? a : b;
}

function earlier(a: bigint, b: bigint): bigint {
  return a < b ? a : b;
}
