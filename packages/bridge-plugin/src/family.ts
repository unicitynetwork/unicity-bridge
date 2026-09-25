import type { BridgePresentation, ChainFamily } from '@unicitylabs/bridge-core';

import type { ConstantCaller, SourceChainRpc } from './source-chain.js';

export interface RpcOptions {
  readonly rpcUrl?: string;
  readonly apiKey?: string;
}

export interface ChainFamilyAdapter {
  readonly family: ChainFamily;
  readonly chainName: string;
  readonly defaultConfirmations: number;
  readonly blockSeconds: number;
  chainRef(chainId: number): string;
  normalizeAddress(address: string): string;
  createRpc(options: RpcOptions): SourceChainRpc & ConstantCaller;
  presentation(chainId: number): BridgePresentation;
}
