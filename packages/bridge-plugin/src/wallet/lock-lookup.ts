import { decodeLockEvent, LOCK_EVENT_TOPIC0 } from '../lock-event.js';
import { toHex } from '../hex.js';
import type { LogReader, SourceChainRpc } from '../source-chain.js';
import type { LoadedBridge } from './manifest.js';

export interface LockSearch {
  /** The account that signed the deposit, 20-byte hex. */
  readonly fromAddressHex: string;
  /** The deposit's recipient commitment, 32-byte hex; what tells its lock from the signer's others. */
  readonly recipientCommitmentHex: string;
  readonly startedAtMs: number;
  readonly nowMs: number;
}

const ETHEREUM_BLOCK_MS = 12_000;
const BLOCKS_OF_SLACK = 300n;
const WINDOW = 10_000n;

/**
 * The transaction that locked a deposit, found from the vault's `Lock` events for the signer
 * since the deposit started, or `null` when none carries its commitment. A read of the chain,
 * nothing is sent. Newest blocks first, in windows a public node serves.
 */
export async function findLockTxid(bridge: LoadedBridge, rpc: LogReader & SourceChainRpc, search: LockSearch): Promise<string | null> {
  const tip = await rpc.getNowBlockNumber();
  const since = BigInt(Math.ceil(Math.max(0, search.nowMs - search.startedAtMs) / ETHEREUM_BLOCK_MS));
  const earliest = later(tip - since - BLOCKS_OF_SLACK, 0n);
  const commitment = search.recipientCommitmentHex.toLowerCase();
  for (let toBlock = tip; toBlock >= earliest; toBlock -= WINDOW) {
    const fromBlock = later(toBlock - WINDOW + 1n, earliest);
    const logs = await rpc.getLogs({
      address: bridge.plugin.resolvedConfig.lockContractHex,
      topics: [LOCK_EVENT_TOPIC0, null, search.fromAddressHex.toLowerCase().padStart(64, '0')],
      fromBlock,
      toBlock,
    });
    const hit = logs.find((log) => toHex(decodeLockEvent(log)?.recipientCommitment ?? new Uint8Array()) === commitment);
    if (hit) return hit.transactionHash;
  }
  return null;
}

function later(a: bigint, b: bigint): bigint {
  return a > b ? a : b;
}
