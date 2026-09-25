import { toEvmAddressHex } from '../address.js';
import type { ChainFamilyAdapter } from '../family.js';
import { TRON_DEFAULT_CONFIRMATIONS } from './config.js';
import { tronPresentation } from './presentation.js';
import { TronHttpRpcClient } from './TronRpcClient.js';

/** The CAIP-2-style chain reference for a Tron numeric chainId (`tron:0x<hex>`). */
export function tronChainRef(chainId: number): string {
  return `tron:0x${chainId.toString(16)}`;
}

export const tronFamily: ChainFamilyAdapter = {
  family: 'tron',
  chainName: 'Tron',
  defaultConfirmations: TRON_DEFAULT_CONFIRMATIONS,
  blockSeconds: 3,
  chainRef: tronChainRef,
  normalizeAddress: toEvmAddressHex,
  createRpc: ({ rpcUrl, apiKey }) => new TronHttpRpcClient({ baseUrl: rpcUrl ?? 'https://api.trongrid.io', apiKey }),
  presentation: tronPresentation,
};
