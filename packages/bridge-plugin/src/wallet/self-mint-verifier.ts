/**
 * The depositor's own mint-reason verifier: identical to the strict one the
 * wallet registers, but at `confirmations: 0`. The depositor witnessed its own
 * lock, so it accepts the mint as soon as the lock is in a block; every other
 * wallet that receives the token re-verifies under the manifest's `K`.
 */
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';

import { createBridgePlugin, type CreateBridgePluginDeps } from '../index.js';
import { assetConfigFromManifest, type LoadedBridge } from './manifest.js';

export function selfMintVerifier(bridge: LoadedBridge, deps: CreateBridgePluginDeps = {}): IMintJustificationVerifier {
  return createBridgePlugin(assetConfigFromManifest(bridge.manifest, 0), deps).verifier;
}
