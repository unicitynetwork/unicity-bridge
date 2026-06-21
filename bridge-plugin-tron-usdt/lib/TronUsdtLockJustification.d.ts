/**
 * CBOR tag for the Tron-USDT lock justification. Bridge tags are allocated
 * one-per-asset so the SDK's tag→verifier registry stays 1:1.
 */
export declare const TRON_USDT_LOCK_JUSTIFICATION_TAG = 1330002n;
/** Decoded contents of a Tron-USDT lock justification (the token's mint reason). */
export interface TronUsdtLockJustificationData {
    /** Tron network id (e.g. mainnet 728126428, Nile 3448148188). */
    readonly chainId: number;
    /** 20-byte EVM-form address of the canonical UnicityLock contract. */
    readonly lockContract: Uint8Array;
    /** 20-byte EVM-form address of the USDT TRC20 token. */
    readonly assetContract: Uint8Array;
    /** 32-byte Tron transaction hash of the lock() call. */
    readonly txid: Uint8Array;
    /** Index of the Lock event within that transaction's logs. */
    readonly logIndex: number;
    /** Locked USDT amount (6 decimals). */
    readonly amount: bigint;
    /** Lock nonce assigned by the contract (echoed in the Lock event). */
    readonly nonce: bigint;
}
/** Encodes/decodes the self-contained lock proof carried in a token's mint reason. */
export declare class TronUsdtLockJustification {
    readonly data: TronUsdtLockJustificationData;
    static readonly CBOR_TAG = 1330002n;
    constructor(data: TronUsdtLockJustificationData);
    toCBOR(): Uint8Array;
    static fromCBOR(bytes: Uint8Array): TronUsdtLockJustification;
}
