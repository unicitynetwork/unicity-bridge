/**
 * Bridge registry (08 Phase 4 — "manifest registry keyed by family+chain+asset").
 *
 * A wallet may load several bridged assets across chains; this indexes the
 * resolved {LoadedBridge}s so a flow can address one deterministically. The
 * canonical key is **family + chain + asset** — expressed as `chainRef` (which
 * already encodes `family:chain`, e.g. `tron:0xcd8690dc`) plus the resolved asset
 * hex. That key is **stable across vault redeploys**: a redeploy changes the vault
 * address (and `configHash`), but the same asset on the same chain keeps the same
 * key — so pending balances/records keep resolving to their bridge (00 "redeploy /
 * migration discipline"). Registering two manifests for the same key is a config
 * error and throws.
 *
 * Chain-neutral: it reads only `manifest.chainRef` + the plugin's resolved asset
 * hex / coinId / tokenType — no Tron-specific logic — so a second family drops in
 * without touching this file.
 */
import type { LoadedBridge } from './manifest.js';

/**
 * The family+chain+asset key for a resolved bridge — stable across vault
 * redeploys. `chainRef` carries `family:chain`; the resolved asset hex completes it.
 */
export function bridgeAssetKey(bridge: LoadedBridge): string {
  return `${bridge.manifest.chainRef}:${bridge.plugin.resolvedConfig.assetContractHex}`;
}

/** An index over the app's resolved bridges, keyed for deterministic lookup. */
export interface BridgeRegistry {
  /** Every registered bridge (insertion order). */
  readonly all: readonly LoadedBridge[];
  /** By the family+chain+asset key ({bridgeAssetKey}); undefined if not registered. */
  byKey(key: string): LoadedBridge | undefined;
  /** By the bridged coinId (what the wallet UI holds); case-insensitive. */
  byCoinId(coinIdHex: string): LoadedBridge | undefined;
  /** By the Unicity TokenType; case-insensitive. */
  byTokenType(tokenTypeHex: string): LoadedBridge | undefined;
}

/**
 * Build a {BridgeRegistry} from resolved bridges. Throws on a duplicate
 * family+chain+asset key — two manifests describing the same bridged asset is a
 * misconfiguration, never a silent last-one-wins.
 */
export function buildBridgeRegistry(loaded: readonly LoadedBridge[]): BridgeRegistry {
  const byKey = new Map<string, LoadedBridge>();
  const byCoin = new Map<string, LoadedBridge>();
  const byType = new Map<string, LoadedBridge>();
  for (const bridge of loaded) {
    const key = bridgeAssetKey(bridge);
    const existing = byKey.get(key);
    if (existing) {
      throw new Error(
        `Duplicate bridge for ${key}: "${bridge.manifest.label}" and "${existing.manifest.label}" ` +
          `describe the same family+chain+asset.`,
      );
    }
    byKey.set(key, bridge);
    byCoin.set(bridge.plugin.coinIdHex, bridge);
    byType.set(bridge.plugin.tokenTypeHex, bridge);
  }
  return {
    all: loaded,
    byKey: (key) => byKey.get(key),
    byCoinId: (coinIdHex) => byCoin.get(coinIdHex.toLowerCase()),
    byTokenType: (tokenTypeHex) => byType.get(tokenTypeHex.toLowerCase()),
  };
}
