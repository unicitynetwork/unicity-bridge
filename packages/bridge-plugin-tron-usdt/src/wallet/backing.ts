import { toHex } from '../hex.js';
import { TronUsdtLockJustification } from '../TronUsdtLockJustification.js';

import type { LoadedBridge } from './manifest.js';

export function mintedAgainst(bridge: LoadedBridge, justification: Uint8Array | null): boolean {
  if (!justification) return false;
  let lock: TronUsdtLockJustification;
  try {
    lock = TronUsdtLockJustification.fromCBOR(justification);
  } catch {
    return false;
  }
  const cfg = bridge.plugin.resolvedConfig;
  return lock.data.chainId === cfg.chainId && toHex(lock.data.lockContract).toLowerCase() === cfg.lockContractHex;
}
