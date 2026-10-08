import { decodeLockEvent, LOCK_EVENT_TOPIC0 } from '../lock-event.js';
import { toHex } from '../hex.js';
import type { ConstantCaller, LogFilter, LogReader, NonceReader, SourceChainRpc, SourceLogEntry } from '../source-chain.js';
import type { LoadedBridge } from './manifest.js';

export interface LockSearch {
  /** The account that signed the deposit, 20-byte hex. */
  readonly fromAddressHex: string;
  /** The deposit's token id, 32-byte hex: derived from its salt, so it names this deposit alone. */
  readonly unicityTokenIdHex: string;
  readonly startedAtMs: number;
  readonly nowMs: number;
}

/**
 * What the chain says about a deposit's lock: found with its transaction; absent, the vault
 * holds no lock with the token id and nothing from the account is in flight; or unknown, with
 * the reason, when the vault has the lock but the node's logs do not show it, or a transaction
 * from the account is still pending.
 */
export type LockSearchResult =
  | { readonly outcome: 'found'; readonly txid: string }
  | { readonly outcome: 'absent' }
  | { readonly outcome: 'unknown'; readonly why: string };

export type LockSearchRpc = LogReader & SourceChainRpc & ConstantCaller & NonceReader;

const ETHEREUM_BLOCK_MS = 12_000;
const BLOCKS_OF_SLACK = 300n;
const WINDOW = 10_000n;
const SMALLEST_WINDOW = 1_000n;
const RATE_LIMIT_PAUSE_MS = 2_000;
const RATE_LIMITED = /\b429\b|\[-32005\]|rate limit|rate exceeded|too many requests/i;

/**
 * The vault records every token id it has locked, so that is asked first; it is exact and needs
 * no block window. A locked token id is then located in the vault's `Lock` events, among the
 * signer's first, then any account's, from the blocks around the deposit's start and further
 * back until found. A read of the chain, nothing is sent. A node that refuses a window gets it
 * halved, down to a floor; a rate limit is retried once after a pause.
 */
export async function findLock(bridge: LoadedBridge, rpc: LockSearchRpc, search: LockSearch): Promise<LockSearchResult> {
  const vault = bridge.plugin.resolvedConfig.lockContractHex;
  const tokenId = search.unicityTokenIdHex.toLowerCase();
  if (!(await tokenIdUsed(rpc, vault, search.fromAddressHex, tokenId))) {
    return (await inFlight(rpc, search.fromAddressHex))
      ? { outcome: 'unknown', why: 'a transaction from the account is still pending' }
      : { outcome: 'absent' };
  }
  const tip = await rpc.getNowBlockNumber();
  const since = BigInt(Math.ceil(Math.max(0, search.nowMs - search.startedAtMs) / ETHEREUM_BLOCK_MS));
  const from = search.fromAddressHex.toLowerCase().padStart(64, '0');
  const txid = await locate(rpc, vault, tokenId, from, later(tip - since - BLOCKS_OF_SLACK, 0n), tip);
  return txid === null
    ? { outcome: 'unknown', why: 'the vault has locked this token id, but its transaction was not found in the logs the node serves' }
    : { outcome: 'found', txid };
}

async function tokenIdUsed(rpc: ConstantCaller, vault: string, ownerHex: string, tokenIdHex: string): Promise<boolean> {
  const word = await rpc.constantCall({ ownerHex, contractHex: vault, functionSignature: 'tokenIdUsed(bytes32)', parameterHex: tokenIdHex });
  return BigInt(`0x${word || '0'}`) !== 0n;
}

async function inFlight(rpc: NonceReader, addressHex: string): Promise<boolean> {
  const [pending, latest] = await Promise.all([rpc.getTransactionCount(addressHex, 'pending'), rpc.getTransactionCount(addressHex, 'latest')]);
  return pending > latest;
}

/** The lock's transaction: the signer's locks from `earliest` on, then anyone's, then further back. */
async function locate(rpc: LogReader, vault: string, tokenId: string, from: string, earliest: bigint, tip: bigint): Promise<string | null> {
  for (const topic of [from, null]) {
    const txid = await scan(rpc, { address: vault, topics: [LOCK_EVENT_TOPIC0, null, topic] }, tokenId, earliest, tip);
    if (txid) return txid;
  }
  let toBlock = earliest - 1n;
  for (let span = tip - earliest + 1n; toBlock >= 0n; span *= 2n) {
    const fromBlock = later(toBlock - span + 1n, 0n);
    const txid = await scan(rpc, { address: vault, topics: [LOCK_EVENT_TOPIC0, null, null] }, tokenId, fromBlock, toBlock);
    if (txid) return txid;
    toBlock = fromBlock - 1n;
  }
  return null;
}

async function scan(rpc: LogReader, filter: Omit<LogFilter, 'fromBlock' | 'toBlock'>, tokenId: string, earliest: bigint, tip: bigint): Promise<string | null> {
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
    if (!RATE_LIMITED.test(String((err as Error)?.message))) throw err;
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
