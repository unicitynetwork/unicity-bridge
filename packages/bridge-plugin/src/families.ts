import type { ChainFamily } from '@unicitylabs/bridge-core';

import { evmFamily } from './evm/family.js';
import type { ChainFamilyAdapter } from './family.js';
import { tronFamily } from './tron/family.js';

export type { ChainFamilyAdapter, RpcOptions } from './family.js';

export function chainFamily(family: ChainFamily): ChainFamilyAdapter {
  switch (family) {
    case 'tron':
      return tronFamily;
    case 'eip155':
      return evmFamily;
  }
  throw new Error(`Unsupported chain family: ${String(family)}`);
}
