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
import { tronPresentation, type BridgePresentation } from './explorer.js';
import {
  createTronUsdtBridgePlugin,
  type CreateTronUsdtBridgePluginDeps,
  type TronUsdtBridgePlugin,
} from '../index.js';

/**
 * Chain-neutral fields every bridged-asset manifest carries, whatever the source
 * chain family. Integrity-pinned; keyed by `tokenTypeHex` (derived from the source
 * chain + `asset`). All byte fields are lowercase hex, no `0x`. The chain is
 * identified by {chainRef} — a CAIP-2-style string/hex reference (`tron:0x…`,
 * `eip155:1`) — *not* a JavaScript number; a family's numeric id (if it has one)
 * lives inside that family's variant (08 Phase 4: string/hex chain refs).
 */
export interface BridgeManifestBase {
  /** Human label for the bridged asset, e.g. "USDT (bridged · Tron)". */
  readonly label: string;
  /** Short ticker for the primary balance display, e.g. "USDT" — the bridged
   * asset's coinId is bridge-derived and never in the token registry, so the
   * wallet UI has no other source for this. */
  readonly symbol: string;
  /** CAIP-2-style chain reference (e.g. `tron:0xcd8690dc`, `eip155:1`) — the
   * generic, family-agnostic chain identity. Cross-checked against the variant's
   * native id at load. */
  readonly chainRef: string;
  /** Deployed `UnicityBridgeVault` (lock) address (base58 `T…`, `41…`, or 20-byte hex). */
  readonly vault: string;
  /** TRC20 asset (USDT) address (any of the same forms). */
  readonly asset: string;
  /** Source-finality threshold an independent receiver enforces (K). */
  readonly confirmations: number;
  /** Token decimals (USDT = 6). */
  readonly decimals: number;
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

/** A Tron-family bridged-asset manifest — the Tron-only fields live here. */
export interface TronBridgeManifest extends BridgeManifestBase {
  readonly family: 'tron';
  /** Tron network id (e.g. Nile = 3448148188). Native id — cross-checked vs `chainRef`. */
  readonly chainId: number;
  /** Tron HTTP RPC base URL. */
  readonly rpcUrl: string;
  /** Optional TronGrid API key. */
  readonly apiKey?: string;
}

/**
 * A bridged-asset manifest, discriminated on `family`. A second chain family
 * (e.g. `eip155`) adds a variant here and the union stays additive — Sphere never
 * branches on it (08 Phase 4).
 */
export type BridgeManifest = TronBridgeManifest;

/** The CAIP-2-style chain reference for a Tron numeric chainId (`tron:0x<hex>`). */
export function tronChainRef(chainId: number): string {
  return `tron:0x${chainId.toString(16)}`;
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
  // Discriminate on family so a future variant dispatches to its own loader; today
  // only Tron exists. Narrowing also unlocks the Tron-only fields below.
  if (m.family !== 'tron') {
    throw new Error(`BridgeManifest(${(m as BridgeManifestBase).label}): unsupported chain family ${(m as { family: string }).family}`);
  }
  // The generic chainRef must agree with the Tron-native chainId — integrity pin,
  // like tokenType/coinId/configHash: a misdescribed chain fails loudly at load.
  const expectedRef = tronChainRef(m.chainId);
  if (m.chainRef.toLowerCase() !== expectedRef) {
    throw new Error(
      `BridgeManifest(${m.label}): chainRef mismatch — declared ${m.chainRef}, derived ${expectedRef} from chainId ${m.chainId}`,
    );
  }

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

/**
 * The {BridgePresentation} for a resolved bridge — dispatched on the manifest
 * family so the wallet UI asks the bridge for its explorer link / address
 * validation instead of keying on a numeric chainId (08 §8). A second family adds
 * a case here; the UI is untouched.
 */
export function bridgePresentation(bridge: LoadedBridge): BridgePresentation {
  const m = bridge.manifest;
  switch (m.family) {
    case 'tron':
      return tronPresentation(m.chainId);
    default:
      throw new Error(`No bridge presentation for chain family ${(m as { family: string }).family}`);
  }
}

/** The justification CBOR tag every bridged USDT mint reason carries (dispatch key). */
export const TRON_USDT_JUSTIFICATION_TAG = TronUsdtLockJustification.CBOR_TAG;

/** Normalize any Tron address form to 20-byte EVM-style hex (re-exported for UI). */
export { toEvmAddressHex };
