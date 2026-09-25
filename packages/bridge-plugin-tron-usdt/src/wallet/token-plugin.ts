import type { WalletTokenPlugin } from '@unicitylabs/bridge-core';

import type { LoadedBridge } from './manifest.js';

export function bridgeTokenPlugin(bridge: LoadedBridge): WalletTokenPlugin {
  return {
    id: `bridge:${bridge.manifest.chainRef}:${bridge.manifest.symbol.toLowerCase()}`,
    mintJustificationVerifiers: [bridge.plugin.verifier],
  };
}
