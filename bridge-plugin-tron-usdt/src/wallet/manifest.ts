/**
 * Bridge manifest — the integrity-pinned descriptor a wallet (Sphere) loads to
 * surface one bridged asset, plus the loader that turns it into a ready plugin.
 *
 * Decision #2 (06 §A2): Sphere holds *zero* chain-agnostic bridge code. It imports
 * this façade + a manifest and renders UI; everything else (derivations, Tron
 * specifics, the verifier) lives here in the plugin. The manifest names the
 * deployed vault/asset/config so the loader can (a) build the bridge plugin and
 * its `IMintJustificationVerifier`, and (b) cross-check the declared `configHash`
 * against the one recomputed from the manifest fields — a misconfigured manifest
 * fails loudly at load, never silently accepts a wrong trust anchor.
 */
import {
  type BridgeConfig,
  configHash as deriveConfigHash,
} from '../bridge-back/derivations.js';
import { fromHex, toHex } from '../hex.js';
import { toEvmAddressHex } from '../tron-address.js';
import { TronUsdtLockJustification } from '../TronUsdtLockJustification.js';
import {
  createTronUsdtBridgePlugin,
  type CreateTronUsdtBridgePluginDeps,
  type TronUsdtBridgePlugin,
} from '../index.js';

/**
 * One bridged asset, integrity-pinned. Keyed by `tokenTypeHex` (derived from
 * `chainId`+`asset`; the registry key). All byte fields are lowercase hex, no `0x`.
 */
export interface BridgeManifest {
  /** Human label for the bridged asset, e.g. "USDT (bridged · Tron)". */
  readonly label: string;
  /** Short ticker for the primary balance display, e.g. "USDT" — the bridged
   * asset's coinId is bridge-derived and never in the token registry, so the
   * wallet UI has no other source for this. */
  readonly symbol: string;
  /** Tron network id (e.g. Nile = 3448148188). */
  readonly chainId: number;
  /** Deployed `UnicityBridgeVault` (lock) address (base58 `T…`, `41…`, or 20-byte hex). */
  readonly vault: string;
  /** TRC20 asset (USDT) address (any of the same forms). */
  readonly asset: string;
  /** Source-finality threshold an independent receiver enforces (K). */
  readonly confirmations: number;
  /** Token decimals (USDT = 6). */
  readonly decimals: number;
  /** Tron HTTP RPC base URL. */
  readonly rpcUrl: string;
  /** Optional TronGrid API key. */
  readonly apiKey?: string;
  /** Part-B return-service base URL (bridge-back handoff). */
  readonly returnServiceUrl: string;
  /** `BridgeBackReason` CBOR tag the vault/prover bind (frozen config). */
  readonly reasonTag: number;
  /** 32-byte lock domain separator the deployed vault was constructed with (hex). */
  readonly lockDomain: string;
  /** 32-byte nullifier domain separator (hex). */
  readonly nullifierDomain: string;
  /** Groth16 verification key fingerprint the vault enforces (`0x…`); display/ops. */
  readonly vkey: string;
  /** 32-byte `configHash` the deployed vault self-derives (hex). Cross-checked at load. */
  readonly configHash: string;
  /** Optional explicit `tokenTypeHex`; derived + cross-checked when present. */
  readonly tokenTypeHex?: string;
  /** Optional explicit `coinIdHex`; derived + cross-checked when present. */
  readonly coinIdHex?: string;
}

/** A manifest entry resolved into everything the wallet needs to use it. */
export interface LoadedBridge {
  readonly manifest: BridgeManifest;
  /** The ready plugin (TokenType/coinId derivations + the registered verifier). */
  readonly plugin: TronUsdtBridgePlugin;
  /** The `BridgeConfig` the bridge-back reason/nullifier bind to (00 §2). */
  readonly bridgeConfig: BridgeConfig;
  /** 32-byte `configHash` recomputed from the manifest (== `manifest.configHash`). */
  readonly configHash: Uint8Array;
}

/** Build the canonical {BridgeConfig} (00 §2) from a manifest + a resolved plugin. */
export function bridgeConfigFromManifest(m: BridgeManifest, plugin: TronUsdtBridgePlugin): BridgeConfig {
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
  deps: CreateTronUsdtBridgePluginDeps = {},
): LoadedBridge[] {
  const list = Array.isArray(manifest) ? manifest : [manifest as BridgeManifest];
  return list.map((m) => loadOne(m, deps));
}

function loadOne(m: BridgeManifest, deps: CreateTronUsdtBridgePluginDeps): LoadedBridge {
  const plugin = createTronUsdtBridgePlugin(
    {
      chainId: m.chainId,
      lockContract: m.vault,
      assetContract: m.asset,
      confirmations: m.confirmations,
      decimals: m.decimals,
      rpcUrl: m.rpcUrl,
      apiKey: m.apiKey,
    },
    deps,
  );

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

/** The justification CBOR tag every bridged USDT mint reason carries (dispatch key). */
export const TRON_USDT_JUSTIFICATION_TAG = TronUsdtLockJustification.CBOR_TAG;

/** Normalize any Tron address form to 20-byte EVM-style hex (re-exported for UI). */
export { toEvmAddressHex };
