import type { BridgePresentation } from '@unicitylabs/bridge-core';

import { ETHEREUM_MAINNET_CHAIN_ID, SEPOLIA_CHAIN_ID } from './config.js';

const EXPLORERS: Record<number, string> = {
  [ETHEREUM_MAINNET_CHAIN_ID]: 'https://etherscan.io',
  [SEPOLIA_CHAIN_ID]: 'https://sepolia.etherscan.io',
};

function with0x(hex: string): string {
  return hex.startsWith('0x') || hex.startsWith('0X') ? hex : `0x${hex}`;
}

export function evmExplorerTxUrl(chainId: number, txid: string): string {
  const base = EXPLORERS[chainId] ?? 'https://blockscan.com';
  return `${base}/tx/${with0x(txid)}`;
}

export function isValidEvmAddress(addr: string): boolean {
  return /^0x[0-9a-fA-F]{40}$/.test(addr.trim());
}

export function evmPresentation(chainId: number): BridgePresentation {
  return {
    explorerTxUrl: (txid) => evmExplorerTxUrl(chainId, txid),
    validateAddress: (addr) => isValidEvmAddress(addr),
  };
}
