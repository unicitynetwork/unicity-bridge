/**
 * Reads the bridged-coin amount declared in a token's `data`, or null if the
 * token declares no value for `coinId`. Injected into the verifier so the
 * mint-reason check can confirm the token's declared value equals the locked
 * amount.
 *
 * In sphere-sdk this is backed by `decodeSpherePaymentData`; standalone/CLI use
 * {@link decodeBridgedValue} over the simple envelope below.
 */
export type BridgedAmountExtractor = (data: Uint8Array | null, coinId: Uint8Array) => bigint | null;
/**
 * Minimal self-contained value envelope used by the CLI/tests:
 * `CBOR [ coinId: bstr, amount: uint ]`.
 */
export declare function encodeBridgedValue(coinId: Uint8Array, amount: bigint): Uint8Array;
export declare function decodeBridgedValue(data: Uint8Array | null, coinId: Uint8Array): bigint | null;
