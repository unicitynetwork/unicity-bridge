/**
 * Deterministic 32-byte Unicity TokenType for a Tron-bridged asset.
 * `TRON_USDT_TYPE = SHA256("unicity-bridge:tron:<chainId>:<assetEvmHex>")`.
 */
export declare function deriveTokenType(chainId: number, assetContract: string): Uint8Array;
/** Deterministic 32-byte Sphere coinId for a Tron-bridged asset. */
export declare function deriveCoinId(chainId: number, assetContract: string): Uint8Array;
/**
 * Commitment the Tron lock must carry so the bridged token can only be owned by
 * the intended recipient: `SHA256(recipient.toCBOR())`.
 */
export declare function recipientCommitment(recipientCbor: Uint8Array): Uint8Array;
