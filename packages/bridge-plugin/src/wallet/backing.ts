import { toHex } from '../hex.js';
import { BridgeLockJustification } from '../BridgeLockJustification.js';

import type { LoadedBridge } from './manifest.js';

export function mintedAgainst(bridge: LoadedBridge, justification: Uint8Array | null): boolean {
  if (!justification) return false;
  let lock: BridgeLockJustification;
  try {
    lock = BridgeLockJustification.fromCBOR(justification);
  } catch {
    return false;
  }
  const cfg = bridge.plugin.resolvedConfig;
  return lock.data.chainId === cfg.chainId && toHex(lock.data.lockContract).toLowerCase() === cfg.lockContractHex;
}
