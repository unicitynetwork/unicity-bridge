import { type TronUsdtBridgeConfig } from './config.js';
import { type TronRpc } from './TronRpcClient.js';
import { type ResolvedTronUsdtConfig, TronUsdtMintJustificationVerifier } from './TronUsdtMintJustificationVerifier.js';
import type { BridgedAmountExtractor } from './value.js';
export * from './config.js';
export * from './hex.js';
export * from './identifiers.js';
export * from './lock-event.js';
export * from './tron-address.js';
export * from './TronRpcClient.js';
export * from './TronUsdtLockJustification.js';
export * from './TronUsdtMintJustificationVerifier.js';
export * from './value.js';
export interface CreateTronUsdtBridgePluginDeps {
    /** Inject a TronRpc (e.g. a mock). If omitted, an HTTP client is built from config.rpcUrl. */
    readonly rpc?: TronRpc;
    /** Override the token-value extractor (sphere-sdk passes a SpherePaymentData-based one). */
    readonly extractAmount?: BridgedAmountExtractor;
}
/** A ready-to-register bridge plugin for one Tron-bridged asset. */
export interface TronUsdtBridgePlugin {
    readonly cborTag: bigint;
    /** 32-byte Unicity TokenType (hex) for this asset. */
    readonly tokenTypeHex: string;
    /** 32-byte Sphere coinId (hex) for this asset. */
    readonly coinIdHex: string;
    readonly decimals: number;
    readonly resolvedConfig: ResolvedTronUsdtConfig;
    /** Register this into a MintJustificationVerifierService. */
    readonly verifier: TronUsdtMintJustificationVerifier;
}
/**
 * Build a Tron-USDT bridge plugin from canonical config. Derives the asset's
 * TokenType/coinId, normalizes trust-anchor addresses, and wires the verifier.
 */
export declare function createTronUsdtBridgePlugin(config: TronUsdtBridgeConfig, deps?: CreateTronUsdtBridgePluginDeps): TronUsdtBridgePlugin;
