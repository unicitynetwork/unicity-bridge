/** Tron network ids (TronWeb genesis-derived chain ids). */
export const TRON_MAINNET_CHAIN_ID = 728126428; // 0x2b6653dc
export const TRON_NILE_CHAIN_ID = 3448148188; // 0xcd8690dc

/** Canonical USDT (TRC20) contract on Tron mainnet. */
export const TRON_MAINNET_USDT = 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t';

/** Canonical USDT (TRC20) contract on the Tron Nile testnet. */
export const TRON_NILE_USDT = 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf';

/**
 * Configuration for one Tron-bridged asset. The contract/asset/chain fields are
 * trust anchors: the verifier rejects any lock proof that does not match them.
 */
export interface TronUsdtBridgeConfig {
  /** Tron network id (see constants above). */
  readonly chainId: number;
  /** Canonical UnicityLock contract (base58 `T…`, `41…` hex, or 20-byte hex). */
  readonly lockContract: string;
  /** USDT TRC20 token address (any of the same forms). */
  readonly assetContract: string;
  /** Required confirmations for source finality (default 20 ≈ Tron SR irreversibility). */
  readonly confirmations?: number;
  /** Token decimals (USDT = 6). */
  readonly decimals?: number;
  /** Tron HTTP API base URL (used when no TronRpc instance is injected). */
  readonly rpcUrl?: string;
  /** TronGrid API key (optional). */
  readonly apiKey?: string;
}

export const DEFAULT_CONFIRMATIONS = 20;
export const DEFAULT_DECIMALS = 6;

/**
 * Mainnet config skeleton. `lockContract` must be filled with the deployed
 * UnicityLock address before use.
 */
export function tronMainnetUsdtConfig(lockContract: string, rpcUrl = 'https://api.trongrid.io'): TronUsdtBridgeConfig {
  return {
    chainId: TRON_MAINNET_CHAIN_ID,
    lockContract,
    assetContract: TRON_MAINNET_USDT,
    confirmations: DEFAULT_CONFIRMATIONS,
    decimals: DEFAULT_DECIMALS,
    rpcUrl,
  };
}

/**
 * Nile testnet config skeleton. `lockContract` must be filled with the deployed
 * vault address before use (see docs/bridge/dev-plan/04-deployment.md).
 */
export function tronNileUsdtConfig(lockContract: string, rpcUrl = 'https://nile.trongrid.io'): TronUsdtBridgeConfig {
  return {
    chainId: TRON_NILE_CHAIN_ID,
    lockContract,
    assetContract: TRON_NILE_USDT,
    confirmations: DEFAULT_CONFIRMATIONS,
    decimals: DEFAULT_DECIMALS,
    rpcUrl,
  };
}
