/** Tron network ids (TronWeb genesis-derived chain ids). */
export const TRON_MAINNET_CHAIN_ID = 728126428; // 0x2b6653dc
export const TRON_NILE_CHAIN_ID = 3448148188; // 0xcd8690dc
/** Canonical USDT (TRC20) contract on Tron mainnet. */
export const TRON_MAINNET_USDT = 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t';
export const DEFAULT_CONFIRMATIONS = 20;
export const DEFAULT_DECIMALS = 6;
/**
 * Mainnet config skeleton. `lockContract` must be filled with the deployed
 * UnicityLock address before use.
 */
export function tronMainnetUsdtConfig(lockContract, rpcUrl = 'https://api.trongrid.io') {
    return {
        chainId: TRON_MAINNET_CHAIN_ID,
        lockContract,
        assetContract: TRON_MAINNET_USDT,
        confirmations: DEFAULT_CONFIRMATIONS,
        decimals: DEFAULT_DECIMALS,
        rpcUrl,
    };
}
