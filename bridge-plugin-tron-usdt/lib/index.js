import { DEFAULT_CONFIRMATIONS, DEFAULT_DECIMALS, } from './config.js';
import { toHex } from './hex.js';
import { deriveCoinId, deriveTokenType } from './identifiers.js';
import { toEvmAddressHex } from './tron-address.js';
import { TronHttpRpcClient } from './TronRpcClient.js';
import { TronUsdtMintJustificationVerifier, } from './TronUsdtMintJustificationVerifier.js';
import { TRON_USDT_LOCK_JUSTIFICATION_TAG } from './TronUsdtLockJustification.js';
export * from './config.js';
export * from './hex.js';
export * from './identifiers.js';
export * from './lock-event.js';
export * from './tron-address.js';
export * from './TronRpcClient.js';
export * from './TronUsdtLockJustification.js';
export * from './TronUsdtMintJustificationVerifier.js';
export * from './value.js';
/**
 * Build a Tron-USDT bridge plugin from canonical config. Derives the asset's
 * TokenType/coinId, normalizes trust-anchor addresses, and wires the verifier.
 */
export function createTronUsdtBridgePlugin(config, deps = {}) {
    const tokenType = deriveTokenType(config.chainId, config.assetContract);
    const coinId = deriveCoinId(config.chainId, config.assetContract);
    const resolvedConfig = {
        chainId: config.chainId,
        lockContractHex: toEvmAddressHex(config.lockContract),
        assetContractHex: toEvmAddressHex(config.assetContract),
        confirmations: config.confirmations ?? DEFAULT_CONFIRMATIONS,
        tokenType,
        coinId,
    };
    const rpc = deps.rpc ??
        new TronHttpRpcClient({
            baseUrl: config.rpcUrl ?? 'https://api.trongrid.io',
            apiKey: config.apiKey,
        });
    const verifier = new TronUsdtMintJustificationVerifier(resolvedConfig, {
        rpc,
        extractAmount: deps.extractAmount,
    });
    return {
        cborTag: TRON_USDT_LOCK_JUSTIFICATION_TAG,
        tokenTypeHex: toHex(tokenType),
        coinIdHex: toHex(coinId),
        decimals: config.decimals ?? DEFAULT_DECIMALS,
        resolvedConfig,
        verifier,
    };
}
