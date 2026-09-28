/**
 * Bridge manifest — the integrity-pinned descriptor a wallet (Sphere) loads to
 * surface one bridged asset, plus the loader that turns it into a ready plugin.
 *
 * Decision #2 (06 §A2): Sphere holds *zero* chain-specific bridge code. It
 * imports this façade + a manifest and renders UI; everything else (derivations,
 * chain specifics, the verifier) lives here in the plugin. The manifest names the
 * deployed vault/asset/config so the loader can (a) build the bridge plugin and
 * its `IMintJustificationVerifier`, and (b) cross-check the declared `configHash`
 * against the one recomputed from the manifest fields — a misconfigured manifest
 * fails loudly at load, never silently accepts a wrong trust anchor.
 */
import type { BridgeManifestBase, BridgePresentation } from '@unicitylabs/bridge-core';

import { toEvmAddressHex } from '../address.js';
import { type BridgeConfig, configHash as deriveConfigHash } from '../bridge-back/derivations.js';
import { BRIDGE_LOCK_JUSTIFICATION_TAG } from '../BridgeLockJustification.js';
import type { BridgeAssetConfig } from '../config.js';
import { chainFamily } from '../families.js';
import { fromHex, toHex } from '../hex.js';
import { createBridgePlugin, type BridgePlugin, type CreateBridgePluginDeps } from '../index.js';

export type { BridgeManifestBase } from '@unicitylabs/bridge-core';
export { evmChainRef } from '../evm/family.js';
export { tronChainRef } from '../tron/family.js';

/** A Tron-family bridged-asset manifest. `chainId` is Tron's genesis-derived id, cross-checked against `chainRef`. */
export interface TronBridgeManifest extends BridgeManifestBase {
  readonly family: 'tron';
  readonly chainId: number;
  /** Tron HTTP RPC base URL. */
  readonly rpcUrl: string;
  /** Optional TronGrid API key. */
  readonly apiKey?: string;
}

/** An Ethereum-family (eip155) bridged-asset manifest. `chainId` is the EIP-155 id, cross-checked against `chainRef`. */
export interface EvmBridgeManifest extends BridgeManifestBase {
  readonly family: 'eip155';
  readonly chainId: number;
  /** JSON-RPC endpoint. */
  readonly rpcUrl: string;
}

export type BridgeManifest = TronBridgeManifest | EvmBridgeManifest;

/** A manifest entry resolved into everything the wallet needs to use it. */
export interface LoadedBridge {
  readonly manifest: BridgeManifest;
  /** The ready plugin (TokenType/coinId derivations + the registered verifier). */
  readonly plugin: BridgePlugin;
  /** The `BridgeConfig` the bridge-back reason/nullifier bind to (00 §2). */
  readonly bridgeConfig: BridgeConfig;
  /** 32-byte `configHash` recomputed from the manifest (== `manifest.configHash`). */
  readonly configHash: Uint8Array;
}

export function assetConfigFromManifest(m: BridgeManifest, confirmations: number): BridgeAssetConfig {
  return {
    family: m.family,
    chainId: m.chainId,
    lockContract: m.vault,
    assetContract: m.asset,
    confirmations,
    decimals: m.decimals,
    rpcUrl: m.rpcUrl,
    apiKey: m.family === 'tron' ? m.apiKey : undefined,
  };
}

/** Build the canonical {BridgeConfig} (00 §2) from a manifest + a resolved plugin. */
export function bridgeConfigFromManifest(m: BridgeManifest, plugin: BridgePlugin): BridgeConfig {
  return {
    sourceChainId: BigInt(plugin.resolvedConfig.chainId),
    vault: fromHex(plugin.resolvedConfig.lockContractHex),
    asset: fromHex(plugin.resolvedConfig.assetContractHex),
    tokenType: plugin.resolvedConfig.tokenType,
    coinId: plugin.resolvedConfig.coinId,
    reasonTag: BigInt(m.reasonTag),
    lockDomain: fromHex(m.lockDomain),
    nullifierDomain: fromHex(m.nullifierDomain),
  };
}

/**
 * Resolve a manifest (or array) into ready {LoadedBridge}s. Throws if a declared
 * identifier (`tokenTypeHex`/`coinIdHex`/`configHash`) does not match the value
 * recomputed from the manifest fields — the integrity-pin (06 Risks: manifest
 * trust-anchor misconfig).
 */
export function loadBridges(
  manifest: BridgeManifest | readonly BridgeManifest[],
  deps: CreateBridgePluginDeps = {},
): LoadedBridge[] {
  const list = Array.isArray(manifest) ? manifest : [manifest as BridgeManifest];
  return list.map((m) => loadOne(m, deps));
}

function loadOne(m: BridgeManifest, deps: CreateBridgePluginDeps): LoadedBridge {
  // The generic chainRef must agree with the family's native chainId — an
  // integrity pin like tokenType/coinId/configHash: a misdescribed chain fails
  // loudly at load.
  const expectedRef = chainFamily(m.family).chainRef(m.chainId);
  if (m.chainRef.toLowerCase() !== expectedRef.toLowerCase()) {
    throw new Error(
      `BridgeManifest(${m.label}): chainRef mismatch — declared ${m.chainRef}, derived ${expectedRef} from chainId ${m.chainId}`,
    );
  }

  const plugin = createBridgePlugin(assetConfigFromManifest(m, m.confirmations), deps);

  if (m.tokenTypeHex && m.tokenTypeHex.toLowerCase() !== plugin.tokenTypeHex) {
    throw new Error(
      `BridgeManifest(${m.label}): tokenTypeHex mismatch — declared ${m.tokenTypeHex}, derived ${plugin.tokenTypeHex}`,
    );
  }
  if (m.coinIdHex && m.coinIdHex.toLowerCase() !== plugin.coinIdHex) {
    throw new Error(
      `BridgeManifest(${m.label}): coinIdHex mismatch — declared ${m.coinIdHex}, derived ${plugin.coinIdHex}`,
    );
  }

  const bridgeConfig = bridgeConfigFromManifest(m, plugin);
  const configHash = deriveConfigHash(bridgeConfig);
  if (toHex(configHash) !== m.configHash.toLowerCase()) {
    throw new Error(
      `BridgeManifest(${m.label}): configHash mismatch — declared ${m.configHash}, derived ${toHex(configHash)}. ` +
        `The manifest does not describe the deployed vault.`,
    );
  }

  return { manifest: m, plugin, bridgeConfig, configHash };
}

/**
 * The {BridgePresentation} for a resolved bridge, from its chain family, so the
 * wallet UI asks the bridge for its explorer link / address validation instead
 * of keying on a numeric chainId (08 §8).
 */
export function bridgePresentation(bridge: LoadedBridge): BridgePresentation {
  return chainFamily(bridge.manifest.family).presentation(bridge.manifest.chainId);
}

export function chainName(bridge: LoadedBridge): string {
  return chainFamily(bridge.manifest.family).chainName;
}

/** The justification CBOR tag every bridged mint reason carries (dispatch key). */
export { BRIDGE_LOCK_JUSTIFICATION_TAG };

/** Normalize any address form to 20-byte EVM-style hex (re-exported for UI). */
export { toEvmAddressHex };
