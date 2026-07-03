/**
 * Presentation helpers the wallet UI needs but should not hardcode (08 §8 —
 * de-Tron the modal): the block-explorer tx URL and destination-address
 * validation, keyed by chainId so the UI stays chain-agnostic (it asks the
 * bridge, it doesn't know Nile URLs or the Tron address shape).
 */
import { TRON_NILE_CHAIN_ID } from '../config.js';

/** Tronscan transaction URL for a Tron chainId (Nile testnet vs mainnet). */
export function explorerTxUrl(chainId: number, txid: string): string {
  const base =
    chainId === TRON_NILE_CHAIN_ID
      ? 'https://nile.tronscan.org/#/transaction/'
      : 'https://tronscan.org/#/transaction/';
  return base + txid;
}

/** Structural validity of a Tron base58 (`T…`) address (a bridge-out destination). */
export function isValidTronAddress(addr: string): boolean {
  return /^T[1-9A-HJ-NP-Za-km-z]{33}$/.test(addr.trim());
}

/**
 * The chain-specific UI presentation a bridge needs (08 §8): a source-chain
 * explorer link and destination-address validation. The wallet UI holds one of
 * these per bridge (from the registry) and never keys on a numeric `chainId` or
 * hardcodes a chain's URL / address shape — the bridge supplies it.
 */
export interface BridgePresentation {
  /** Block-explorer URL for a source-chain transaction. */
  explorerTxUrl(txid: string): string;
  /** Structural validity of a destination address on this bridge's source chain. */
  validateAddress(addr: string): boolean;
}

/** The {BridgePresentation} for a Tron chainId. */
export function tronPresentation(chainId: number): BridgePresentation {
  return {
    explorerTxUrl: (txid) => explorerTxUrl(chainId, txid),
    validateAddress: (addr) => isValidTronAddress(addr),
  };
}
