/**
 * What the wallet UI shows for a Tron chain without knowing Tron: the
 * Tronscan link for a transaction and the shape of a valid destination.
 */
import type { BridgePresentation } from '@unicitylabs/bridge-core';

import { TRON_NILE_CHAIN_ID } from './config.js';

function tronscan(chainId: number): string {
  return chainId === TRON_NILE_CHAIN_ID ? 'https://nile.tronscan.org/#' : 'https://tronscan.org/#';
}

/** Tronscan transaction URL for a Tron chainId (Nile testnet vs mainnet). */
export function explorerTxUrl(chainId: number, txid: string): string {
  return `${tronscan(chainId)}/transaction/${txid}`;
}

/** Tronscan address URL for a Tron chainId (Nile testnet vs mainnet); takes the base58 `T…` form Tronscan expects. */
export function explorerAddressUrl(chainId: number, address: string): string {
  return `${tronscan(chainId)}/address/${address}`;
}

/** Structural validity of a Tron base58 (`T…`) address (a bridge-out destination). */
export function isValidTronAddress(addr: string): boolean {
  return /^T[1-9A-HJ-NP-Za-km-z]{33}$/.test(addr.trim());
}

export function tronPresentation(chainId: number): BridgePresentation {
  return {
    explorerTxUrl: (txid) => explorerTxUrl(chainId, txid),
    explorerAddressUrl: (address) => explorerAddressUrl(chainId, address),
    validateAddress: (addr) => isValidTronAddress(addr),
  };
}
