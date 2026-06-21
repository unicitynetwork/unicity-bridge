/** Tron network ids (TronWeb genesis-derived chain ids). */
export declare const TRON_MAINNET_CHAIN_ID = 728126428;
export declare const TRON_NILE_CHAIN_ID = 3448148188;
/** Canonical USDT (TRC20) contract on Tron mainnet. */
export declare const TRON_MAINNET_USDT = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
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
export declare const DEFAULT_CONFIRMATIONS = 20;
export declare const DEFAULT_DECIMALS = 6;
/**
 * Mainnet config skeleton. `lockContract` must be filled with the deployed
 * UnicityLock address before use.
 */
export declare function tronMainnetUsdtConfig(lockContract: string, rpcUrl?: string): TronUsdtBridgeConfig;
