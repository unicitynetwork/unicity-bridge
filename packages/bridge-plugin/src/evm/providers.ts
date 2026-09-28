import type { SourceWalletProvider } from '../wallet/signer.js';
import { InjectedEvmSigner, type EvmWindow } from './signer.js';

export function injectedEvmProvider(win: EvmWindow = globalThis as unknown as EvmWindow): SourceWalletProvider {
  return {
    id: 'injected-evm',
    name: 'MetaMask',
    isAvailable: () => Boolean(win.ethereum),
    create: (chainId) => new InjectedEvmSigner(win, chainId),
  };
}
