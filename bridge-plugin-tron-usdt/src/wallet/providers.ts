/**
 * Tron wallet providers (08 Phase 3 — wallet picker, adapter/WalletConnect seam).
 *
 * The bridge-in flow needs a {TronSigner}; *which* wallet supplies it (injected
 * TronLink, WalletConnect, a managed key) is a user choice. This is the small
 * registry the picker renders: each {TronWalletProvider} knows its id/name, whether
 * it's usable here, and how to `create` a (not-yet-connected) `TronSigner` for a
 * given source chain. `runBridgeIn` then drives that signer uniformly — `connect()`
 * prompts (TronLink authorize, or the WalletConnect QR), the rest is identical.
 *
 * TronLink (priority 1) needs no extra dependency and is implemented here.
 * WalletConnect is wired by **injecting a signer factory** ({TronWalletConfig})
 * from the app once `@tronweb3/walletconnect-tron` is installed and a `projectId`
 * is configured — so this module (and everything that typechecks against it) has
 * **no** dependency on the WalletConnect package. Until the factory is supplied,
 * only TronLink is offered.
 */
import {
  TronLinkSigner,
  type TronLinkWindow,
  type TronSigner,
} from './tron-signer.js';

/** Stable ids for the wallets the picker can offer. */
export type TronWalletId = 'tronlink' | 'walletconnect';

/** One selectable wallet: identity + availability + a `TronSigner` factory. */
export interface TronWalletProvider {
  readonly id: TronWalletId;
  /** Display name for the picker, e.g. "TronLink". */
  readonly name: string;
  /** True when this wallet can be used in the current environment (extension present, etc.). */
  isAvailable(): boolean;
  /** Construct a **not-yet-connected** signer for `chainId`; `runBridgeIn` calls `connect()`. */
  create(chainId: number): TronSigner;
}

/**
 * Factory the app injects once `@tronweb3/walletconnect-tron` v4 is installed and a
 * WalletConnect `projectId` is configured. Returns a `TronSigner` that wraps the
 * adapter (its `connect()` opens the WC modal). Keeping this a callback is what lets
 * the plugin stay free of any WalletConnect dependency.
 */
export type WalletConnectSignerFactory = (chainId: number) => TronSigner;

/** App-supplied wallet configuration (which optional wallets to offer + how to build them). */
export interface TronWalletConfig {
  /** Present only when WalletConnect is configured (projectId set + factory wired). */
  readonly walletConnect?: {
    readonly signerFactory: WalletConnectSignerFactory;
  };
}

/** Injected-TronLink provider (browser extension) — priority 1, no extra deps. */
export function tronLinkProvider(
  win: TronLinkWindow = globalThis as unknown as TronLinkWindow,
): TronWalletProvider {
  return {
    id: 'tronlink',
    name: 'TronLink',
    isAvailable: () => Boolean(win.tronWeb),
    create: (chainId) => new TronLinkSigner(win, chainId),
  };
}

/** WalletConnect provider built from the app-injected signer factory. */
export function walletConnectProvider(factory: WalletConnectSignerFactory): TronWalletProvider {
  return {
    id: 'walletconnect',
    name: 'WalletConnect',
    isAvailable: () => true, // usable on any device once configured (that's the point of WC)
    create: (chainId) => factory(chainId),
  };
}

/**
 * The wallets to offer in the picker: TronLink always, plus WalletConnect when the
 * app has configured it. Availability (extension present?) is left to the UI via
 * `isAvailable()` — the list is not pre-filtered, so a missing TronLink can still be
 * shown with an "install it" hint rather than silently vanishing.
 */
export function availableTronWallets(config: TronWalletConfig = {}): TronWalletProvider[] {
  const providers: TronWalletProvider[] = [tronLinkProvider()];
  if (config.walletConnect) {
    providers.push(walletConnectProvider(config.walletConnect.signerFactory));
  }
  return providers;
}
