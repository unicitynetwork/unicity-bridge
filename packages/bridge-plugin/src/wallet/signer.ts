import type { ContractCall } from '../contract-call.js';

export interface SourceSigner {
  connect(): Promise<string>;
  getAddress(): Promise<string>;
  getNetwork(): Promise<number>;
  sendCall(call: ContractCall): Promise<string>;
  onChange?(cb: (e: WalletChange) => void): () => void;
}

export interface WalletChange {
  readonly kind: 'accountsChanged' | 'chainChanged' | 'disconnect';
}

export interface SourceWalletProvider {
  readonly id: string;
  readonly name: string;
  /** True when this wallet can be used in the current environment (extension present, etc.). */
  isAvailable(): boolean;
  create(chainId: number): SourceSigner;
}
