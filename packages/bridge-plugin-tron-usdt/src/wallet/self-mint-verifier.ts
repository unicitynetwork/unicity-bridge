/**
 * Self-mint verifier (06 §A1.1, frozen decision: "the minter trusts its own
 * lock" — mints at `confirmations: 0` right after locking, since the wallet
 * itself just broadcast and witnessed the lock; there is no reorg risk it
 * hasn't already accepted by locking in the first place).
 *
 * The manifest's K-confirmation threshold still guards every OTHER
 * verification — an independent receiver accepting this token later, or this
 * same wallet reloading it — via the shared `bridgeJustificationVerifiers`
 * service registered once at `Sphere.init()`. This builds a throwaway,
 * one-off verifier service for a single immediate post-lock mint call; it
 * never replaces or mutates the shared service.
 */
import { MintJustificationVerifierService } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/MintJustificationVerifierService.js';

import { createTronUsdtBridgePlugin, type CreateTronUsdtBridgePluginDeps } from '../index.js';
import type { LoadedBridge } from './manifest.js';

/** A one-off {MintJustificationVerifierService} that trusts this bridge's own lock at 0 confirmations. */
export function buildSelfMintVerifierService(
  bridge: LoadedBridge,
  deps: CreateTronUsdtBridgePluginDeps = {},
): MintJustificationVerifierService {
  const selfMintPlugin = createTronUsdtBridgePlugin(
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
  );
  const service = new MintJustificationVerifierService();
  service.register(selfMintPlugin.verifier);
  return service;
}
