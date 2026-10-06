import type { SourceWalletProvider } from '../wallet/signer.js';
import { InjectedEvmSigner, type Eip1193Provider } from './signer.js';

export interface EvmWindow {
  ethereum?: Eip1193Provider;
}

/** What a wallet says about itself in an EIP-6963 announcement. */
export interface Eip6963ProviderInfo {
  readonly uuid: string;
  readonly name: string;
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
 * page to do so. `window.ethereum` is listed as the browser wallet when no announced wallet
 * owns it, and alone when nothing announced itself, so a page without a wallet still has an
 * option to show as unavailable.
 */
export function evmWallets(win: EvmDiscoveryWindow = globalThis as unknown as EvmDiscoveryWindow): EvmWallets {
  const announced = new Map<string, Eip6963ProviderDetail>();
  win.addEventListener('eip6963:announceProvider', (e) => {
    const { detail } = e as CustomEvent<Eip6963ProviderDetail>;
    announced.set(detail.info.uuid, detail);
  });
  win.dispatchEvent(new Event('eip6963:requestProvider'));
  return {
    list: () => {
      const details = [...announced.values()];
      const wallets = details.map(announcedEvmProvider);
      return offersBrowserWallet(details, win.ethereum) ? [...wallets, injectedEvmProvider(win)] : wallets;
    },
  };
}

function announcedEvmProvider({ info, provider }: Eip6963ProviderDetail): SourceWalletProvider {
  return {
    id: info.rdns,
    name: info.name,
    icon: info.icon,
    isAvailable: () => true,
    create: (chainId) => new InjectedEvmSigner(provider, chainId),
  };
}

function offersBrowserWallet(announced: readonly Eip6963ProviderDetail[], ethereum: Eip1193Provider | undefined): boolean {
  if (announced.length === 0) return true;
  return ethereum !== undefined && !announced.some((d) => d.provider === ethereum);
}

function requireEthereum(win: EvmWindow): Eip1193Provider {
  if (!win.ethereum) {
    throw new Error('No Ethereum wallet found. Install MetaMask and reload.');
  }
  return win.ethereum;
}
