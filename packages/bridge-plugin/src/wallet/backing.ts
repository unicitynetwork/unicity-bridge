import { SplitMintJustification } from '@unicitylabs/state-transition-sdk/lib/payment/SplitMintJustification.js';

import { toHex } from '../hex.js';
import { BridgeLockJustification } from '../BridgeLockJustification.js';

import type { LoadedBridge } from './manifest.js';

export type SplitSourceReader = (justification: Uint8Array) => Promise<Uint8Array | null>;

export const splitSourceJustification: SplitSourceReader = async (justification) => {
  try {
    const split = await SplitMintJustification.fromCBOR(justification);
    return split.token.genesis.justification;
  } catch {
    return null;
  }
};

const MAX_SPLIT_DEPTH = 8;

export async function lockBehind(
  justification: Uint8Array | null,
  readSplit: SplitSourceReader = splitSourceJustification,
): Promise<BridgeLockJustification | null> {
  let bytes = justification;
  for (let depth = 0; bytes && depth <= MAX_SPLIT_DEPTH; depth++) {
    const lock = asLock(bytes);
    if (lock) return lock;
    bytes = await readSplit(bytes).catch(() => null);
  }
  return null;
}

function asLock(bytes: Uint8Array): BridgeLockJustification | null {
  try {
    return BridgeLockJustification.fromCBOR(bytes);
  } catch {
    return null;
  }
}

export async function mintedAgainst(
  bridge: LoadedBridge,
  justification: Uint8Array | null,
  readSplit: SplitSourceReader = splitSourceJustification,
): Promise<boolean> {
  const lock = await lockBehind(justification, readSplit);
  if (!lock) return false;
  const cfg = bridge.plugin.resolvedConfig;
  return lock.data.chainId === cfg.chainId && toHex(lock.data.lockContract).toLowerCase() === cfg.lockContractHex;
}
