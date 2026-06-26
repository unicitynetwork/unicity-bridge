export declare const DOMAIN_CONFIG = "unicity-bridge-return-config:v1";
export declare const DOMAIN_LOCK = "unicity-bridge-lock:v1";
export declare const DOMAIN_RETURN = "unicity-bridge-return:v1";
export declare const DOMAIN_BURN_TRANSITION = "unicity-burn-transition:v1";
export declare const DOMAIN_NULLIFIER = "unicity-bridge-return-nullifier:v1";
/** Deployment configuration (00 §2). Byte fields are raw (no `0x`). */
export interface BridgeConfig {
    readonly sourceChainId: bigint;
    readonly vault: Uint8Array;
    readonly asset: Uint8Array;
    readonly tokenType: Uint8Array;
    readonly coinId: Uint8Array;
    readonly reasonTag: bigint;
    readonly lockDomain: Uint8Array;
    readonly nullifierDomain: Uint8Array;
}
/** 32-byte big-endian representation of an unsigned integer. */
export declare function u256be(n: bigint): Uint8Array;
/** `configHash = K(abi.encode("...config:v1", fields...))` (00 §2). */
export declare function configHash(c: BridgeConfig): Uint8Array;
/** `domainTag = K("unicity-bridge-return:v1")` over raw bytes (00 §7). */
export declare function domainTag(): Uint8Array;
/** `lockDigest = K(abi.encode("...lock:v1", fields...))` (00 §3). */
export declare function lockDigest(params: {
    sourceChainId: bigint;
    vault: Uint8Array;
    nonce: bigint;
    asset: Uint8Array;
    tokenType: Uint8Array;
    coinId: Uint8Array;
    amount: bigint;
    unicityTokenId: Uint8Array;
    recipientCommitment: Uint8Array;
}): Uint8Array;
/** Per-return parameters of a `BridgeBackReason` (00 §4); config-bound fields
 *  (sourceChainId/vault/asset/tokenType/coinId) come from {BridgeConfig}. */
export interface BridgeBackReason {
    readonly version: bigint;
    readonly recipient: Uint8Array;
    readonly amount: bigint;
    readonly feeRecipient: Uint8Array;
    readonly feeAmount: bigint;
    readonly deadline: bigint;
}
/**
 * Canonical CBOR of the 11-field `BridgeBackReason` array under `reasonTag`
 * (00 §4). This is the `reasonBytes` that ride in the terminal burn's auxiliary
 * data; the burn predicate binds `H(reasonBytes)` (see {reasonHash}).
 */
export declare function encodeBridgeBackReason(c: BridgeConfig, r: BridgeBackReason): Uint8Array;
/** `reasonHash = H(reasonBytes)` — the value `BurnPredicate(H(reasonBytes))`
 *  binds (00 §4, PROVISIONAL preimage pending M0 SDK confirmation). */
export declare function reasonHash(reasonBytes: Uint8Array): Uint8Array;
/** `burnTransitionId = H("unicity-burn-transition:v1", stateId, txHash)` (00 §5). */
export declare function burnTransitionId(stateId: Uint8Array, txHash: Uint8Array): Uint8Array;
/** `nullifier = H("...nullifier:v1", configHash, burnTransitionId)` (00 §5). */
export declare function nullifier(configHashBytes: Uint8Array, burnTransitionIdBytes: Uint8Array): Uint8Array;
/** One settlement leaf committed by `returnRoot`, in submission order (00 §7). */
export interface ReturnLeaf {
    readonly nullifier: Uint8Array;
    readonly recipient: Uint8Array;
    readonly amount: bigint;
    readonly feeRecipient: Uint8Array;
    readonly feeAmount: bigint;
    readonly deadline: bigint;
}
/** `returnRoot = K( concat of fixed-width abi.encode(leaf) )` (00 §7). */
export declare function returnRoot(leaves: ReturnLeaf[]): Uint8Array;
/** One source-lock reference committed by `lockRefRoot` (00 §7). */
export interface SourceLockRef {
    readonly nonce: bigint;
    readonly digest: Uint8Array;
}
/** `lockRefRoot = K( concat of fixed-width abi.encode(ref) )`, sorted by nonce
 *  with duplicates rejected (00 §7). */
export declare function lockRefRoot(refs: SourceLockRef[]): Uint8Array;
/** The public statement the circuit commits and the vault decodes (00 §7). */
export interface PublicValues {
    readonly domainTag: Uint8Array;
    readonly configHash: Uint8Array;
    readonly trustBaseHash: Uint8Array;
    readonly spentRootOld: Uint8Array;
    readonly spentRootNew: Uint8Array;
    readonly returnRoot: Uint8Array;
    readonly lockRefRoot: Uint8Array;
    readonly batchSize: number;
    readonly totalAmount: bigint;
}
/** `abi.encode(PublicValues)` — the all-static layout the vault abi.decodes (00 §7). */
export declare function publicValuesAbi(p: PublicValues): Uint8Array;
