import type { SourceWalletProvider } from '../wallet/signer.js';
import { InjectedEvmSigner, type Eip1193Provider } from './signer.js';

export interface EvmWindow {
  ethereum?: Eip1193Provider;
}

/** What a wallet says about itself in an EIP-6963 announcement. */
export interface Eip6963ProviderInfo {
  readonly uuid: string;
  readonly name: string;
  /** An inline `data:image/…` URI by the standard; anything else is not shown. */
  readonly icon: string;
  readonly rdns: string;
}

export interface Eip6963ProviderDetail {
  readonly info: Eip6963ProviderInfo;
  readonly provider: Eip1193Provider;
}

export interface EvmDiscoveryWindow extends EvmWindow {
  addEventListener(type: string, listener: (e: unknown) => void): void;
  dispatchEvent(e: Event): boolean;
}

/** The Ethereum wallets on the page now; a wallet can announce itself after this was created. */
export interface EvmWallets {
  list(): readonly SourceWalletProvider[];
}

/** The wallet behind `window.ethereum`, for wallets that do not announce themselves (EIP-6963). */
export function injectedEvmProvider(win: EvmWindow = globalThis as unknown as EvmWindow): SourceWalletProvider {
  return {
    id: 'injected-evm',
    name: 'Browser wallet',
    isAvailable: () => Boolean(win.ethereum),
    create: (chainId) => new InjectedEvmSigner(requireEthereum(win), chainId),
  };
}

/**
 * Listens for the wallets announcing themselves (EIP-6963) and asks the ones already on the
 * page to do so. The first announcement under an rdns stays: a later one, from any script on
 * the page, cannot take over a wallet already listed. `window.ethereum` is listed as the
 * browser wallet only when nothing announced itself, so an older wallet is still reachable and
 * a page without a wallet still has an option to show as unavailable. Where nothing can
 * announce itself, as under Node, only the browser wallet is considered.
 */
export function evmWallets(win: EvmDiscoveryWindow = globalThis as unknown as EvmDiscoveryWindow): EvmWallets {
  const announced = new Map<string, Eip6963ProviderDetail>();
  if (typeof win.addEventListener === 'function' && typeof win.dispatchEvent === 'function') {
    win.addEventListener('eip6963:announceProvider', (e) => {
      const detail = announcedDetail((e as CustomEvent<unknown>).detail);
      if (detail && !announced.has(detail.info.rdns)) announced.set(detail.info.rdns, detail);
    });
    win.dispatchEvent(new Event('eip6963:requestProvider'));
  }
  return {
    list: () => {
      const wallets = [...announced.values()].map(announcedEvmProvider);
      return wallets.length === 0 ? [injectedEvmProvider(win)] : wallets;
    },
  };
}

function announcedDetail(detail: unknown): Eip6963ProviderDetail | null {
  const { info, provider } = (detail ?? {}) as { info?: Partial<Eip6963ProviderInfo>; provider?: Partial<Eip1193Provider> };
  const named = [info?.uuid, info?.rdns, info?.name].every((v) => typeof v === 'string' && v.length > 0);
  if (!named || typeof provider?.request !== 'function') return null;
  return { info: { ...(info as Eip6963ProviderInfo), icon: String(info?.icon ?? '') }, provider: provider as Eip1193Provider };
}

function announcedEvmProvider({ info, provider }: Eip6963ProviderDetail): SourceWalletProvider {
  return {
    id: info.rdns,
    name: info.name,
    icon: info.icon.startsWith('data:image/') ? info.icon : undefined,
    isAvailable: () => true,
    create: (chainId) => new InjectedEvmSigner(provider, chainId),
  };
}

function requireEthereum(win: EvmWindow): Eip1193Provider {
  if (!win.ethereum) {
    throw new Error('No Ethereum wallet found. Install a browser wallet and reload.');
  }
  return win.ethereum;
}
