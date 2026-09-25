import { DEFAULT_DECIMALS, type BridgeAssetConfig } from '../config.js';

/** Tron network ids (TronWeb genesis-derived chain ids). */
export const TRON_MAINNET_CHAIN_ID = 728126428; // 0x2b6653dc
export const TRON_NILE_CHAIN_ID = 3448148188; // 0xcd8690dc

/** Canonical USDT (TRC20) contract on Tron mainnet. */
export const TRON_MAINNET_USDT = 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t';

/** Canonical USDT (TRC20) contract on the Tron Nile testnet. */
export const TRON_NILE_USDT = 'TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf';

export const TRON_DEFAULT_CONFIRMATIONS = 20;

export function tronMainnetUsdtConfig(lockContract: string, rpcUrl = 'https://api.trongrid.io'): BridgeAssetConfig {
  return {
    family: 'tron',
    chainId: TRON_MAINNET_CHAIN_ID,
    lockContract,
    assetContract: TRON_MAINNET_USDT,
    confirmations: TRON_DEFAULT_CONFIRMATIONS,
    decimals: DEFAULT_DECIMALS,
    rpcUrl,
  };
}

export function tronNileUsdtConfig(lockContract: string, rpcUrl = 'https://nile.trongrid.io'): BridgeAssetConfig {
  return {
    family: 'tron',
    chainId: TRON_NILE_CHAIN_ID,
    lockContract,
    assetContract: TRON_NILE_USDT,
    confirmations: TRON_DEFAULT_CONFIRMATIONS,
    decimals: DEFAULT_DECIMALS,
    rpcUrl,
  };
}
