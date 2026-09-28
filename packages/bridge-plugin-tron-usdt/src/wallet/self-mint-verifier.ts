/**
 * The depositor's own mint-reason verifier: identical to the strict one the
 * wallet registers, but at `confirmations: 0`. The depositor witnessed its own
 * lock, so it accepts the mint as soon as the lock is in a block; every other
 * wallet that receives the token re-verifies under the manifest's `K`.
 */
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';

import { createTronUsdtBridgePlugin, type CreateTronUsdtBridgePluginDeps } from '../index.js';
import type { LoadedBridge } from './manifest.js';

export function selfMintVerifier(bridge: LoadedBridge, deps: CreateTronUsdtBridgePluginDeps = {}): IMintJustificationVerifier {
  return createTronUsdtBridgePlugin(
    {
      chainId: bridge.manifest.chainId,
      lockContract: bridge.manifest.vault,
      assetContract: bridge.manifest.asset,
      confirmations: 0,
      decimals: bridge.manifest.decimals,
      rpcUrl: bridge.manifest.rpcUrl,
      apiKey: bridge.manifest.apiKey,
    },
    deps,
  ).verifier;
}
