import type { ChainFamily } from '@unicitylabs/bridge-core';

/**
 * Configuration for one bridged asset. The family, chain, contract and asset
 * fields are trust anchors: the verifier rejects any lock proof that does not
 * match them.
 */
export interface BridgeAssetConfig {
  /** Source chain family (CAIP-2 namespace). */
  readonly family: ChainFamily;
  /** Source chain id in the family's numbering. */
  readonly chainId: number;
  /** Canonical vault (lock) contract, in any of the family's address forms. */
  readonly lockContract: string;
  /** Bridged token contract, same forms. */
  readonly assetContract: string;
  /** Required confirmations for source finality; the family's default when omitted. */
  readonly confirmations?: number;
  /** Token decimals (USDT and USDC: 6). */
  readonly decimals?: number;
  /** Node URL for the family's RPC client (used when no client is injected). */
  readonly rpcUrl?: string;
  /** Node API key, where the family's node wants one (TronGrid). */
  readonly apiKey?: string;
}

export const DEFAULT_DECIMALS = 6;
