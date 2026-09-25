import type { WalletTokenPlugin } from '@unicitylabs/bridge-core';

import { BridgeMintJustificationVerifier } from '../BridgeMintJustificationVerifier.js';
import { LockMintJustificationVerifier } from '../LockMintJustificationVerifier.js';
import type { LoadedBridge } from './manifest.js';

export function bridgeTokenPlugin(bridge: LoadedBridge): WalletTokenPlugin {
  return {
    id: `bridge:${bridge.manifest.chainRef}:${bridge.manifest.symbol.toLowerCase()}`,
    mintJustificationVerifiers: [bridge.plugin.verifier],
  };
}

export function mergeBridgeTokenPlugins(plugins: readonly WalletTokenPlugin[]): WalletTokenPlugin {
  const verifiers = plugins.flatMap((p) => p.mintJustificationVerifiers);
  for (const v of verifiers) {
    if (!(v instanceof LockMintJustificationVerifier)) {
      throw new Error(`mergeBridgeTokenPlugins: verifier for tag ${v.tag} is not a bridge lock verifier`);
    }
  }
  return {
    id: 'bridge',
    mintJustificationVerifiers: [new BridgeMintJustificationVerifier(verifiers as LockMintJustificationVerifier[])],
  };
}
