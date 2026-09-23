import { BridgeLockJustification } from '../BridgeLockJustification.js';
import { chainFamily } from '../families.js';
import { toHex } from '../hex.js';
import type { LoadedBridge } from './manifest.js';

export interface LockFinality {
  readonly confirmations: bigint;
  readonly required: number;
  readonly final: boolean;
  readonly secondsLeft: number;
}

export async function lockFinality(bridge: LoadedBridge, justification: Uint8Array | null): Promise<LockFinality | null> {
  if (!justification) return null;
  let lock: BridgeLockJustification;
  try {
    lock = BridgeLockJustification.fromCBOR(justification);
  } catch {
    return null;
  }
  const cfg = bridge.plugin.resolvedConfig;
  if (lock.data.chainId !== cfg.chainId || toHex(lock.data.lockContract).toLowerCase() !== cfg.lockContractHex) return null;

  const required = cfg.confirmations;
  const blockSeconds = chainFamily(cfg.family).blockSeconds;
  const info = await bridge.plugin.rpc.getTransactionInfo(toHex(lock.data.txid));
  const confirmations = info ? (await bridge.plugin.rpc.getNowBlockNumber()) - info.blockNumber : 0n;
  const missing = BigInt(required) - confirmations;
  return {
    confirmations,
    required,
    final: missing <= 0n,
    secondsLeft: missing <= 0n ? 0 : Number(missing) * blockSeconds,
  };
}
