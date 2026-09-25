import { toEvmAddressHex } from '../address.js';
import type { ChainFamilyAdapter } from '../family.js';
import { EVM_DEFAULT_CONFIRMATIONS } from './config.js';
import { EvmJsonRpcClient } from './EvmRpcClient.js';
import { evmPresentation } from './presentation.js';

export function evmChainRef(chainId: number): string {
  return `eip155:${chainId}`;
}

export const evmFamily: ChainFamilyAdapter = {
  family: 'eip155',
  chainName: 'Ethereum',
  defaultConfirmations: EVM_DEFAULT_CONFIRMATIONS,
  blockSeconds: 12,
  chainRef: evmChainRef,
  normalizeAddress: toEvmAddressHex,
  createRpc: ({ rpcUrl }) => {
    if (!rpcUrl) throw new Error('An Ethereum-family bridge needs an rpcUrl.');
    return new EvmJsonRpcClient({ rpcUrl });
  },
  presentation: evmPresentation,
};
